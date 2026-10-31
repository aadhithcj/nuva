//! System integrations: volume, keyboard (Caps Lock / input language), battery, network,
//! Bluetooth headsets, monitors, idle time and full-screen detection.

use crate::events::{self, Event, WM_APP_CAPS, WM_APP_LANGCHECK, WM_APP_VOLUME};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{implement, Result, HSTRING, PCWSTR};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothMajorClass};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::TypedEventHandler;
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Globalization::{GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SISO639LANGNAME, LOCALE_SLOCALIZEDDISPLAYNAME};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, GetLastInputInfo, LASTINPUTINFO, VK_CAPITAL};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------------------------------------------------------------- volume

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCb;

impl IAudioEndpointVolumeCallback_Impl for VolumeCb_Impl {
    fn OnNotify(&self, p: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { p.as_ref() } {
            events::post(WM_APP_VOLUME, (d.fMasterVolume * 1000.0).round() as usize, d.bMuted.0 as isize);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCb;

impl IMMNotificationClient_Impl for DeviceCb_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            // Re-bind on the UI thread; lparam = -1 marks "device changed".
            events::post(WM_APP_VOLUME, 0, -1);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

pub struct Volume {
    enumerator: IMMDeviceEnumerator,
    endpoint: Option<IAudioEndpointVolume>,
    cb: IAudioEndpointVolumeCallback,
    _dev_cb: IMMNotificationClient,
}

impl Volume {
    pub fn new() -> Option<Volume> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let dev_cb: IMMNotificationClient = DeviceCb.into();
            let _ = enumerator.RegisterEndpointNotificationCallback(&dev_cb);
            let mut v = Volume { enumerator, endpoint: None, cb: VolumeCb.into(), _dev_cb: dev_cb };
            v.rebind();
            Some(v)
        }
    }

    pub fn rebind(&mut self) {
        unsafe {
            if let Some(e) = self.endpoint.take() {
                let _ = e.UnregisterControlChangeNotify(&self.cb);
            }
            let Ok(dev) = self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) else { return };
            if let Ok(ep) = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                let _ = ep.RegisterControlChangeNotify(&self.cb);
                self.endpoint = Some(ep);
            }
        }
    }
}

// ---------------------------------------------------------------- keyboard hook

static HOOK: Mutex<isize> = Mutex::new(0);

unsafe extern "system" fn kb_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && (w.0 as u32 == WM_KEYUP || w.0 as u32 == WM_SYSKEYUP) {
        let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
        match k.vkCode {
            0x14 => events::post(WM_APP_CAPS, 0, 0),
            // Shift, Ctrl, Alt, Space, Win: possible input-language switch chords.
            0x10 | 0x11 | 0x12 | 0x20 | 0x5B | 0x5C | 0xA0..=0xA5 => events::post(WM_APP_LANGCHECK, 0, 0),
            _ => {}
        }
    }
    CallNextHookEx(None, code, w, l)
}

pub fn install_keyboard_hook() {
    unsafe {
        if let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0) {
            *HOOK.lock().unwrap() = h.0 as isize;
        }
    }
}

pub fn caps_on() -> bool {
    unsafe { GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 }
}

/// (language id, short code like "EN", display name like "English (United States)").
pub fn input_language() -> (u32, String, String) {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(tid);
        let lang = (hkl.0 as usize & 0xffff) as u32;
        let mut name = [0u16; 85];
        let n = LCIDToLocaleName(lang, Some(&mut name), 0);
        if n <= 0 {
            return (lang, "??".into(), String::new());
        }
        let loc = PCWSTR(name.as_ptr());
        let mut buf = [0u16; 128];
        let k = GetLocaleInfoEx(loc, LOCALE_SISO639LANGNAME, Some(&mut buf));
        let short = String::from_utf16_lossy(&buf[..(k.max(1) - 1) as usize]).to_uppercase();
        let k = GetLocaleInfoEx(loc, LOCALE_SLOCALIZEDDISPLAYNAME, Some(&mut buf));
        let long = String::from_utf16_lossy(&buf[..(k.max(1) - 1) as usize]);
        (lang, short, long)
    }
}

