#![windows_subsystem = "windows"]
//! Nuva — a living notch for Windows.

mod anim;
mod app;
mod clipboard;
mod events;
mod gfx;
mod lyrics;
mod media;
mod notch;
mod notifications;
mod pomodoro;
mod settings;
mod settings_window;
mod shelf;
mod sound;
mod stats;
mod store;
mod system;
mod time;
mod todo;
mod tray;
mod ui;
mod water;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

fn main() {
    let open_settings = std::env::args().any(|a| a == "--settings");
    // `--preview=<page>` starts expanded on a page and stays open (for testing layouts).
    let preview = std::env::args().find_map(|a| a.strip_prefix("--preview=").map(str::to_string));
    // A windowed app has no console: record panics where the user can find them.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("[{}] {info}\n{}\n", time::date_str(time::today()), std::backtrace::Backtrace::force_capture());
        let path = store::local_dir().join("crash.log");
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
    }));
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _mutex = CreateMutexW(None, true, w!("Local\\NuvaNotch.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS && preview.is_none() {
            // Already running: ask it to show settings instead.
            if let Ok(h) = FindWindowW(w!("NuvaNotchWindow"), None) {
                let _ = PostMessageW(h, events::WM_APP_SHOW_SETTINGS, WPARAM(0), LPARAM(0));
            }
            return;
        }
        let _ = OleInitialize(None);
        app::run(open_settings, preview);
    }
}
