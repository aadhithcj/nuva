//! User settings, stored as key=value in %LOCALAPPDATA%\Nuva\settings.txt.

use crate::store;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

#[derive(Clone, Debug)]
pub struct Settings {
    /// 0 = auto, else percent (100/115/130/150).
    pub size: u32,
    /// 0 = main display, else 1-based monitor index.
    pub display: u32,
    pub pet: bool,
    pub minimal: bool,
    pub sound: bool,
    pub lyrics: bool,
    pub todo_start: bool,
    pub todo_end: bool,
    pub snooze: u32,
    pub week_monday: bool,
    pub focus: u32,
    pub short_break: u32,
    pub long_break: u32,
    pub hud_volume: bool,
    pub hud_brightness: bool,
    pub hud_caps: bool,
    pub hud_language: bool,
    pub popup_battery: bool,
    pub popup_wifi: bool,
    pub popup_bluetooth: bool,
    pub open_click: bool,
    pub page_icons: bool,
    pub fast_animation: bool,
    pub shelf_clear: bool,
    pub calendar_week: bool,
    pub hide_fullscreen: bool,
    pub water: bool,
    // Additions beyond the original app.
    pub lyrics_pill: bool,
    pub clipboard: bool,
    pub clip_persist: bool,
    pub notifications: bool,
    pub notif_popup: bool,
    pub stats: bool,
    pub autohide_window: bool,
    pub exclude_capture: bool,
    pub mic_privacy: bool,
    pub scroll_volume: bool,
    pub downloads: bool,
    // Page visibility options
    pub page_home: bool,
    pub page_timer: bool,
    pub page_water: bool,
    pub page_todo: bool,
    pub page_calendar: bool,
    pub page_shelf: bool,
    pub page_clipboard: bool,
    pub page_notifs: bool,
    pub page_stats: bool,
    pub page_calc: bool,
    // Header tool visibility options
    pub tool_picker: bool,
    pub tool_search: bool,
    pub tool_audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            size: 0,
            display: 0,
            pet: true,
            minimal: false,
            sound: true,
            lyrics: true,
            todo_start: true,
            todo_end: true,
            snooze: 5,
            week_monday: false,
            focus: 25,
            short_break: 5,
            long_break: 15,
            hud_volume: true,
            hud_brightness: true,
            hud_caps: true,
            hud_language: true,
            popup_battery: true,
            popup_wifi: true,
            popup_bluetooth: true,
            open_click: false,
            page_icons: true,
            fast_animation: false,
            shelf_clear: false,
            calendar_week: false,
            hide_fullscreen: true,
            water: true,
            lyrics_pill: false,
            clipboard: true,
            clip_persist: false,
            notifications: true,
            notif_popup: true,
            stats: true,
            autohide_window: true,
            exclude_capture: true,
            mic_privacy: true,
            scroll_volume: true,
            downloads: true,
            page_home: true,
            page_timer: true,
            page_water: true,
            page_todo: true,
            page_calendar: true,
            page_shelf: true,
            page_clipboard: true,
            page_notifs: true,
            page_stats: true,
            page_calc: true,
            tool_picker: true,
            tool_search: true,
            tool_audio: true,
        }
    }
}

