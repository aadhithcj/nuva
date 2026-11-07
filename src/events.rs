//! Cross-thread events: worker threads push here and wake the notch window.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Defined in Win32::UI::Controls, which we don't otherwise need.
pub const WM_MOUSELEAVE: u32 = 0x02A3;

pub const WM_APP_EVENT: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 2;
pub const WM_APP_SHOW_SETTINGS: u32 = WM_APP + 3;
pub const WM_APP_CAPS: u32 = WM_APP + 4;
pub const WM_APP_LANGCHECK: u32 = WM_APP + 5;
pub const WM_APP_VOLUME: u32 = WM_APP + 6;
pub const WM_APP_FOREGROUND: u32 = WM_APP + 7;
pub const WM_APP_COLOR_PICKED: u32 = WM_APP + 8;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LiveActivityData {
    pub title: String,
    pub sub: String,
    pub progress: Option<f32>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DownloadInfo {
    pub filename: String,
    pub size_str: String,
    pub progress: Option<f32>,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct LoupeSample {
    pub x: i32,
    pub y: i32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 9x9 row-major grid of 0x00RRGGBB pixels centered on (x, y)
    pub grid: [u32; 81],
}

impl Default for LoupeSample {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            r: 0,
            g: 0,
            b: 0,
            grid: [0; 81],
        }
    }
}

#[allow(dead_code)]
pub enum Event {
    Media(crate::media::MediaState),
    MediaArt(String, Vec<u8>),
    AudioSpectrum([f32; 5]),
    Lyrics(u64, Option<crate::lyrics::Lyrics>),
    Network(crate::system::NetState),
    Bluetooth(String, bool),
    Notifications(Vec<crate::notifications::Notif>),
    NotifIcon(String, Vec<u8>),
    NotificationToast {
        title: String,
        sub: String,
    },
    LiveActivity(Option<LiveActivityData>),
    ShelfImageConverted {
        old_path: std::path::PathBuf,
        new_path: std::path::PathBuf,
    },
    ShelfOcrSuccess {
        path: std::path::PathBuf,
        text: String,
    },
    ShelfZipCreated {
        path: std::path::PathBuf,
    },
    ShelfActionFailed(String),
    ColorHover(LoupeSample),
    ColorPicked(u8, u8, u8),
    ColorCanceled,
}

static QUEUE: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
static NOTCH: AtomicIsize = AtomicIsize::new(0);

pub fn set_target(hwnd: HWND) {
    NOTCH.store(hwnd.0 as isize, Ordering::SeqCst);
}

pub fn target() -> HWND {
    HWND(NOTCH.load(Ordering::SeqCst) as *mut _)
}

pub fn push(e: Event) {
    let first = {
        let mut q = QUEUE.lock().unwrap();
        q.push_back(e);
        q.len() == 1
    };
    if first {
        post(WM_APP_EVENT, 0, 0);
    }
}

pub fn post(msg: u32, w: usize, l: isize) {
    let h = target();
    if !h.0.is_null() {
        unsafe {
            let _ = PostMessageW(h, msg, WPARAM(w), LPARAM(l));
        }
    }
}

pub fn drain() -> Vec<Event> {
    QUEUE.lock().unwrap().drain(..).collect()
}
