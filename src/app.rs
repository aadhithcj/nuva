//! The notch window: state, input, timers, alerts and system events.

use crate::anim::Spring;
use crate::clipboard::Clipboard;
use crate::events::{self, Event, *};
use crate::gfx::{ico, pal, Canvas, Color, CompSurface, Gfx, Rect};
use crate::lyrics::{self, Lyrics};
use crate::media::{Media, MediaCmd, MediaState};
use crate::notifications::{self, Notif};
use crate::pomodoro::{Mode, Pomodoro};
use crate::settings::{self, Settings};
use crate::settings_window::SettingsWindow;
use crate::shelf::{self, Dropped, Shelf};
use crate::sound::{self, Chime};
use crate::stats::Stats;
use crate::system::{self, Battery, NetState, Volume};
use crate::time;
use crate::todo::{TaskEvent, Todo};
use crate::tray;
use crate::ui::Action;
use crate::water::{Water, WaterAlert};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::{Duration, Instant};
use windows::core::{w, Interface, GUID, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1RenderTarget};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, MonitorFromPoint, SetWindowRgn, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Graphics::Imaging::IWICBitmapSource;
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::RegisterPowerSettingNotification;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------------------------------------------------------------- constants

/// Window size in DIPs; the notch is drawn top-centre inside it.
pub const WIN_W: f32 = 820.0;
pub const WIN_H: f32 = 330.0;
pub const FLARE: f32 = 7.0;

const T_TICK: usize = 1;
const T_ANIM: usize = 2;
const T_OPEN: usize = 3;
const T_CLOSE: usize = 4;
const T_LYRIC: usize = 5;
const T_NET: usize = 6;
const T_LANG: usize = 8;
const T_EDGE_POLL: usize = 9;
const T_VOLUME_COOLDOWN: usize = 10;

const GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);
const GUID_BATTERY_PERCENTAGE: GUID = GUID::from_u128(0xa7ad8041_b45a_4cae_87a3_eecbb468a9e1);
const GUID_MONITOR_BRIGHTNESS: GUID = GUID::from_u128(0x8ffee2c6_2d01_46be_adb9_398addc5b4ff);
const GUID_POLICY_BRIGHTNESS: GUID = GUID::from_u128(0xaded5e82_b909_4619_9949_f5d71dac0bcb);

#[repr(C)]
struct PowerSetting {
    guid: GUID,
    len: u32,
    data: [u8; 4],
}

// ---------------------------------------------------------------- view model

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Page {
    Home,
    Timer,
    Water,
    Todo,
    Calendar,
    Shelf,
    Clipboard,
    Notifs,
    Stats,
    Calc,
}

impl Page {
    pub fn icon(self) -> char {
        match self {
            Page::Home => ico::MUSIC,
            Page::Timer => ico::TIMER,
            Page::Water => ico::DROP,
            Page::Todo => ico::CHECKLIST,
            Page::Calendar => ico::CALENDAR,
            Page::Shelf => ico::FOLDER,
            Page::Clipboard => ico::CLIPBOARD,
            Page::Notifs => ico::BELL,
            Page::Stats => ico::STATS,
            Page::Calc => ico::CALCULATOR,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Page::Home => "Now",
            Page::Timer => "Focus",
            Page::Water => "Water",
            Page::Todo => "To-do",
            Page::Calendar => "Calendar",
            Page::Shelf => "Shelf",
            Page::Clipboard => "Clipboard",
            Page::Notifs => "Notifications",
            Page::Stats => "System",
            Page::Calc => "Calculator",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Pill,
    Toast,
    Alert,
    Open(Page),
}


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToastKind {
    Volume,
    Brightness,
    Caps,
    Language,
    Battery,
    Network,
    Bluetooth,
    Notification,
    Info,
}

pub struct Toast {
    #[allow(dead_code)]
    pub kind: ToastKind,
    pub icon: char,
    pub color: Color,
    pub title: String,
    pub sub: String,
    pub level: Option<f32>,
    pub until: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AlertAct {
    Dismiss,
    PomoStart,
    WaterDrank,
    WaterRefilled,
    WaterSnooze,
    TaskAck(u64),
    TaskLater(u64, TaskEvent),
    TaskDone(u64),
}

pub struct Alert {
    pub icon: char,
    pub color: Color,
    pub title: String,
    pub sub: String,
    pub buttons: Vec<(String, AlertAct)>,
    pub created: Instant,
}

pub struct Editor {
    pub id: Option<u64>,
    pub title: String,
    pub day: i64,
    pub start: u32,
    pub dur: u32,
    pub focused: bool,
}

// ---------------------------------------------------------------- app

pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub surf: Option<CompSurface>,
    pub settings: Settings,
    pub scale: f32,
    win_px: (i32, i32, i32, i32),
    pub monitor: RECT,
    pub h24: bool,
    started: Instant,

    // shape animation
    pub sw: Spring,
    pub sh: Spring,
    pub sr: Spring,
    pub fade: Spring,
    last_view: View,
    last_frame: Instant,
    anim_on: bool,
    dirty: bool,
    region: (i32, i32, i32),
    hidden: bool,

    // interaction
    pub expanded: bool,
    pub page: Page,
    pub mouse: Option<(f32, f32)>,
    tracking: bool,
    pub hits: Vec<(Rect, Action)>,
    pressed: Option<(Action, (f32, f32))>,
    pub seeking: Option<f64>,
    pub drag_over: bool,
    pub scroll: HashMap<Page, f32>,

    // modules
    pub media: MediaState,
    media_ctl: Media,
    art_src: Option<IWICBitmapSource>,
    art_key: String,
    pub media_accent: Option<Color>,
    pub audio_spectrum: [f32; 5],
    pub show_split: bool,
    pub lyrics: Option<Lyrics>,
    pub lyrics_loading: bool,
    lyrics_key: String,
    lyrics_gen: u64,
    pub lyric_scroll: Spring,
    pub pomo: Pomodoro,
    pub water: Water,
    pub todo: Todo,
    pub editor: Option<Editor>,
    pub cal_anchor: i64,
    pub cal_sel: i64,
    pub cal_week: bool,
    pub shelf: Shelf,
    shelf_px: HashMap<String, (u32, u32, Vec<u8>)>,
    pub clip: Clipboard,
    pub notifs: Vec<Notif>,
    seen_notifs: Option<HashSet<u32>>,
    dismissed_notifs: HashSet<u32>,
    notif_srcs: HashMap<String, IWICBitmapSource>,
    pub stats: Stats,
    pub alerts: VecDeque<Alert>,
    pub toast: Option<Toast>,
    pub drop_action_zone: Option<crate::shelf::DropAction>,
    pub drag_has_images: bool,
    volume: Option<Volume>,
    battery: Battery,
    pending_net: Option<NetState>,
    net: NetState,
    lang: u32,
    lang_fg: isize,
    last_minute: u32,
    tick_n: u64,
    bmp_cache: HashMap<String, ID2D1Bitmap>,
    pub settings_win: Option<SettingsWindow>,
    preview: bool,
    pub is_desktop: bool,
    pub window_hovered: bool,
    pub last_fg: isize,
    pub mic_active: bool,
    pub mic_muted: bool,
    pub picking_color: bool,
    pub preview_color: Option<(u8, u8, u8)>,
    pub cmd: crate::cmd::CommandPalette,
    pub live_activity: Option<crate::events::LiveActivityData>,
    pub download: Option<crate::events::DownloadInfo>,
    pub last_volume_scroll: Option<Instant>,
    pub close_deadline: Option<Instant>,
}

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());

/// Run `f` against the single app instance (UI thread only).
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let p = APP.load(Ordering::SeqCst);
    if p.is_null() {
        None
    } else {
        Some(f(unsafe { &mut *p }))
    }
}

