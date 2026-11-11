//! System integrations: volume, keyboard (Caps Lock / input language), battery, network,
//! Bluetooth headsets, monitors, idle time and full-screen detection.

#![allow(non_snake_case)]

use crate::events::{self, Event, WM_APP_CAPS, WM_APP_LANGCHECK, WM_APP_VOLUME};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{implement, Result, HSTRING, PCWSTR};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothMajorClass};
use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationUpdate, DeviceWatcher};
use windows::Foundation::TypedEventHandler;
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Foundation::{BOOL, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ};
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

    pub fn change_volume(&self, delta: f32) {
        unsafe {
            let Some(ep) = self.endpoint.as_ref() else { return };
            if let Ok(cur) = ep.GetMasterVolumeLevelScalar() {
                let new_v = (cur + delta).clamp(0.0, 1.0);
                let _ = ep.SetMasterVolumeLevelScalar(new_v, std::ptr::null());
                if new_v > 0.0 && ep.GetMute().map(|m| m.as_bool()).unwrap_or(false) {
                    let _ = ep.SetMute(false, std::ptr::null());
                }
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

#[allow(dead_code)]
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

// ---------------------------------------------------------------- microphone

pub fn mic_in_use() -> bool {
    unsafe {
        let base_path = windows::core::w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone");
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, base_path, 0, KEY_READ, &mut key).is_ok() {
            if check_consent_key(key) {
                let _ = RegCloseKey(key);
                return true;
            }
            let _ = RegCloseKey(key);
        }
        let np_path = windows::core::w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone\NonPackaged");
        if RegOpenKeyExW(HKEY_CURRENT_USER, np_path, 0, KEY_READ, &mut key).is_ok() {
            if check_consent_key(key) {
                let _ = RegCloseKey(key);
                return true;
            }
            let _ = RegCloseKey(key);
        }
    }
    false
}

unsafe fn check_consent_key(key: HKEY) -> bool {
    let mut index = 0;
    let mut name = [0u16; 260];
    while RegEnumKeyW(key, index, Some(&mut name)) == windows::Win32::Foundation::ERROR_SUCCESS {
        index += 1;
        let mut sub = HKEY::default();
        if RegOpenKeyExW(key, PCWSTR(name.as_ptr()), 0, KEY_READ, &mut sub).is_ok() {
            let mut stop = 0u64;
            let mut start = 0u64;
            let mut sz = 8u32;
            let _ = RegQueryValueExW(sub, windows::core::w!("LastUsedTimeStop"), None, None, Some(&mut stop as *mut _ as *mut u8), Some(&mut sz));
            sz = 8;
            let _ = RegQueryValueExW(sub, windows::core::w!("LastUsedTimeStart"), None, None, Some(&mut start as *mut _ as *mut u8), Some(&mut sz));
            let _ = RegCloseKey(sub);
            if stop == 0 && start > 0 {
                return true;
            }
        }
    }
    false
}

pub fn mic_muted() -> Option<bool> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
        let ep = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        Some(ep.GetMute().ok()?.as_bool())
    }
}

pub fn toggle_mic_mute() -> Option<bool> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
        let ep = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        let current = ep.GetMute().ok()?.as_bool();
        let target = !current;
        let _ = ep.SetMute(target, std::ptr::null());
        Some(target)
    }
}

// ---------------------------------------------------------------- audio switcher

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_audio_devices() -> Vec<AudioDevice> {
    let mut list = Vec::new();
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return list };
        let def_id = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
            .and_then(|d| d.GetId())
            .map(|id| id.to_string().unwrap_or_default())
            .unwrap_or_default();
        let Ok(col) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return list };
        let count = col.GetCount().unwrap_or(0);
        for i in 0..count {
            if let Ok(dev) = col.Item(i) {
                let id = dev.GetId().map(|id| id.to_string().unwrap_or_default()).unwrap_or_default();
                let name = get_device_name(&dev).unwrap_or_else(|| "Audio Device".into());
                let is_default = id == def_id;
                list.push(AudioDevice { id, name, is_default });
            }
        }
    }
    list
}

unsafe fn get_device_name(dev: &IMMDevice) -> Option<String> {
    let ps = dev.OpenPropertyStore(windows::Win32::System::Com::STGM_READ).ok()?;
    let pkey = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };
    let pv = ps.GetValue(&pkey).ok()?;
    let s = pv.to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self) -> windows::core::HRESULT;
    fn GetDeviceFormat(&self) -> windows::core::HRESULT;
    fn ResetDeviceFormat(&self) -> windows::core::HRESULT;
    fn SetDeviceFormat(&self) -> windows::core::HRESULT;
    fn GetProcessingPeriod(&self) -> windows::core::HRESULT;
    fn SetProcessingPeriod(&self) -> windows::core::HRESULT;
    fn GetShareMode(&self) -> windows::core::HRESULT;
    fn SetShareMode(&self) -> windows::core::HRESULT;
    fn GetPropertyValue(&self) -> windows::core::HRESULT;
    fn SetPropertyValue(&self) -> windows::core::HRESULT;
    fn SetDefaultEndpoint(&self, wsz_device_id: PCWSTR, role: i32) -> windows::core::HRESULT;
    fn SetEndpointVisibility(&self) -> windows::core::HRESULT;
}