// ---------------------------------------------------------------- battery

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Battery {
    pub pct: u32,
    pub ac: bool,
    pub present: bool,
}

pub fn battery() -> Battery {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe {
        let _ = GetSystemPowerStatus(&mut s);
    }
    Battery {
        pct: if s.BatteryLifePercent == 255 { 100 } else { s.BatteryLifePercent as u32 },
        ac: s.ACLineStatus == 1,
        present: s.BatteryFlag & 128 == 0 && s.BatteryFlag != 255,
    }
}

// ---------------------------------------------------------------- network

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NetState {
    /// 0 = disconnected, 1 = local only / no internet, 2 = internet.
    pub level: u8,
    pub name: String,
    pub wifi: bool,
}

pub fn net_state() -> NetState {
    let Ok(p) = NetworkInformation::GetInternetConnectionProfile() else { return NetState::default() };
    let level = match p.GetNetworkConnectivityLevel() {
        Ok(NetworkConnectivityLevel::InternetAccess) => 2,
        Ok(NetworkConnectivityLevel::None) | Err(_) => 0,
        _ => 1,
    };
    NetState { level, name: p.ProfileName().map(|h| h.to_string()).unwrap_or_default(), wifi: p.IsWlanConnectionProfile().unwrap_or(false) }
}

pub fn watch_network() {
    std::thread::spawn(|| unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let last = Mutex::new(net_state());
        let _ = NetworkInformation::NetworkStatusChanged(&NetworkStatusChangedEventHandler::new(move |_| {
            let s = net_state();
            let mut l = last.lock().unwrap();
            if *l != s {
                *l = s.clone();
                events::push(Event::Network(s));
            }
            Ok(())
        }));
        // Keep the registration alive for the life of the process.
        loop {
            std::thread::park();
        }
    });
}

// ---------------------------------------------------------------- bluetooth

pub fn watch_bluetooth() {
    std::thread::spawn(|| unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let Ok(sel) = BluetoothDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected) else { return };
        let Ok(watcher) = DeviceInformation::CreateWatcherAqsFilter(&sel) else { return };
        let names: std::sync::Arc<Mutex<HashMap<String, (String, bool)>>> = Default::default();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let (names, ready) = (names.clone(), ready.clone());
            let _ = watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, info| {
                let Some(info) = info.as_ref() else { return Ok(()) };
                let id = info.Id()?.to_string();
                let mut name = info.Name().map(|h| h.to_string()).unwrap_or_default();
                let mut audio = false;
                if let Ok(dev) = BluetoothDevice::FromIdAsync(&HSTRING::from(&id)).and_then(|o| o.get()) {
                    audio = dev.ClassOfDevice().and_then(|c| c.MajorClass()).map(|m| m == BluetoothMajorClass::AudioVideo).unwrap_or(false);
                    if let Ok(n) = dev.Name() {
                        if !n.is_empty() {
                            name = n.to_string();
                        }
                    }
                }
                names.lock().unwrap().insert(id, (name.clone(), audio));
                if audio && ready.load(std::sync::atomic::Ordering::SeqCst) {
                    events::push(Event::Bluetooth(name, true));
                }
                Ok(())
            }));
        }
        {
            let names = names.clone();
            let _ = watcher.Removed(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, upd| {
                if let Some(u) = upd.as_ref() {
                    if let Some((name, audio)) = names.lock().unwrap().remove(&u.Id()?.to_string()) {
                        if audio {
                            events::push(Event::Bluetooth(name, false));
                        }
                    }
                }
                Ok(())
            }));
        }
        let _ = watcher.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(|_, _| Ok(())));
        {
            let ready = ready.clone();
            let _ = watcher.EnumerationCompleted(&TypedEventHandler::new(move |_, _| {
                ready.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }));
        }
        let _ = watcher.Start();
        loop {
            std::thread::park();
        }
    });
}

// ---------------------------------------------------------------- monitors, idle, full screen

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub handle: HMONITOR,
    pub rect: RECT,
    pub primary: bool,
}