pub fn run(open_settings: bool, preview: Option<String>) {
    unsafe {
        let gfx = match Gfx::new() {
            Ok(g) => g,
            Err(e) => {
                let msg = HSTRING::from(format!("Nuva could not start graphics: {e}"));
                MessageBoxW(None, &msg, w!("Nuva"), MB_ICONERROR);
                return;
            }
        };
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hIcon: crate::tray::app_icon(false),
            hIconSm: crate::tray::app_icon(true),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("NuvaNotchWindow"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        let Ok(hwnd) = CreateWindowExW(ex, w!("NuvaNotchWindow"), w!("Nuva"), WS_POPUP, 0, 0, 100, 100, None, None, hinst, None) else {
            return;
        };
        events::set_target(hwnd);

        let settings = Settings::load();
        let media_ctl = Media::start();
        let mut app = Box::new(App {
            hwnd,
            gfx,
            surf: None,
            scale: 1.0,
            win_px: (0, 0, 0, 0),
            monitor: RECT::default(),
            h24: time::is_24h(),
            started: Instant::now(),
            sw: Spring::new(180.0),
            sh: Spring::new(30.0),
            sr: Spring::new(12.0),
            fade: Spring::new(1.0),
            last_view: View::Pill,
            last_frame: Instant::now(),
            anim_on: false,
            dirty: true,
            region: (0, 0, 0),
            hidden: false,
            expanded: false,
            page: Page::Home,
            mouse: None,
            tracking: false,
            hits: Vec::new(),
            pressed: None,
            seeking: None,
            drag_over: false,
            scroll: HashMap::new(),
            media: MediaState::default(),
            media_ctl,
            art_src: None,
            art_key: String::new(),
            media_accent: None,
            audio_spectrum: [0.0; 5],
            show_split: false,
            lyrics: None,
            lyrics_loading: false,
            lyrics_key: String::new(),
            lyrics_gen: 0,
            lyric_scroll: Spring::new(0.0),
            pomo: Pomodoro::new(&settings),
            water: Water::load(),
            todo: Todo::load(),
            editor: None,
            cal_anchor: 0,
            cal_sel: time::today(),
            cal_week: settings.calendar_week,
            shelf: Shelf::load(settings.shelf_clear),
            shelf_px: HashMap::new(),
            clip: Clipboard::new(settings.clip_persist),
            notifs: Vec::new(),
            seen_notifs: None,
            dismissed_notifs: HashSet::new(),
            notif_srcs: HashMap::new(),
            stats: Stats::new(),
            alerts: VecDeque::new(),
            toast: None,
            drop_action_zone: None,
            drag_has_images: false,
            volume: None,
            battery: system::battery(),
            pending_net: None,
            net: system::net_state(),
            lang: 0,
            lang_fg: 0,
            last_minute: 99,
            tick_n: 0,
            bmp_cache: HashMap::new(),
            settings_win: None,
            preview: false,
            is_desktop: true,
            window_hovered: false,
            last_fg: 0,
            mic_active: false,
            mic_muted: false,
            picking_color: false,
            preview_color: None,
            cmd: crate::cmd::CommandPalette::new(),
            live_activity: None,
            download: None,
            last_volume_scroll: None,
            close_deadline: None,
            settings,
        });
        app.cal_reset();
        APP.store(&mut *app as *mut App, Ordering::SeqCst);

        app.place_window();
        shelf::register(hwnd, &shelf::DropTarget::new());
        tray::add(hwnd);
        let _ = AddClipboardFormatListener(hwnd);
        for g in [GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE, GUID_MONITOR_BRIGHTNESS, GUID_POLICY_BRIGHTNESS] {
            let _ = RegisterPowerSettingNotification(HANDLE(hwnd.0), &g, REGISTER_NOTIFICATION_FLAGS(0));
        }
        system::install_keyboard_hook();
        let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        system::watch_network();
        system::watch_bluetooth();
        notifications::set_enabled(app.settings.notifications);
        notifications::start();
        system::start_pipe_listener();
        let _ = SetWindowDisplayAffinity(hwnd, WINDOW_DISPLAY_AFFINITY(if app.settings.exclude_capture { 17 } else { 0 }));
        app.volume = Volume::new();
        app.lang = system::input_language().0;
        let is_desktop = system::is_desktop_homescreen(&app.monitor, hwnd);
        app.is_desktop = is_desktop;
        if !is_desktop && app.settings.autohide_window && preview.is_none() {
            app.sh.snap(0.0);
            app.sr.snap(0.0);
        }
        SetTimer(hwnd, T_TICK, 1000, None);
        SetTimer(hwnd, T_EDGE_POLL, 25, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if let Some(p) = preview {
            let all = [Page::Home, Page::Timer, Page::Water, Page::Todo, Page::Calendar, Page::Shelf, Page::Clipboard, Page::Notifs, Page::Stats, Page::Calc];
            app.page = all.into_iter().find(|x| x.title().eq_ignore_ascii_case(&p) || format!("{x:?}").eq_ignore_ascii_case(&p) || p.eq_ignore_ascii_case("calc")).unwrap_or(Page::Home);
            if app.page == Page::Calc {
                app.set_activatable(true);
            }
            app.preview = true;
            app.expanded = true;
            if p.eq_ignore_ascii_case("lyrics") {
                app.show_split = true;
            }
        }
        app.request_frame();
        if open_settings {
            app.open_settings();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray::remove(hwnd);
        APP.store(std::ptr::null_mut(), Ordering::SeqCst);
        drop(app);
    }
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    events::post(WM_APP_FOREGROUND, 0, 0);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    static TASKBAR_CREATED: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let tbc = *TASKBAR_CREATED.get_or_init(|| RegisterWindowMessageW(w!("TaskbarCreated")));
    if msg == tbc && tbc != 0 {
        tray::add(hwnd);
        return LRESULT(0);
    }
    if let Some(r) = with(|a| a.handle(msg, wp, lp)).flatten() {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn lo(v: isize) -> i32 {
    (v & 0xffff) as i16 as i32
}
fn hi(v: isize) -> i32 {
    ((v >> 16) & 0xffff) as i16 as i32
}

impl App {
    // ------------------------------------------------ window placement

    pub fn place_window(&mut self) {
        let mons = system::monitors();
        let mon = if self.settings.display > 0 {
            mons.get(self.settings.display as usize - 1).copied()
        } else {
            None
        }
        .or_else(|| mons.iter().find(|m| m.primary).copied())
        .or_else(|| mons.first().copied());
        let (rect, handle) = match mon {
            Some(m) => (m.rect, m.handle),
            None => unsafe {
                let h = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
                (RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, h)
            },
        };
        let (mut dx, mut dy) = (96u32, 96u32);
        unsafe {
            let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        }
        let dpi_scale = dx as f32 / 96.0;
        let size = if self.settings.size == 0 {
            let dip_w = (rect.right - rect.left) as f32 / dpi_scale;
            if dip_w >= 2400.0 {
                1.15
            } else {
                1.0
            }
        } else {
            self.settings.size as f32 / 100.0
        };
        self.scale = dpi_scale * size;
        self.monitor = rect;
        let w = (WIN_W * self.scale).ceil() as i32;
        let h = (WIN_H * self.scale).ceil() as i32;
        let x = rect.left + ((rect.right - rect.left) - w) / 2;
        let y = rect.top;
        self.win_px = (x, y, w, h);
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
        }
        match &mut self.surf {
            Some(s) => {
                if s.resize(w as u32, h as u32).is_err() {
                    self.surf = None;
                }
            }
            None => {}
        }
        if self.surf.is_none() {
            self.surf = CompSurface::new(&self.gfx, self.hwnd, w as u32, h as u32).ok();
            self.bmp_cache.clear();
        }
        self.region = (0, 0, 0);
        self.request_frame();
    }

    // ------------------------------------------------ frame scheduling

    pub fn request_frame(&mut self) {
        self.dirty = true;
        if !self.anim_on {
            self.anim_on = true;
            self.last_frame = Instant::now();
            unsafe {
                SetTimer(self.hwnd, T_ANIM, 8, None);
            }
        }
    }

    fn anim_frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let view = self.view();
        if view != self.last_view {
            self.on_view_change(view);
        }
        let (tw, mut th, mut tr) = self.view_size(view);
        if !self.is_notch_visible() {
            th = 0.0;
            tr = 0.0;
        }
        self.sw.set(tw);
        self.sh.set(th);
        self.sr.set(tr);
        let k = if self.settings.fast_animation { 520.0 } else { 260.0 };
        let mut moving = false;
        moving |= self.sw.step(dt, k, 0.82);
        moving |= self.sh.step(dt, k, 0.86);
        moving |= self.sr.step(dt, k, 0.9);
        moving |= self.fade.step(dt, k * 1.2, 1.0);
        moving |= self.lyric_scroll.step(dt, 180.0, 1.0);
        if moving || self.dirty {
            self.dirty = false;
            self.render();
            // Drawing can retarget the lyric scroll; keep animating if it did.
            moving |= self.lyric_scroll.moving();
        }
        if !moving && !self.dirty {
            self.anim_on = false;
            unsafe {
                let _ = KillTimer(self.hwnd, T_ANIM);
            }
        }
    }

    fn on_view_change(&mut self, v: View) {
        // Crossfade content when what we show changes.
        if std::mem::discriminant(&v) != std::mem::discriminant(&self.last_view) || matches!(v, View::Open(_)) {
            self.fade.snap(0.0);
            self.fade.set(1.0);
        }
        if let View::Open(Page::Stats) = v {
            self.stats.sample();
        }
        self.last_view = v;
        self.schedule_lyric_timer();
    }

    fn render(&mut self) {
        let Some(surf) = &self.surf else { return };
        let dc = surf.dc.clone();
        let _ = surf;
        let rt: ID2D1RenderTarget = match dc.cast() {
            Ok(r) => r,
            Err(_) => return,
        };
        unsafe {
            dc.SetDpi(96.0 * self.scale, 96.0 * self.scale);
            dc.BeginDraw();
        }
        let mut c = match Canvas::new(rt, self.gfx.clone(), self.mouse) {
            Ok(c) => c,
            Err(_) => {
                unsafe {
                    let _ = dc.EndDraw(None, None);
                }
                return;
            }
        };
        c.clear(Color(0.0, 0.0, 0.0, 0.0));
        let view = self.last_view;
        if !self.hidden {
            self.draw(&mut c, view);
        }
        let ok = unsafe { dc.EndDraw(None, None) }.is_ok() && self.surf.as_ref().map(|s| s.present().is_ok()).unwrap_or(false);
        self.hits = std::mem::take(&mut c.hits);
        self.update_region();
        if !ok {
            // Device lost: rebuild the surface on the next frame.
            self.surf = None;
            self.bmp_cache.clear();
            self.place_window();
        }
    }

    fn update_region(&mut self) {
        let s = self.scale;
        let cx = WIN_W / 2.0;
        let x0 = ((cx - self.sw.v / 2.0 - FLARE) * s).floor() as i32;
        let x1 = ((cx + self.sw.v / 2.0 + FLARE) * s).ceil() as i32;
        let h = (self.sh.v * s).ceil() as i32;
        if h <= 0 {
            if self.region != (0, 0, 0) {
                self.region = (0, 0, 0);
                unsafe {
                    let rgn = CreateRectRgn(0, 0, 0, 0);
                    SetWindowRgn(self.hwnd, rgn, true);
                }
            }
            return;
        }
        let h = h + 1;
        if (x0, x1, h) != self.region {
            self.region = (x0, x1, h);
            unsafe {
                let rgn = CreateRectRgn(x0, 0, x1, h);
                SetWindowRgn(self.hwnd, rgn, true);
            }
        }
    }

    pub fn notch_rect(&self) -> Rect {
        Rect::new(WIN_W / 2.0 - self.sw.v / 2.0, 0.0, self.sw.v, self.sh.v)
    }

    pub fn dropzone_rect(r: Rect, i: usize) -> Rect {
        let gap = 8.0;
        let card_w = (r.w - 3.0 * gap) / 4.0;
        Rect::new(r.x + i as f32 * (card_w + gap), r.y, card_w, r.h)
    }

    // ------------------------------------------------ what to show

    pub fn pages(&self) -> Vec<Page> {
        let mut v = Vec::new();
        for key in self.settings.page_order_list() {
            match key.as_str() {
                "home" => if self.settings.page_home { v.push(Page::Home); },
                "timer" => if self.settings.page_timer { v.push(Page::Timer); },
                "water" => if self.settings.water && self.settings.page_water { v.push(Page::Water); },
                "todo" => if self.settings.page_todo { v.push(Page::Todo); },
                "calendar" => if self.settings.page_calendar { v.push(Page::Calendar); },
                "shelf" => if self.settings.page_shelf { v.push(Page::Shelf); },
                "clipboard" => if self.settings.clipboard && self.settings.page_clipboard { v.push(Page::Clipboard); },
                "notifs" => if self.settings.notifications && self.settings.page_notifs { v.push(Page::Notifs); },
                "stats" => if self.settings.stats && self.settings.page_stats { v.push(Page::Stats); },
                "calc" => if self.settings.page_calc { v.push(Page::Calc); },
                _ => {}
            }
        }
        if v.is_empty() {
            v.push(Page::Home);
        }
        v
    }

    pub fn view(&self) -> View {
        if !self.alerts.is_empty() {
            return View::Alert;
        }
        if self.expanded {
            let pages = self.pages();
            return View::Open(if pages.contains(&self.page) { self.page } else { Page::Home });
        }
        if self.toast.is_some() {
            return View::Toast;
        }
        View::Pill
    }

    pub fn pill_has_media(&self) -> bool {
        self.media.active && (self.media.playing || !self.media.title.is_empty())
    }

    pub fn pill_lyric(&self) -> Option<String> {
        if !self.settings.lyrics_pill || !self.media.playing {
            return None;
        }
        let l = self.lyrics.as_ref()?;
        let i = l.current(self.media.position())?;
        let t = l.lines[i].text.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    // ------------------------------------------------ message handling

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if let Some(sw) = &self.settings_win {
            if msg != WM_TIMER && sw.hwnd.0.is_null() {
                self.settings_win = None;
            }
        }
        match msg {
            WM_TIMER => self.on_timer(wp.0),
            WM_MOUSEMOVE => self.on_mouse_move(lo(lp.0) as f32 / self.scale, hi(lp.0) as f32 / self.scale),
            WM_MOUSELEAVE => {
                self.tracking = false;
                if self.seeking.is_none() {
                    self.mouse = None;
                }
                unsafe {
                    let _ = KillTimer(self.hwnd, T_OPEN);
                }
                if self.close_deadline.is_none() {
                    self.close_deadline = Some(Instant::now() + Duration::from_millis(280));
                }
                self.request_frame();
            }
            WM_LBUTTONDOWN => self.on_down(lo(lp.0) as f32 / self.scale, hi(lp.0) as f32 / self.scale),
            WM_LBUTTONUP => self.on_up(lo(lp.0) as f32 / self.scale, hi(lp.0) as f32 / self.scale),
            WM_RBUTTONUP => tray::show_menu(self.hwnd, &self.settings),
            WM_MOUSEWHEEL => {
                let delta = hi(wp.0 as isize) as f32 / 120.0;
                self.on_wheel(delta);
            }
            WM_MOUSEHWHEEL => {
                let delta = hi(wp.0 as isize) as f32 / 120.0;
                self.on_wheel(-delta);
            }
            WM_MOUSEACTIVATE => {
                let focus = self.editor.as_ref().map(|e| e.focused).unwrap_or(false) || self.page == Page::Calc;
                return Some(LRESULT(if focus { MA_ACTIVATE } else { MA_NOACTIVATE } as isize));
            }
            WM_CHAR => self.on_char(wp.0 as u32),
            WM_KILLFOCUS => {
                if let Some(e) = &mut self.editor {
                    e.focused = false;
                    self.set_activatable(false);
                    self.request_frame();
                }
            }
            WM_APP_EVENT => {
                for e in events::drain() {
                    self.on_event(e);
                }
            }
            WM_APP_TRAY => match lp.0 as u32 {
                WM_LBUTTONUP => self.open_settings(),
                WM_RBUTTONUP | WM_CONTEXTMENU => tray::show_menu(self.hwnd, &self.settings),
                _ => {}
            },
            WM_COMMAND => self.on_command(wp.0 & 0xffff),
            WM_APP_SHOW_SETTINGS => self.open_settings(),
            WM_APP_CAPS => {
                if self.settings.hud_caps {
                    let on = system::caps_on();
                    self.show_toast(ToastKind::Caps, ico::CAPS, if on { pal::ORANGE } else { pal::DIM }, "Caps Lock", if on { "On" } else { "Off" }, None, 1.4);
                }
            }
            WM_APP_LANGCHECK => unsafe {
                SetTimer(self.hwnd, T_LANG, 140, None);
            },
            WM_APP_VOLUME => {
                if lp.0 == -1 {
                    if let Some(v) = &mut self.volume {
                        v.rebind();
                    }
                } else if self.settings.hud_volume && self.started.elapsed() > Duration::from_secs(2) {
                    let level = wp.0 as f32 / 1000.0;
                    let muted = lp.0 != 0;
                    let icon = if muted || level <= 0.001 { ico::MUTE } else { ico::VOLUME };
                    let sub = if muted { "Muted".to_string() } else { format!("{}", (level * 100.0).round()) };
                    self.show_toast(ToastKind::Volume, icon, pal::WHITE, "Volume", &sub, Some(if muted { 0.0 } else { level }), 1.6);
                }
            }
            WM_APP_FOREGROUND => {
                self.check_fullscreen();
                self.check_desktop_state();
            }
            events::WM_APP_COLOR_PICKED => {
                let pixel = wp.0 as u32;
                let r = (pixel & 0xff) as u8;
                let g = ((pixel >> 8) & 0xff) as u8;
                let b = ((pixel >> 16) & 0xff) as u8;
                let hex = format!("#{r:02X}{g:02X}{b:02X}");
                self.clip.record_text(&hex);
                self.picking_color = false;
                self.show_toast(ToastKind::Notification, ico::CHECK, pal::GREEN, "Color Copied", &hex, None, 2.5);
                self.request_frame();
            }
            WM_POWERBROADCAST => {
                if wp.0 == 0x8013 && lp.0 != 0 {
                    let ps = unsafe { &*(lp.0 as *const PowerSetting) };
                    self.on_power_setting(ps.guid, u32::from_le_bytes(ps.data));
                }
                return Some(LRESULT(1));
            }
            WM_CLIPBOARDUPDATE => {
                if self.settings.clipboard && self.clip.capture(self.hwnd) && self.page == Page::Clipboard {
                    self.request_frame();
                }
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                self.h24 = time::is_24h();
                unsafe {
                    SetTimer(self.hwnd, T_NET + 100, 400, None);
                }
            }
            WM_DESTROY => unsafe { PostQuitMessage(0) },
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            T_ANIM => self.anim_frame(),
            T_TICK => self.tick(),
            T_EDGE_POLL => self.on_edge_poll(),
            T_OPEN => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_OPEN);
                }
                let is_vol_cooldown = self.last_volume_scroll.as_ref().map(|t| t.elapsed() < Duration::from_millis(2000)).unwrap_or(false);
                if self.cursor_in_notch() && !self.expanded && !is_vol_cooldown {
                    self.expand(true);
                }
            }
            T_VOLUME_COOLDOWN => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_VOLUME_COOLDOWN);
                }
                if self.cursor_in_notch() && !self.expanded && !self.settings.open_click && self.alerts.is_empty() {
                    self.expand(true);
                }
            }
            T_CLOSE => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_CLOSE);
                }
                let editing = self.editor.as_ref().map(|e| e.focused).unwrap_or(false);
                if !self.cursor_in_notch() && !editing && self.seeking.is_none() && !self.drag_over {
                    self.expand(false);
                    if !self.is_desktop {
                        self.window_hovered = false;
                    }
                    self.request_frame();
                }
            }
            T_LYRIC => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_LYRIC);
                }
                self.request_frame();
                self.schedule_lyric_timer();
            }
            T_NET => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_NET);
                }
                if let Some(s) = self.pending_net.take() {
                    self.show_network(s);
                }
            }
            T_LANG => {
                unsafe {
                    let _ = KillTimer(self.hwnd, T_LANG);
                }
                self.check_language();
            }
            x if x == T_NET + 100 => {
                unsafe {
                    let _ = KillTimer(self.hwnd, x);
                }
                self.place_window();
            }
            _ => {}
        }
    }

    fn cursor_in_notch(&self) -> bool {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let x = (p.x - self.win_px.0) as f32 / self.scale;
        let y = (p.y - self.win_px.1) as f32 / self.scale;
        let r = self.notch_rect();
        let target_h = self.sh.target.max(crate::notch::PILL_H);
        let max_y = self.sh.v.max(target_h);
        x >= r.x - FLARE && x <= r.r() + FLARE && y >= -2.0 && y <= max_y + 4.0
    }

    pub fn expand(&mut self, open: bool) {
        if self.expanded == open || (self.preview && !open) {
            return;
        }
        self.expanded = open;
        if open {
            if self.page == Page::Calc {
                self.set_activatable(true);
            }
            // Music starting is the most common reason to open; land on it.
            if self.media.playing && self.page != Page::Home && !self.drag_over && self.page != Page::Calc {
                self.page = Page::Home;
            }
        } else {
            if let Some(e) = &self.editor {
                if !e.focused {
                    self.editor = None;
                }
            }
            self.set_activatable(false);
        }
        self.request_frame();
    }

    fn on_mouse_move(&mut self, x: f32, y: f32) {
        self.mouse = Some((x, y));
        self.window_hovered = true;
        self.close_deadline = None;
        if !self.tracking {
            self.tracking = true;
            let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut t);
            }
        }
        unsafe {
            let _ = KillTimer(self.hwnd, T_CLOSE);
        }
        let is_vol_cooldown = self.last_volume_scroll.as_ref().map(|t| t.elapsed() < Duration::from_millis(2000)).unwrap_or(false);
        if !self.expanded && !self.settings.open_click && self.alerts.is_empty() && !is_vol_cooldown {
            unsafe {
                SetTimer(self.hwnd, T_OPEN, 90, None);
            }
        }
        if self.seeking.is_some() {
            if let Some(r) = self.hits.iter().find(|(_, a)| *a == Action::SeekBar).map(|(r, _)| *r) {
                let f = ((x - r.x) / r.w).clamp(0.0, 1.0) as f64;
                self.seeking = Some(f * self.media.dur);
            }
        }
        // Start a shelf drag-out once the pointer travels far enough.
        if let Some((Action::ShelfItem(i), (px, py))) = self.pressed.clone() {
            if (x - px).hypot(y - py) > 6.0 {
                self.pressed = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(it) = self.shelf.items.get(i) {
                    let p = it.path.clone();
                    shelf::drag_out(&[p]);
                }
            }
        }
        self.request_frame();
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().rev().find(|(r, _)| r.contains((x, y))).map(|(_, a)| a.clone())
    }

    fn on_down(&mut self, x: f32, y: f32) {
        if self.picking_color {
            unsafe {
                let mut pt = windows::Win32::Foundation::POINT::default();
                if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt).is_ok() {
                    let hdc = windows::Win32::Graphics::Gdi::GetDC(None);
                    let pixel = windows::Win32::Graphics::Gdi::GetPixel(hdc, pt.x, pt.y);
                    let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc);
                    let r = (pixel.0 & 0xff) as u8;
                    let g = ((pixel.0 >> 8) & 0xff) as u8;
                    let b = ((pixel.0 >> 16) & 0xff) as u8;
                    let hex = format!("#{r:02X}{g:02X}{b:02X}");
                    crate::clipboard::set_text(&hex);
                    self.clip.record_text(&hex);
                    self.show_toast(ToastKind::Notification, ico::CHECK, pal::GREEN, "Color Copied", &hex, None, 2.5);
                }
            }
            self.picking_color = false;
            self.preview_color = None;
            self.request_frame();
            return;
        }

        if !self.expanded && self.alerts.is_empty() {
            self.expand(true);
            return;
        }
        let a = self.hit_at(x, y);
        if let Some(Action::SeekBar) = a {
            if self.media.can_seek && self.media.dur > 0.0 {
                if let Some(r) = self.hits.iter().find(|(_, a)| *a == Action::SeekBar).map(|(r, _)| *r) {
                    self.seeking = Some(((x - r.x) / r.w).clamp(0.0, 1.0) as f64 * self.media.dur);
                }
            }
        }
        if let Some(a) = a {
            self.pressed = Some((a, (x, y)));
            unsafe {
                SetCapture(self.hwnd);
            }
        }
        // Clicking outside the text field takes focus away from it.
        let on_field = self.hit_at(x, y) == Some(Action::TodoField);
        if let Some(e) = &mut self.editor {
            if !on_field && e.focused {
                e.focused = false;
                self.set_activatable(false);
            }
        }
        self.request_frame();
    }

    fn on_up(&mut self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        if let Some(target) = self.seeking.take() {
            self.media_ctl.send(MediaCmd::Seek(target));
            self.media.pos = target;
            self.media.stamp = time::utc_ticks();
            self.pressed = None;
            self.request_frame();
            return;
        }
        let pressed = self.pressed.take();
        let a = self.hit_at(x, y);
        if let (Some((p, _)), Some(a)) = (pressed, a) {
            if p == a {
                self.dispatch(a);
            }
        }
        self.request_frame();
    }

    fn on_wheel(&mut self, delta: f32) {
        let Some((x, y)) = self.mouse else { return };
        match self.view() {
            View::Open(page) => {
                let (_, h, _) = self.view_size(View::Open(page));
                // Wheel over the page dots or header switches pages.
                if y > h - 26.0 || y < 30.0 {
                    let pages = self.pages();
                    if let Some(i) = pages.iter().position(|p| *p == page) {
                        let n = (i as i32 - delta.signum() as i32).clamp(0, pages.len() as i32 - 1) as usize;
                        self.page = pages[n];
                        if self.page == Page::Calc {
                            self.set_activatable(true);
                        } else if self.editor.is_none() {
                            self.set_activatable(false);
                        }
                    }
                } else if page == Page::Home && self.show_split && x > WIN_W / 2.0 {
                    // Manual lyric scrolling is not needed; ignore.
                } else {
                    let s = self.scroll.entry(page).or_insert(0.0);
                    *s = (*s - delta * 40.0).max(0.0);
                }
            }
            View::Pill | View::Toast => {
                if self.settings.scroll_volume {
                    self.last_volume_scroll = Some(Instant::now());
                    unsafe {
                        let _ = KillTimer(self.hwnd, T_OPEN);
                        let _ = SetTimer(self.hwnd, T_VOLUME_COOLDOWN, 2000, None);
                    }
                    self.change_volume(delta * 0.02);
                }
            }
            _ => {}
        }
        self.request_frame();
    }

    pub fn change_volume(&mut self, delta: f32) {
        if let Some(v) = &self.volume {
            v.change_volume(delta);
        }
    }

    pub fn set_activatable(&mut self, on: bool) {
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let na = WS_EX_NOACTIVATE.0 as isize;
            let new = if on { ex & !na } else { ex | na };
            if new != ex {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new);
            }
            if on {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.hwnd);
            }
        }
    }

    fn on_char(&mut self, ch: u32) {
        if self.picking_color && ch == 27 {
            system::stop_color_picker();
            self.picking_color = false;
            self.show_toast(ToastKind::Notification, ico::CLOSE, pal::DIM, "Color Picker", "Cancelled", None, 1.5);
            self.request_frame();
            return;
        }

        if self.page == Page::Calc {
            match ch {
                8 => {
                    self.cmd.backspace();
                    self.request_frame();
                }
                13 => {
                    self.dispatch(Action::CommandRun);
                }
                27 => {
                    self.cmd.clear();
                    self.set_activatable(false);
                    self.page = Page::Home;
                    self.request_frame();
                }
                22 => {
                    if let Some(t) = read_clip_text(self.hwnd) {
                        for c in t.chars() {
                            if !c.is_control() {
                                self.cmd.push_char(c);
                            }
                        }
                        self.request_frame();
                    }
                }
                c if c >= 32 => {
                    if let Some(ch) = char::from_u32(c) {
                        self.cmd.push_char(ch);
                        self.request_frame();
                    }
                }
                _ => {}
            }
            return;
        }

        if self.expanded && (ch == '/' as u32 || ch == 11) && self.editor.is_none() {
            self.dispatch(Action::OpenCommandPalette);
            return;
        }

        let Some(e) = &mut self.editor else { return };
        if !e.focused {
            return;
        }
        match ch {
            8 => {
                if unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0 {
                    let t = e.title.trim_end();
                    let cut = t.rfind(' ').map(|i| i + 1).unwrap_or(0);
                    e.title.truncate(cut);
                } else {
                    e.title.pop();
                }
            }
            13 => {
                self.dispatch(Action::TodoSave);
                return;
            }
            27 => {
                self.dispatch(Action::TodoCancel);
                return;
            }
            22 => {
                if let Some(t) = read_clip_text(self.hwnd) {
                    e.title.push_str(t.lines().next().unwrap_or(""));
                }
            }
            c if c >= 32 => {
                if let Some(c) = char::from_u32(c) {
                    if e.title.chars().count() < 120 {
                        e.title.push(c);
                    }
                }
            }
            _ => {}
        }
        self.request_frame();
    }

    fn on_command(&mut self, id: usize) {
        match id {
            tray::ID_SETTINGS => self.open_settings(),
            tray::ID_STARTUP => settings::set_startup(!settings::startup_enabled()),
            tray::ID_MINIMAL => self.toggle_setting("minimal"),
            tray::ID_ABOUT => {
                self.open_settings();
                if let Some(s) = &mut self.settings_win {
                    s.section = crate::settings_window::SECTIONS.len() - 1;
                    s.invalidate();
                }
            }
            tray::ID_QUIT => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
            x if (tray::ID_SIZE_BASE..tray::ID_SIZE_BASE + tray::SIZES.len()).contains(&x) => {
                self.set_num("size", tray::SIZES[x - tray::ID_SIZE_BASE]);
            }
            x if (tray::ID_DISPLAY_BASE..tray::ID_DISPLAY_BASE + 16).contains(&x) => {
                self.set_num("display", (x - tray::ID_DISPLAY_BASE) as u32);
            }
            _ => {}
        }
    }

    // ------------------------------------------------ settings

    pub fn open_settings(&mut self) {
        match &self.settings_win {
            Some(s) if !s.hwnd.0.is_null() => s.focus(),
            _ => self.settings_win = SettingsWindow::open(&self.gfx),
        }    }

    pub fn settings_changed(&mut self) {
        self.settings.save();
        if let Some(s) = &self.settings_win {
            s.invalidate();
        }
        self.request_frame();
    }

    pub fn toggle_setting(&mut self, key: &'static str) {
        if let Some(b) = self.settings.bool_mut(key) {
            *b = !*b;
        }
        match key {
            "notifications" => notifications::set_enabled(self.settings.notifications),
            "clip_persist" => self.clip.set_persist(self.settings.clip_persist),
            "calendar_week" => self.cal_week = self.settings.calendar_week,
            "week_monday" => self.cal_reset(),
            "lyrics" => {
                if !self.settings.lyrics {
                    self.show_split = false;
                }
                self.lyrics_key.clear();
                self.maybe_fetch_lyrics();
            }
            "lyrics_pill" => self.maybe_fetch_lyrics(),
            "hide_fullscreen" => self.check_fullscreen(),
            "autohide_window" => self.check_desktop_state(),
            "exclude_capture" => {
                unsafe {
                    let _ = SetWindowDisplayAffinity(self.hwnd, WINDOW_DISPLAY_AFFINITY(if self.settings.exclude_capture { 17 } else { 0 }));
                }
            }
            _ => {}
        }
        self.settings_changed();
    }

    pub fn set_num(&mut self, key: &'static str, v: u32) {
        if let Some(n) = self.settings.num_mut(key) {
            *n = v;
        }
        match key {
            "size" | "display" => self.place_window(),
            "focus" | "short_break" | "long_break" => self.pomo.settings_changed(&self.settings),
            _ => {}
        }
        self.settings_changed();
    }

    // ------------------------------------------------ actions

    pub fn dispatch(&mut self, a: Action) {
        match a {
            Action::None => {}
            Action::Page(i) => {
                if let Some(p) = self.pages().get(i) {
                    self.page = *p;
                    if self.page == Page::Calc {
                        self.set_activatable(true);
                    } else if self.editor.is_none() {
                        self.set_activatable(false);
                    }
                }
            }
            Action::PlayPause => {
                self.media_ctl.send(MediaCmd::PlayPause);
                self.media.pos = self.media.position();
                self.media.stamp = time::utc_ticks();
                self.media.playing = !self.media.playing;
            }
            Action::Next => self.media_ctl.send(MediaCmd::Next),
            Action::Prev => self.media_ctl.send(MediaCmd::Prev),
            Action::SeekBar => {}
            Action::ToggleLyrics => {
                self.show_split = !self.show_split;
                if self.show_split {
                    self.maybe_fetch_lyrics();
                }
                self.schedule_lyric_timer();
            }
            Action::LyricSeek(t) => {
                if self.media.can_seek {
                    self.media_ctl.send(MediaCmd::Seek(t));
                    self.media.pos = t;
                    self.media.stamp = time::utc_ticks();
                    self.schedule_lyric_timer();
                }
            }
            Action::PomoStartPause => self.pomo.start_pause(),
            Action::PomoReset => self.pomo.reset(&self.settings),
            Action::PomoSkip => self.pomo.skip(&self.settings),
            Action::PomoMode(m) => {
                let m = [Mode::Focus, Mode::Short, Mode::Long][m as usize % 3];
                self.pomo.set_mode(m, &self.settings);
            }
            Action::WaterDrank => self.water.drank(),
            Action::WaterRefilled => self.water.refilled(),
            Action::WaterUndo => self.water.undo(),
            Action::TodoToggle(id) => self.todo.toggle(id),
            Action::TodoDelete(id) => self.todo.delete(id),
            Action::TodoEdit(id) => {
                if let Some(t) = self.todo.get(id) {
                    self.editor = Some(Editor { id: Some(id), title: t.title.clone(), day: t.day, start: t.start, dur: t.dur, focused: true });
                    self.set_activatable(true);
                }
            }
            Action::TodoAdd => {
                let now = time::now();
                let day = if self.page == Page::Calendar { self.cal_sel } else { now.day_num() };
                let start = if day == now.day_num() { ((now.minute_of_day() + 14) / 15 * 15).min(23 * 60 + 45) } else { 9 * 60 };
                self.editor = Some(Editor { id: None, title: String::new(), day, start, dur: 30, focused: true });
                self.page = Page::Todo;
                self.set_activatable(true);
            }
            Action::TodoSave => {
                if let Some(e) = self.editor.take() {
                    let title = e.title.trim().to_string();
                    if !title.is_empty() {
                        self.todo.upsert(e.id, title, e.day, e.start, e.dur);
                    }
                }
                self.set_activatable(false);
            }
            Action::TodoCancel => {
                self.editor = None;
                self.set_activatable(false);
            }
            Action::TodoField => {
                if let Some(e) = &mut self.editor {
                    e.focused = true;
                }
                self.set_activatable(true);
            }
            Action::TodoStart(d) => {
                if let Some(e) = &mut self.editor {
                    e.start = (e.start as i32 + d).rem_euclid(24 * 60) as u32;
                }
            }
            Action::TodoDur(d) => {
                if let Some(e) = &mut self.editor {
                    e.dur = (e.dur as i32 + d).clamp(0, 12 * 60) as u32;
                }
            }
            Action::TodoDay(d) => {
                if let Some(e) = &mut self.editor {
                    e.day = (e.day + d as i64).max(time::today());
                }
            }
            Action::CalShift(d) => self.cal_shift(d),
            Action::CalView(week) => {
                self.cal_week = week;
                self.cal_reset_to(self.cal_sel);
            }
            Action::CalDay(d) => self.cal_sel = d,
            Action::ShelfItem(i) => {
                if let Some(it) = self.shelf.items.get(i) {
                    if it.kind == shelf::Kind::Text {
                        let t = it.content.clone();
                        copy_text(self.hwnd, &t);
                        self.show_toast(ToastKind::Info, ico::COPY, pal::GREEN, "Copied", "", None, 1.2);
                    } else {
                        self.shelf.open(i);
                    }
                }
            }
            Action::ShelfRemove(i) => self.shelf.remove(i),
            Action::ShelfClear => self.shelf.clear(),
            Action::ShelfCopyPath(i) => {
                let info = self.shelf.items.get(i).map(|it| (it.path.display().to_string(), it.label.clone()));
                if let Some((path_str, label)) = info {
                    crate::clipboard::set_text(&path_str);
                    self.clip.record_text(&path_str);
                    self.show_toast(ToastKind::Notification, ico::LINK, pal::ACCENT, "Path Copied", &label, None, 1.8);
                }
            }
            Action::ShelfConvert(i) => {
                if let Some(out_p) = self.shelf.convert_image(i) {
                    let name = out_p.file_name().unwrap_or_default().to_string_lossy();
                    self.show_toast(ToastKind::Notification, ico::CONVERT, pal::GREEN, "Image Converted", &name, None, 2.5);
                }
            }
            Action::ShelfZip(i) => {
                if let Some(out_p) = self.shelf.zip_item(i) {
                    let name = out_p.file_name().unwrap_or_default().to_string_lossy();
                    self.show_toast(ToastKind::Notification, ico::ZIP, pal::GREEN, "Created ZIP Archive", &name, None, 2.5);
                }
            }
            Action::ShelfExplore(i) => {
                if let Some(item) = self.shelf.items.get(i) {
                    let _ = std::process::Command::new("explorer.exe")
                        .arg(format!("/select,\"{}\"", item.path.display()))
                        .spawn();
                }
            }
            Action::ToggleMicMute => {
                if let Some(muted) = system::toggle_mic_mute() {
                    self.mic_muted = muted;
                    if muted {
                        self.show_toast(ToastKind::Notification, ico::MIC_OFF, pal::RED, "Microphone", "Muted", None, 1.8);
                    } else {
                        self.show_toast(ToastKind::Notification, ico::MIC, pal::GREEN, "Microphone", "Unmuted", None, 1.8);
                    }
                }
            }
            Action::CycleAudioDevice => {
                if let Some(name) = system::cycle_audio_output() {
                    self.show_toast(ToastKind::Notification, ico::SPEAKER, pal::ACCENT, "Audio Output", &name, None, 2.2);
                }
            }
            Action::PickColor => {
                self.picking_color = true;
                system::start_color_picker(self.hwnd);
                self.show_toast(ToastKind::Notification, ico::EYEDROPPER, pal::ACCENT, "Color Picker", "Click any pixel on screen (Right-click to cancel)", None, 4.0);
            }
            Action::OpenCommandPalette => {
                self.page = Page::Calc;
                self.cmd.active = true;
                self.set_activatable(true);
                self.expand(true);
            }
            Action::CalcInput(s) => {
                for c in s.chars() {
                    self.cmd.push_char(c);
                }
                self.set_activatable(true);
                self.request_frame();
            }
            Action::CalcClear => {
                self.cmd.clear();
                self.request_frame();
            }
            Action::CalcBackspace => {
                self.cmd.backspace();
                self.request_frame();
            }
            Action::CommandConfirm(cmd) => {
                self.cmd.confirm_cmd = Some(cmd);
                self.cmd.update();
                self.request_frame();
            }
            Action::CommandRun => {
                if let Some(item) = self.cmd.result.clone() {
                    match item.kind {
                        crate::cmd::CommandKind::Copy(s) => {
                            crate::clipboard::set_text(&s);
                            self.clip.record_text(&s);
                            self.show_toast(ToastKind::Notification, ico::CHECK, pal::GREEN, "Copied to Clipboard", &s, None, 2.0);
                        }
                        crate::cmd::CommandKind::Launch(cmd) => {
                            let _ = std::process::Command::new("cmd").args(["/c", "start", "", &cmd]).spawn();
                        }
                        crate::cmd::CommandKind::Page(idx) => {
                            let pages = self.pages();
                            if let Some(p) = pages.get(idx) {
                                self.page = *p;
                            }
                        }
                        crate::cmd::CommandKind::Action(act) => {
                            self.dispatch(act);
                        }
                    }
                }
            }
            Action::ClipCopy(i) => {
                self.clip.copy(i, self.hwnd);
                self.show_toast(ToastKind::Info, ico::COPY, pal::GREEN, "Copied", "", None, 1.0);
            }
            Action::ClipRemove(i) => self.clip.remove(i),
            Action::ClipPin(i) => self.clip.toggle_pin(i),
            Action::ClipClear => self.clip.clear(),
            Action::NotifRemove(i) => {
                if i < self.notifs.len() {
                    let n = self.notifs.remove(i);
                    self.dismissed_notifs.insert(n.id);
                }
            }
            Action::NotifClear => {
                self.dismissed_notifs.extend(self.notifs.iter().map(|n| n.id));
                self.notifs.clear();
            }
            Action::AlertButton(i) => {
                if let Some(al) = self.alerts.pop_front() {
                    if let Some((_, act)) = al.buttons.get(i) {
                        self.alert_act(act.clone());
                    }
                }
            }
            Action::OpenSettings => self.open_settings(),
            Action::OpenUrl(u) => unsafe {
                ShellExecuteW(None, w!("open"), &HSTRING::from(u), None, None, SW_SHOWNORMAL);
            },
            other => {
                // Settings-window actions routed through the app.
                crate::settings_window::apply(self, other);
            }
        }
        self.request_frame();
    }

    fn alert_act(&mut self, a: AlertAct) {
        match a {
            AlertAct::Dismiss => {}
            AlertAct::PomoStart => {
                if !self.pomo.running {
                    self.pomo.start_pause();
                }
            }
            AlertAct::WaterDrank => self.water.drank(),
            AlertAct::WaterRefilled => self.water.refilled(),
            AlertAct::WaterSnooze => self.water.snooze(self.settings.snooze.max(10)),
            AlertAct::TaskAck(_) => {}
            AlertAct::TaskLater(id, ev) => self.todo.snooze(id, self.settings.snooze, ev),
            AlertAct::TaskDone(id) => {
                if let Some(t) = self.todo.get_mut(id) {
                    t.done = true;
                }
                self.todo.save();
            }
        }
    }

    // ------------------------------------------------ alerts & toasts

    pub fn alert(&mut self, icon: char, color: Color, title: &str, sub: &str, buttons: Vec<(&str, AlertAct)>, chime: Option<Chime>) {
        if let (Some(c), true) = (chime, self.settings.sound) {
            sound::play(c);
        }
        self.alerts.push_back(Alert {
            icon,
            color,
            title: title.into(),
            sub: sub.into(),
            buttons: buttons.into_iter().map(|(l, a)| (l.to_string(), a)).collect(),
            created: Instant::now(),
        });
        self.request_frame();
    }

    pub fn show_toast(&mut self, kind: ToastKind, icon: char, color: Color, title: &str, sub: &str, level: Option<f32>, secs: f32) {
        if self.hidden {
            return;
        }
        self.toast = Some(Toast { kind, icon, color, title: title.into(), sub: sub.into(), level, until: Instant::now() + Duration::from_secs_f32(secs) });
        // The 1s tick clears it; use a finer timer for short toasts.
        unsafe {
            SetTimer(self.hwnd, T_TICK, 250, None);
        }
        self.request_frame();
    }

    // ------------------------------------------------ periodic work

    fn tick(&mut self) {
        self.tick_n += 1;
        let now = time::now();
        let mut dirty = false;

        if let Some(t) = &self.toast {
            if Instant::now() >= t.until {
                self.toast = None;
                dirty = true;
            }
        }
        if self.toast.is_none() {
            unsafe {
                SetTimer(self.hwnd, T_TICK, 1000, None);
            }
        } else if self.tick_n % 4 != 0 {
            // Fast ticks while a toast is up only check expiry.
            if dirty {
                self.request_frame();
            }
            return;
        }

        // Periodic desktop detection check
        self.check_desktop_state();

        // Stale alerts fall away after two minutes.
        if let Some(a) = self.alerts.front() {
            if a.created.elapsed() > Duration::from_secs(120) {
                self.alerts.pop_front();
                dirty = true;
            }
        }

        if now.minute != self.last_minute {
            self.last_minute = now.minute;
            dirty = true;
        }

        if self.settings.mic_privacy {
            let active = system::mic_in_use();
            let muted = system::mic_muted().unwrap_or(false);
            if active != self.mic_active || muted != self.mic_muted {
                self.mic_active = active;
                self.mic_muted = muted;
                dirty = true;
            }
        }

        if self.settings.downloads {
            let dl = system::check_active_downloads();
            if dl.is_some() != self.download.is_some() {
                self.download = dl;
                dirty = true;
            }
        }

        if let Some(done) = self.pomo.tick(&self.settings) {
            let (title, sub) = match (done, self.pomo.mode) {
                (Mode::Focus, Mode::Long) => ("Long Break!", format!("{} min, you earned it", self.settings.long_break)),
                (Mode::Focus, _) => ("Break Time!", format!("{} minute short break", self.settings.short_break)),
                _ => ("Focus Time!", format!("Back to work, {} min", self.settings.focus)),
            };
            self.alert(ico::TIMER, pal::ORANGE, title, &sub, vec![("Start", AlertAct::PomoStart), ("Later", AlertAct::Dismiss)], Some(Chime::Focus));
        }
        if self.pomo.running {
            dirty = true;
        }

        if let Some(w) = self.water.tick(&now, self.settings.water) {
            match w {
                WaterAlert::Drink => {
                    let sub = format!("Stay hydrated! {} ml", self.water.per());
                    self.alert(ico::DROP, pal::WATER, "Time to Drink", &sub, vec![("Drank", AlertAct::WaterDrank), ("Snooze", AlertAct::WaterSnooze)], Some(Chime::Water));
                }
                WaterAlert::Refill => {
                    self.alert(ico::DROP, pal::WATER, "Time to Refill", "Your bottle is probably empty", vec![("Refilled", AlertAct::WaterRefilled), ("Snooze", AlertAct::WaterSnooze)], Some(Chime::Water));
                }
            }
        }

        for (id, ev) in self.todo.tick(&now, self.settings.todo_start, self.settings.todo_end) {
            let Some(t) = self.todo.get(id).cloned() else { continue };
            match ev {
                TaskEvent::Start => {
                    let sub = if t.dur > 0 { format!("{} · {} min", t.title, t.dur) } else { t.title.clone() };
                    self.alert(ico::CHECKLIST, pal::ACCENT, "Task Time!", &sub, vec![("Start", AlertAct::TaskAck(id)), ("Later", AlertAct::TaskLater(id, ev))], Some(Chime::Task));
                }
                TaskEvent::End => {
                    self.alert(ico::CLOCK, pal::PINK, "Time's Up!", &t.title, vec![("Done", AlertAct::TaskDone(id)), ("Later", AlertAct::TaskLater(id, ev))], Some(Chime::Task));
                }
            }
        }

        if self.expanded {
            match self.page {
                Page::Home if self.media.playing => dirty = true,
                Page::Stats => {
                    self.stats.sample();
                    dirty = true;
                }
                _ => {}
            }
        }

        if self.tick_n % 2 == 0 {
            self.check_fullscreen();
            self.check_desktop_state();
        }
        if self.tick_n == 8 {
            // Startup work (fonts, GPU device, WinRT) is done; hand unused pages back to Windows.
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            }
        }
        if self.tick_n % 10 == 0 {
            // Stay above other topmost windows (e.g. after a full-screen app exits).
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        if dirty {
            self.request_frame();
        }
    }

    fn check_fullscreen(&mut self) {
        let hide = self.settings.hide_fullscreen && system::fullscreen_on(&self.monitor, self.hwnd);
        if hide != self.hidden {
            self.hidden = hide;
            unsafe {
                let _ = ShowWindow(self.hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
            }
            if hide {
                self.expanded = false;
                self.toast = None;
            }
            self.request_frame();
        }
    }

    pub fn is_notch_visible(&self) -> bool {
        if !self.settings.autohide_window {
            return true;
        }
        if self.is_desktop {
            return true;
        }
        if self.window_hovered {
            return true;
        }
        if self.expanded {
            return true;
        }
        if !self.alerts.is_empty() {
            return true;
        }
        if self.toast.is_some() {
            return true;
        }
        if self.preview {
            return true;
        }
        if self.drag_over {
            return true;
        }
        if let Some(e) = &self.editor {
            if e.focused {
                return true;
            }
        }
        false
    }

    fn check_desktop_state(&mut self) {
        let fg = unsafe { GetForegroundWindow() };
        let is_nuva_win = fg == self.hwnd || self.settings_win.as_ref().map(|s| s.hwnd == fg).unwrap_or(false);
        if is_nuva_win {
            return;
        }
        let was_desktop = self.is_desktop;
        self.is_desktop = system::is_desktop_homescreen(&self.monitor, self.hwnd);
        if self.is_desktop != was_desktop {
            if self.is_desktop {
                self.close_deadline = None;
            } else if !self.cursor_in_notch() {
                if self.close_deadline.is_none() {
                    self.close_deadline = Some(Instant::now() + Duration::from_millis(280));
                }
            }
            self.request_frame();
        }
    }

    fn check_top_hover(&mut self) {
        if self.is_desktop || !self.settings.autohide_window || self.hidden {
            return;
        }
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let mon = self.monitor;
        if p.x < mon.left || p.x > mon.right {
            return;
        }
        let cx = mon.left + (mon.right - mon.left) / 2;
        let trigger_w = (self.sw.v.max(260.0) * self.scale) as i32;
        let in_x = p.x >= cx - trigger_w / 2 && p.x <= cx + trigger_w / 2;
        let in_y = p.y >= mon.top && p.y <= mon.top + 4;

        if in_x && in_y {
            self.close_deadline = None;
            if !self.window_hovered {
                self.window_hovered = true;
                self.request_frame();
            }
        }
    }

    fn on_edge_poll(&mut self) {
        self.check_top_hover();

        let editing = self.editor.as_ref().map(|e| e.focused).unwrap_or(false);
        let busy = editing || self.seeking.is_some() || self.drag_over || !self.alerts.is_empty() || self.toast.is_some() || self.preview;

        let should_auto_close = if self.is_desktop {
            self.expanded
        } else {
            self.window_hovered || self.expanded
        };

        if should_auto_close && !busy {
            if self.cursor_in_notch() {
                self.close_deadline = None;
            } else {
                match self.close_deadline {
                    None => {
                        self.close_deadline = Some(Instant::now() + Duration::from_millis(280));
                    }
                    Some(deadline) => {
                        if Instant::now() >= deadline {
                            self.close_deadline = None;
                            self.expand(false);
                            if !self.is_desktop {
                                self.window_hovered = false;
                            }
                            self.request_frame();
                        }
                    }
                }
            }
        } else if self.cursor_in_notch() || busy {
            self.close_deadline = None;
        }

        let fg = unsafe { GetForegroundWindow() };
        if fg.0 as isize != self.last_fg {
            self.last_fg = fg.0 as isize;
            self.check_desktop_state();
        }
    }

    fn check_language(&mut self) {
        let fg = unsafe { GetForegroundWindow() }.0 as isize;
        let (id, short, long) = system::input_language();
        if fg != self.lang_fg {
            // Switching windows can change layout without the user asking; just remember it.
            self.lang_fg = fg;
            self.lang = id;
            return;
        }
        if id != self.lang {
            self.lang = id;
            if self.settings.hud_language {
                self.show_toast(ToastKind::Language, ico::KEYBOARD, pal::ACCENT, &short, &long, None, 1.6);
            }
        }
    }

    fn on_power_setting(&mut self, g: GUID, value: u32) {
        let settled = self.started.elapsed() > Duration::from_secs(3);
        if g == GUID_MONITOR_BRIGHTNESS || g == GUID_POLICY_BRIGHTNESS {
            if settled && self.settings.hud_brightness && value <= 100 {
                self.show_toast(ToastKind::Brightness, ico::BRIGHT, pal::YELLOW, "Brightness", &value.to_string(), Some(value as f32 / 100.0), 1.6);
            }
            return;
        }
        let b = system::battery();
        let old = self.battery;
        self.battery = b;
        if !settled || !b.present || !self.settings.popup_battery {
            return;
        }
        let glyph = crate::gfx::battery_glyph(b.pct, b.ac);
        if b.ac != old.ac {
            if b.ac {
                self.show_toast(ToastKind::Battery, glyph, pal::GREEN, "Charging", &format!("{}%", b.pct), Some(b.pct as f32 / 100.0), 2.5);
            } else {
                self.show_toast(ToastKind::Battery, glyph, pal::DIM, "Charger unplugged", &format!("{}% remaining", b.pct), Some(b.pct as f32 / 100.0), 2.5);
            }
        } else if b.ac && b.pct >= 100 && old.pct < 100 {
            self.show_toast(ToastKind::Battery, glyph, pal::GREEN, "Fully charged", "You can unplug", None, 3.0);
        } else if !b.ac && b.pct <= 10 && old.pct > 10 {
            self.alert(glyph, pal::RED, "Battery critical", &format!("Plug in now · {}%", b.pct), vec![("OK", AlertAct::Dismiss)], None);
        } else if !b.ac && b.pct <= 20 && old.pct > 20 {
            self.show_toast(ToastKind::Battery, glyph, pal::ORANGE, "Low battery", &format!("Plug in soon · {}%", b.pct), Some(b.pct as f32 / 100.0), 3.5);
        }
    }

    fn show_network(&mut self, s: NetState) {
        let old = std::mem::replace(&mut self.net, s.clone());
        if !self.settings.popup_wifi || old == s {
            return;
        }
        match s.level {
            2 => {
                let title = if s.wifi { "Connected" } else { "Online" };
                self.show_toast(ToastKind::Network, ico::WIFI, pal::GREEN, title, &s.name, None, 2.2);
            }
            1 => self.show_toast(ToastKind::Network, ico::WARN, pal::ORANGE, "No internet access", &s.name, None, 2.6),
            _ => self.show_toast(ToastKind::Network, ico::WIFI, pal::RED, "Disconnected", "", None, 2.6),
        }
    }

    // ------------------------------------------------ worker events

    fn on_event(&mut self, e: Event) {
        match e {
            Event::Media(m) => {
                let changed_track = m.key() != self.media.key();
                self.media = m;
                if changed_track {
                    if self.art_key != self.media.key() {
                        self.art_src = None;
                        self.media_accent = None;
                        self.bmp_cache.remove("art");
                    }
                    self.lyrics = None;
                    self.lyric_scroll.snap(0.0);
                    self.maybe_fetch_lyrics();
                }
                self.schedule_lyric_timer();
                self.request_frame();
            }
            Event::MediaArt(key, bytes) => {
                if key == self.media.key() {
                    self.art_src = self.gfx.decode(&bytes, 256);
                    if let Some((_, _, px)) = self.gfx.decode_bgra_pixels(&bytes, 64) {
                        if let Some((r, g, b)) = crate::media::extract_vibrant_accent(&px) {
                            self.media_accent = Some(Color::rgb(r, g, b));
                        }
                    }
                    self.art_key = key;
                    self.bmp_cache.remove("art");
                    self.request_frame();
                }
            }
            Event::Lyrics(gen, l) => {
                if gen == self.lyrics_gen {
                    self.lyrics_loading = false;
                    self.lyrics = l;
                    self.schedule_lyric_timer();
                    self.request_frame();
                }
            }
            Event::Network(s) => {
                self.pending_net = Some(s);
                unsafe {
                    SetTimer(self.hwnd, T_NET, 1500, None);
                }
            }
            Event::Bluetooth(name, connected) => {
                if self.settings.popup_bluetooth && self.started.elapsed() > Duration::from_secs(3) {
                    if connected {
                        self.show_toast(ToastKind::Bluetooth, ico::HEADPHONE, pal::ACCENT, "Connected", &name, None, 2.5);
                    } else {
                        self.show_toast(ToastKind::Bluetooth, ico::BLUETOOTH, pal::DIM, "Disconnected", &name, None, 2.0);
                    }
                }
            }
            Event::Notifications(list) => {
                let ids: HashSet<u32> = list.iter().map(|n| n.id).collect();
                if let Some(seen) = &self.seen_notifs {
                    if self.settings.notif_popup {
                        if let Some(n) = list.iter().find(|n| !seen.contains(&n.id)) {
                            let title = if n.title.is_empty() { n.app.clone() } else { n.title.clone() };
                            let sub = if n.title.is_empty() { n.body.clone() } else { n.app.clone() };
                            let icon = if n.app.to_lowercase().contains("mail") || n.app.to_lowercase().contains("outlook") { ico::MAIL } else { ico::BELL };
                            self.show_toast(ToastKind::Notification, icon, pal::ACCENT, &title, &sub, None, 3.5);
                        }
                    }
                }
                let mut seen = self.seen_notifs.take().unwrap_or_default();
                seen.extend(ids.iter().copied());
                self.seen_notifs = Some(seen);
                // Locally dismissed notifications stay hidden; forget ids Windows no longer has.
                self.dismissed_notifs.retain(|id| ids.contains(id));
                self.notifs = list.into_iter().filter(|n| !self.dismissed_notifs.contains(&n.id)).collect();
                if self.page == Page::Notifs {
                    self.request_frame();
                }
            }
            Event::NotifIcon(app, bytes) => {
                if let Some(src) = self.gfx.decode(&bytes, 64) {
                    self.notif_srcs.insert(app, src);
                }
            }
            Event::LiveActivity(act) => {
                if let Some(a) = &act {
                    if a.progress.is_none() {
                        self.show_toast(ToastKind::Notification, ico::INFO, pal::ACCENT, &a.title, &a.sub, None, 3.5);
                    }
                }
                self.live_activity = act;
                self.request_frame();
            }
            Event::AudioSpectrum(bands) => {
                self.audio_spectrum = bands;
                if self.media.playing {
                    self.request_frame();
                }
            }
            Event::NotificationToast { title, sub } => {
                self.show_toast(ToastKind::Notification, ico::INFO, pal::ACCENT, &title, &sub, None, 3.5);
            }
            Event::ShelfImageConverted { old_path, new_path } => {
                self.shelf.replace_or_add(old_path, new_path.clone());
                let name = new_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
                self.show_toast(ToastKind::Info, ico::CONVERT, pal::GREEN, "Image Converted", &name, None, 3.0);
            }
            Event::ShelfOcrSuccess { path: _, text } => {
                crate::app::copy_text(self.hwnd, &text);
                let preview = if text.len() > 30 {
                    format!("{}…", &text[..30])
                } else {
                    text
                };
                self.show_toast(ToastKind::Info, ico::SEARCH, pal::ACCENT, "Text Copied via OCR", &preview, None, 3.0);
            }
            Event::ShelfZipCreated { path } => {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "archive.zip".into());
                self.shelf.add_file(path);
                self.show_toast(ToastKind::Info, ico::ZIP, pal::ACCENT, "ZIP Archive Created", &name, None, 3.0);
            }
            Event::ShelfActionFailed(reason) => {
                self.show_toast(ToastKind::Info, ico::CLOSE, pal::RED, "Shelf Action Failed", &reason, None, 3.0);
            }
            Event::ColorHover(_) | Event::ColorPicked(_, _, _) | Event::ColorCanceled => {}
        }
    }

    fn maybe_fetch_lyrics(&mut self) {
        if !(self.settings.lyrics || self.settings.lyrics_pill) || !self.media.active || self.media.title.is_empty() {
            return;
        }
        let key = self.media.key();
        if key == self.lyrics_key {
            return;
        }
        self.lyrics_key = key;
        self.lyrics_gen += 1;
        self.lyrics_loading = true;
        self.lyrics = None;
        let m = &self.media;
        lyrics::request(self.lyrics_gen, m.title.clone(), m.artist.clone(), m.album.clone(), m.dur);
    }

    /// Wake exactly when the next lyric line starts (only while lyrics are on screen).
    pub fn schedule_lyric_timer(&self) {
        unsafe {
            let _ = KillTimer(self.hwnd, T_LYRIC);
        }
        let visible = (self.show_split && self.last_view == View::Open(Page::Home)) || (self.settings.lyrics_pill && self.last_view == View::Pill);
        if !visible || !self.media.playing {
            return;
        }
        if let Some(l) = &self.lyrics {
            let pos = self.media.position();
            if let Some(t) = l.next_time(pos) {
                let ms = (((t - 0.25) - pos) * 1000.0).clamp(30.0, 60_000.0) as u32;
                unsafe {
                    SetTimer(self.hwnd, T_LYRIC, ms, None);
                }
            }
        }
    }

    // ------------------------------------------------ drag & drop

    pub fn drag_enter(&mut self) {
        self.drag_over = true;
        self.drop_action_zone = Some(crate::shelf::DropAction::Hold);
        self.page = Page::Shelf;
        self.expanded = true;
        self.request_frame();
    }

    pub fn drag_over_pt(&mut self, screen_x: i32, screen_y: i32) {
        let scale = self.scale.max(1.0);
        let x = (screen_x - self.win_px.0) as f32 / scale;
        let y = (screen_y - self.win_px.1) as f32 / scale;

        let a = self.notch_rect();
        let r = Rect::new(a.x + 20.0, 36.0, (a.w - 40.0).max(10.0), (a.h - 64.0).max(10.0));
        
        let mut chosen_col = None;
        for i in 0..4 {
            let card = Self::dropzone_rect(r, i);
            if card.contains((x, y)) {
                chosen_col = Some(i);
                break;
            }
        }
        let mut col = chosen_col.unwrap_or_else(|| {
            let rel = (x - r.x) / r.w.max(1.0);
            ((rel * 4.0).floor() as i32).clamp(0, 3) as usize
        });

        if !self.drag_has_images && (col == 1 || col == 3) {
            col = 0;
        }

        let action = match col {
            0 => crate::shelf::DropAction::Hold,
            1 => crate::shelf::DropAction::Convert,
            2 => crate::shelf::DropAction::Compress,
            _ => crate::shelf::DropAction::Ocr,
        };
        if self.drop_action_zone != Some(action) {
            self.drop_action_zone = Some(action);
            self.request_frame();
        }
    }

    pub fn drag_leave(&mut self) {
        self.drag_over = false;
        self.drop_action_zone = None;
        self.drag_has_images = false;
        unsafe {
            SetTimer(self.hwnd, T_CLOSE, 400, None);
        }
        self.request_frame();
    }

    pub fn dropped(&mut self, d: Option<Dropped>) {
        self.drag_over = false;
        let action = self.drop_action_zone.take().unwrap_or(crate::shelf::DropAction::Hold);
        match d {
            Some(Dropped::Files(files)) => {
                if files.is_empty() {
                    return;
                }
                match action {
                    crate::shelf::DropAction::Hold => {
                        for p in files {
                            self.shelf.add_file(p);
                        }
                    }
                    crate::shelf::DropAction::Convert => {
                        for p in files {
                            self.shelf.add_file(p.clone());
                            crate::shelf::convert_image_async(p);
                        }
                    }
                    crate::shelf::DropAction::Compress => {
                        crate::shelf::compress_files_async(files);
                    }
                    crate::shelf::DropAction::Ocr => {
                        for p in files {
                            crate::shelf::extract_text_from_image_async(p);
                        }
                    }
                }
            }
            Some(Dropped::Text(t)) => self.shelf.add_text(&t),
            None => {}
        }
        self.request_frame();
    }

    // ------------------------------------------------ calendar

    pub fn week_start(&self, day: i64) -> i64 {
        let wd = time::weekday(day) as i64;
        let off = if self.settings.week_monday { (wd + 6) % 7 } else { wd };
        day - off
    }

    fn cal_reset(&mut self) {
        self.cal_reset_to(time::today());
    }

    fn cal_reset_to(&mut self, day: i64) {
        self.cal_anchor = if self.cal_week {
            self.week_start(day)
        } else {
            let (y, m, _) = time::civil_from_days(day);
            time::days_from_civil(y, m, 1)
        };
    }

    fn cal_shift(&mut self, d: i32) {
        if self.cal_week {
            self.cal_anchor += 7 * d as i64;
        } else {
            let (y, m, _) = time::civil_from_days(self.cal_anchor);
            let idx = y * 12 + (m as i32 - 1) + d;
            self.cal_anchor = time::days_from_civil(idx.div_euclid(12), (idx.rem_euclid(12) + 1) as u32, 1);
        }
    }

    // ------------------------------------------------ bitmaps

    pub fn art(&mut self, rt: &ID2D1RenderTarget) -> Option<ID2D1Bitmap> {
        if let Some(b) = self.bmp_cache.get("art") {
            return Some(b.clone());
        }
        let b = crate::gfx::bitmap_from_wic(rt, self.art_src.as_ref()?)?;
        self.bmp_cache.insert("art".into(), b.clone());
        Some(b)
    }

    pub fn shelf_icon(&mut self, rt: &ID2D1RenderTarget, path: &std::path::Path) -> Option<ID2D1Bitmap> {
        let key = format!("shelf:{}", path.display());
        if let Some(b) = self.bmp_cache.get(&key) {
            return Some(b.clone());
        }
        if !self.shelf_px.contains_key(&key) {
            let px = shelf::icon_pixels(path, (48.0 * self.scale) as i32)?;
            self.shelf_px.insert(key.clone(), px);
        }
        let (w, h, px) = self.shelf_px.get(&key)?;
        let b = crate::gfx::bitmap_from_bgra(rt, *w, *h, px)?;
        self.bmp_cache.insert(key, b.clone());
        Some(b)
    }

    pub fn clip_thumb(&mut self, rt: &ID2D1RenderTarget, i: usize) -> Option<ID2D1Bitmap> {
        let it = self.clip.items.get(i)?;
        let key = format!("clip:{}", it.id);
        if let Some(b) = self.bmp_cache.get(&key) {
            return Some(b.clone());
        }
        if let crate::clipboard::ClipData::Image { w, h, thumb, .. } = &it.data {
            let b = crate::gfx::bitmap_from_bgra(rt, *w, *h, thumb)?;
            self.bmp_cache.insert(key, b.clone());
            return Some(b);
        }
        None
    }

    pub fn notif_icon(&mut self, rt: &ID2D1RenderTarget, app: &str) -> Option<ID2D1Bitmap> {
        let key = format!("notif:{app}");
        if let Some(b) = self.bmp_cache.get(&key) {
            return Some(b.clone());
        }
        let b = crate::gfx::bitmap_from_wic(rt, self.notif_srcs.get(app)?)?;
        self.bmp_cache.insert(key, b.clone());
        Some(b)
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.settings.save();
        self.water.save();
        self.todo.save();
    }
}

pub fn copy_text(hwnd: HWND, s: &str) {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        if OpenClipboard(hwnd).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        let u: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, u.len() * 2) {
            let p = GlobalLock(g) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(u.as_ptr(), p, u.len());
                let _ = GlobalUnlock(g);
                let _ = SetClipboardData(13, HANDLE(g.0));
            }
        }
        let _ = CloseClipboard();
    }
}

fn read_clip_text(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        OpenClipboard(hwnd).ok()?;
        let out = (|| {
            let h = GetClipboardData(13).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let max = GlobalSize(g) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        out
    }
}