const CLSID_POLICY_CONFIG: windows::core::GUID = windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

pub fn cycle_audio_output() -> Option<String> {
    let devices = list_audio_devices();
    if devices.is_empty() { return None; }
    let cur_idx = devices.iter().position(|d| d.is_default).unwrap_or(0);
    let next_idx = (cur_idx + 1) % devices.len();
    let next_dev = &devices[next_idx];
    set_default_audio_device(&next_dev.id);
    Some(next_dev.name.clone())
}

pub fn set_default_audio_device(device_id: &str) -> bool {
    unsafe {
        let policy: Result<IPolicyConfig> = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL);
        if let Ok(p) = policy {
            let id_w: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = p.SetDefaultEndpoint(PCWSTR(id_w.as_ptr()), 0); // eConsole
            let _ = p.SetDefaultEndpoint(PCWSTR(id_w.as_ptr()), 1); // eMultimedia
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------- downloads

pub fn check_active_downloads() -> Option<crate::events::DownloadInfo> {
    let downloads_dir = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("Downloads"))?;
    if !downloads_dir.exists() { return None; }

    let entries = std::fs::read_dir(downloads_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("crdownload") || ext.eq_ignore_ascii_case("part") || ext.eq_ignore_ascii_case("download") {
                let file_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Downloading").to_string();
                let meta = entry.metadata().ok();
                let bytes = meta.map(|m| m.len()).unwrap_or(0);
                let size_str = if bytes > 1024 * 1024 * 1024 {
                    format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
                } else {
                    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
                };
                return Some(crate::events::DownloadInfo {
                    filename: file_name,
                    size_str,
                    progress: None,
                });
            }
        }
    }
    None
}

// ---------------------------------------------------------------- live activity pipe

pub fn start_pipe_listener() {
    std::thread::Builder::new()
        .name("nuva_pipe".into())
        .spawn(|| {
            unsafe {
                use windows::Win32::System::Pipes::*;
                use windows::Win32::Storage::FileSystem::*;
                let pipe_name = windows::core::w!(r"\\.\pipe\nuva_notch");
                loop {
                    let pipe = CreateNamedPipeW(
                        pipe_name,
                        PIPE_ACCESS_INBOUND,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                        PIPE_UNLIMITED_INSTANCES,
                        65536,
                        65536,
                        0,
                        None,
                    );
                    if pipe.is_invalid() {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    }
                    // A fast client may connect before ConnectNamedPipe runs; that is
                    // reported as ERROR_PIPE_CONNECTED, not a failure.
                    let connected = ConnectNamedPipe(pipe, None).is_ok()
                        || windows::Win32::Foundation::GetLastError()
                            == windows::Win32::Foundation::ERROR_PIPE_CONNECTED;
                    if connected {
                        // A client can connect without sending anything; never block this
                        // thread forever. Poll for data, then drain until the client goes idle.
                        let mut buf = vec![0u8; 65536];
                        let mut read_total = 0usize;
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                        let mut last_data = std::time::Instant::now();
                        loop {
                            let mut avail = 0u32;
                            if PeekNamedPipe(pipe, None, 0, None, Some(&mut avail), None).is_err() {
                                break;
                            }
                            if avail == 0 {
                                // Messages are written in one shot; a short idle gap means done.
                                if read_total > 0
                                    && last_data.elapsed() >= std::time::Duration::from_millis(150)
                                {
                                    break;
                                }
                                if std::time::Instant::now() >= deadline {
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(10));
                                continue;
                            }
                            let remaining = buf.len().saturating_sub(read_total);
                            if remaining == 0 {
                                break;
                            }
                            let want = (avail as usize).min(remaining);
                            let mut read = 0u32;
                            if ReadFile(
                                HANDLE(pipe.0),
                                Some(&mut buf[read_total..read_total + want]),
                                Some(&mut read),
                                None,
                            )
                            .is_err()
                                || read == 0
                            {
                                break;
                            }
                            read_total += read as usize;
                            last_data = std::time::Instant::now();
                            if read_total >= buf.len() {
                                break;
                            }
                        }
                        if read_total > 0 {
                            let text = String::from_utf8_lossy(&buf[..read_total]);
                            handle_pipe_message(&text);
                        }
                        let _ = DisconnectNamedPipe(pipe);
                    }
                    let _ = windows::Win32::Foundation::CloseHandle(pipe);
                }
            }
        })
        .ok();
}