unsafe extern "system" fn enum_mon(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let v = &mut *(data.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(h, &mut mi).as_bool() {
        v.push(Monitor { handle: h, rect: mi.rcMonitor, primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0 });
    }
    true.into()
}

/// Monitors ordered left-to-right.
pub fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_mon), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort_by_key(|m| (m.rect.left, m.rect.top));
    v
}

pub fn idle_secs() -> u32 {
    let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut li).as_bool() {
            return GetTickCount().wrapping_sub(li.dwTime) / 1000;
        }
    }
    0
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Is something full screen on the given monitor (games, video, slideshows)?
pub fn fullscreen_on(mon: &RECT, own: HWND) -> bool {
    unsafe {
        if let Ok(s) = SHQueryUserNotificationState() {
            if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE {
                return true;
            }
        }
        // Borderless full-screen windows (browsers, video players) need a geometry check.
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == own {
            return false;
        }
        let cls = class_name(fg);
        if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"].contains(&cls.as_str()) {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        let captioned = style & WS_CAPTION.0 == WS_CAPTION.0;
        !captioned && r.left <= mon.left && r.top <= mon.top && r.right >= mon.right && r.bottom >= mon.bottom
    }
}

struct EnumWinData {
    mon: RECT,
    own: HWND,
    found: bool,
}

unsafe extern "system" fn enum_app_win(h: HWND, l: LPARAM) -> BOOL {
    let d = &mut *(l.0 as *mut EnumWinData);
    if h == d.own || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
        return true.into();
    }
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 {
        return true.into();
    }
    let cls = class_name(h);
    if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Windows.UI.Core.CoreWindow"].contains(&cls.as_str()) {
        return true.into();
    }
    let mut cloaked: u32 = 0;
    if DwmGetWindowAttribute(
        h,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    ).is_ok() && cloaked != 0 {
        return true.into();
    }
    let mut r = RECT::default();
    if GetWindowRect(h, &mut r).is_ok() {
        let w = r.right - r.left;
        let h_rect = r.bottom - r.top;
        if w > 100 && h_rect > 100 && r.right > d.mon.left && r.left < d.mon.right && r.bottom > d.mon.top && r.top < d.mon.bottom {
            d.found = true;
            return false.into();
        }
    }
    true.into()
}

pub fn has_open_windows_on(mon: &RECT, own: HWND) -> bool {
    let mut d = EnumWinData { mon: *mon, own, found: false };
    unsafe {
        let _ = EnumWindows(Some(enum_app_win), LPARAM(&mut d as *mut _ as isize));
    }
    d.found
}

/// Is the user on the desktop homescreen (no normal application window open / active on this monitor)?
pub fn is_desktop_homescreen(mon: &RECT, own: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if !fg.0.is_null() && fg != own {
            let cls = class_name(fg);
            // If the user explicitly focused Progman or WorkerW (clicked desktop wallpaper or Win+D),
            // they are on the desktop homescreen.
            if cls == "Progman" || cls == "WorkerW" {
                return true;
            }
            // If an actual normal application window is in the foreground on this monitor:
            if !["Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&cls.as_str()) {
                let mut r = RECT::default();
                if GetWindowRect(fg, &mut r).is_ok() {
                    let on_this_mon = r.right > mon.left && r.left < mon.right && r.bottom > mon.top && r.top < mon.bottom;
                    if on_this_mon && !IsIconic(fg).as_bool() && IsWindowVisible(fg).as_bool() {
                        let mut cloaked: u32 = 0;
                        let is_cloaked = DwmGetWindowAttribute(
                            fg,
                            DWMWA_CLOAKED,
                            &mut cloaked as *mut _ as *mut _,
                            std::mem::size_of::<u32>() as u32,
                        ).is_ok() && cloaked != 0;
                        if !is_cloaked {
                            return false;
                        }
                    }
                }
            }
        }
        // If foreground is taskbar or null (or foreground window is on another monitor):
        // check whether any open application windows exist on this monitor.
        !has_open_windows_on(mon, own)
    }
}