/// Every boolean setting, by its file key.
pub const BOOL_KEYS: &[&str] = &[
    "pet", "minimal", "sound", "lyrics", "todo_start", "todo_end", "week_monday", "hud_volume",
    "hud_brightness", "hud_caps", "hud_language", "popup_battery", "popup_wifi", "popup_bluetooth",
    "open_click", "page_icons", "fast_animation", "shelf_clear", "calendar_week", "hide_fullscreen",
    "water", "lyrics_pill", "clipboard", "clip_persist", "notifications", "notif_popup", "stats",
    "autohide_window", "exclude_capture", "mic_privacy", "scroll_volume", "downloads",
    "page_home", "page_timer", "page_water", "page_todo", "page_calendar", "page_shelf",
    "page_clipboard", "page_notifs", "page_stats", "page_calc",
    "tool_picker", "tool_search", "tool_audio",
];
pub const NUM_KEYS: &[&str] = &["size", "display", "snooze", "focus", "short_break", "long_break"];

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let text = store::read("settings.txt", true);
        for (k, v) in store::kv(&text) {
            if let Some(b) = s.bool_mut(k) {
                *b = v == "1" || v.eq_ignore_ascii_case("true");
            } else if let (Some(n), Ok(x)) = (s.num_mut(k), v.parse::<u32>()) {
                *n = x;
            }
        }
        s.clamp();
        s
    }

    pub fn save(&self) {
        let mut out = String::new();
        let mut me = self.clone();
        for k in NUM_KEYS {
            out += &format!("{k}={}\n", me.num_mut(k).map(|v| *v).unwrap_or(0));
        }
        for k in BOOL_KEYS {
            out += &format!("{k}={}\n", if me.bool_mut(k).map(|v| *v).unwrap_or(false) { 1 } else { 0 });
        }
        store::write("settings.txt", true, &out);
    }

    fn clamp(&mut self) {
        if ![0, 100, 115, 130, 150].contains(&self.size) {
            self.size = 0;
        }
        if ![5, 10, 15].contains(&self.snooze) {
            self.snooze = 5;
        }
        self.focus = self.focus.clamp(5, 120);
        self.short_break = self.short_break.clamp(1, 60);
        self.long_break = self.long_break.clamp(5, 90);
    }

    pub fn bool_mut(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            "pet" => &mut self.pet,
            "minimal" => &mut self.minimal,
            "sound" => &mut self.sound,
            "lyrics" => &mut self.lyrics,
            "todo_start" => &mut self.todo_start,
            "todo_end" => &mut self.todo_end,
            "week_monday" => &mut self.week_monday,
            "hud_volume" => &mut self.hud_volume,
            "hud_brightness" => &mut self.hud_brightness,
            "hud_caps" => &mut self.hud_caps,
            "hud_language" => &mut self.hud_language,
            "popup_battery" => &mut self.popup_battery,
            "popup_wifi" => &mut self.popup_wifi,
            "popup_bluetooth" => &mut self.popup_bluetooth,
            "open_click" => &mut self.open_click,
            "page_icons" => &mut self.page_icons,
            "fast_animation" => &mut self.fast_animation,
            "shelf_clear" => &mut self.shelf_clear,
            "calendar_week" => &mut self.calendar_week,
            "hide_fullscreen" => &mut self.hide_fullscreen,
            "water" => &mut self.water,
            "lyrics_pill" => &mut self.lyrics_pill,
            "clipboard" => &mut self.clipboard,
            "clip_persist" => &mut self.clip_persist,
            "notifications" => &mut self.notifications,
            "notif_popup" => &mut self.notif_popup,
            "stats" => &mut self.stats,
            "autohide_window" => &mut self.autohide_window,
            "exclude_capture" => &mut self.exclude_capture,
            "mic_privacy" => &mut self.mic_privacy,
            "scroll_volume" => &mut self.scroll_volume,
            "downloads" => &mut self.downloads,
            "page_home" => &mut self.page_home,
            "page_timer" => &mut self.page_timer,
            "page_water" => &mut self.page_water,
            "page_todo" => &mut self.page_todo,
            "page_calendar" => &mut self.page_calendar,
            "page_shelf" => &mut self.page_shelf,
            "page_clipboard" => &mut self.page_clipboard,
            "page_notifs" => &mut self.page_notifs,
            "page_stats" => &mut self.page_stats,
            "page_calc" => &mut self.page_calc,
            "tool_picker" => &mut self.tool_picker,
            "tool_search" => &mut self.tool_search,
            "tool_audio" => &mut self.tool_audio,
            _ => return None,
        })
    }

    pub fn num_mut(&mut self, key: &str) -> Option<&mut u32> {
        Some(match key {
            "size" => &mut self.size,
            "display" => &mut self.display,
            "snooze" => &mut self.snooze,
            "focus" => &mut self.focus,
            "short_break" => &mut self.short_break,
            "long_break" => &mut self.long_break,
            _ => return None,
        })
    }
}

fn exe_command() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("\"{}\"", exe.display())
}

pub fn startup_enabled() -> bool {
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            w!("Nuva"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
    };
    if r != ERROR_SUCCESS {
        return false;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    // Only count it as ours when it points at this exe.
    String::from_utf16_lossy(&buf[..n]).eq_ignore_ascii_case(&exe_command())
}

pub fn set_startup(on: bool) {
    unsafe {
        if on {
            let cmd: Vec<u16> = exe_command().encode_utf16().chain(Some(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("Nuva"),
                REG_SZ.0,
                Some(cmd.as_ptr() as *const _),
                (cmd.len() * 2) as u32,
            );
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, &HSTRING::from("Nusa"));
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, &HSTRING::from("Nuva"));
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, &HSTRING::from("Nusa"));
        }
    }
}