fn handle_pipe_message(text: &str) {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("clear") {
        events::push(events::Event::LiveActivity(None));
        return;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let is_notify = v.get("type").and_then(|t| t.as_str()) == Some("notify") || v.get("notify").is_some();
        if is_notify {
            let title = v.get("title").or_else(|| v.get("notify")).and_then(|t| t.as_str()).unwrap_or("").to_string();
            let sub = v.get("sub").and_then(|s| s.as_str()).unwrap_or("").to_string();
            events::push(events::Event::NotificationToast { title, sub });
        } else if let Some(title) = v.get("title").and_then(|t| t.as_str()) {
            let sub = v.get("sub").and_then(|s| s.as_str()).unwrap_or("");
            let progress = v.get("progress").and_then(|p| p.as_f64()).map(|p| p as f32);
            events::push(events::Event::LiveActivity(Some(events::LiveActivityData {
                title: title.to_string(),
                sub: sub.to_string(),
                progress,
            })));
        }
    } else {
        events::push(events::Event::LiveActivity(Some(events::LiveActivityData {
            title: trimmed.to_string(),
            sub: String::new(),
            progress: None,
        })));
    }
}

static COLOR_HOOK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static COLOR_TARGET: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Read one screen pixel with BitBlt + GetDIBits (works with hardware-accelerated windows
/// where GetPixel returns CLR_INVALID).
pub fn sample_screen_pixel(x: i32, y: i32) -> Option<u32> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        if mem.is_invalid() {
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let bmp = CreateCompatibleBitmap(screen, 1, 1);
        if bmp.is_invalid() {
            let _ = DeleteDC(mem);
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let old = SelectObject(mem, bmp);
        let copied = BitBlt(mem, 0, 0, 1, 1, screen, x, y, SRCCOPY).is_ok();
        let mut pixel = 0u32;
        if copied {
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = 1;
            bi.bmiHeader.biHeight = -1;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB.0;
            let got = GetDIBits(
                mem,
                bmp,
                0,
                1,
                Some(&mut pixel as *mut u32 as *mut core::ffi::c_void),
                &mut bi,
                DIB_RGB_COLORS,
            );
            if got == 0 {
                pixel = 0;
            }
        }
        let _ = SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(None, screen);
        if !copied {
            return None;
        }
        // DIB memory is B,G,R,A; return the COLORREF-style 0x00BBGGRR the rest of the app expects.
        let b = (pixel & 0xff) as u32;
        let g = ((pixel >> 8) & 0xff) as u32;
        let r = ((pixel >> 16) & 0xff) as u32;
        Some(r | (g << 8) | (b << 16))
    }
}

pub fn start_color_picker(hwnd: windows::Win32::Foundation::HWND) {
    unsafe {
        stop_color_picker();
        COLOR_TARGET.store(hwnd.0 as isize, std::sync::atomic::Ordering::SeqCst);
        let hook = windows::Win32::UI::WindowsAndMessaging::SetWindowsHookExW(
            windows::Win32::UI::WindowsAndMessaging::WH_MOUSE_LL,
            Some(mouse_color_hook_proc),
            windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut()),
            0,
        );
        if let Ok(h) = hook {
            COLOR_HOOK.store(h.0 as isize, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

pub fn stop_color_picker() {
    let raw = COLOR_HOOK.swap(0, std::sync::atomic::Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(
                windows::Win32::UI::WindowsAndMessaging::HHOOK(raw as *mut core::ffi::c_void),
            );
        }
    }
}

unsafe extern "system" fn mouse_color_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::*;
    if code >= 0 {
        let msg = wparam.0 as u32;
        let target = COLOR_TARGET.load(std::sync::atomic::Ordering::SeqCst);
        if msg == WM_LBUTTONDOWN {
            let ms = *(lparam.0 as *const MSLLHOOKSTRUCT);
            if let Some(pixel) = sample_screen_pixel(ms.pt.x, ms.pt.y) {
                let r = (pixel & 0xff) as u8;
                let g = ((pixel >> 8) & 0xff) as u8;
                let b = ((pixel >> 16) & 0xff) as u8;
                let hex = format!("#{r:02X}{g:02X}{b:02X}");
                crate::clipboard::set_text(&hex);
                if target != 0 {
                    let _ = PostMessageW(HWND(target as *mut _), events::WM_APP_COLOR_PICKED, WPARAM(pixel as usize), LPARAM(0));
                }
            } else if target != 0 {
                let _ = PostMessageW(HWND(target as *mut _), events::WM_APP_COLOR_CANCELED, WPARAM(0), LPARAM(0));
            }
            stop_color_picker();
            return LRESULT(1);
        } else if msg == WM_RBUTTONDOWN {
            if target != 0 {
                let _ = PostMessageW(HWND(target as *mut _), events::WM_APP_COLOR_CANCELED, WPARAM(0), LPARAM(0));
            }
            stop_color_picker();
            return LRESULT(1);
        }
    }
    let hook = COLOR_HOOK.load(std::sync::atomic::Ordering::SeqCst);
    CallNextHookEx(
        windows::Win32::UI::WindowsAndMessaging::HHOOK(hook as *mut core::ffi::c_void),
        code,
        wparam,
        lparam,
    )
}
