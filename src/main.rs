#![windows_subsystem = "windows"]
//! Nuva — a living notch for Windows.

mod anim;
mod app;
mod clipboard;
mod cmd;
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
        let args: Vec<String> = std::env::args().collect();
        let mut activity = None;
        let mut progress = None;
        let mut notify = None;
        let mut clear_act = false;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "--activity" && i + 1 < args.len() {
                activity = Some(args[i + 1].clone());
                i += 2;
            } else if args[i] == "--progress" && i + 1 < args.len() {
                progress = args[i + 1].parse::<f32>().ok();
                i += 2;
            } else if args[i] == "--notify" && i + 1 < args.len() {
                notify = Some(args[i + 1].clone());
                i += 2;
            } else if args[i] == "--clear-activity" {
                clear_act = true;
                i += 1;
            } else {
                i += 1;
            }
        }

        if clear_act || activity.is_some() || notify.is_some() {
            let payload = if clear_act {
                "clear".to_string()
            } else if let Some(n) = notify {
                format!(r#"{{"notify":"{}"}}"#, n.replace('"', "\\\""))
            } else if let Some(a) = activity {
                if let Some(p) = progress {
                    let frac = if p > 1.0 { p / 100.0 } else { p };
                    format!(r#"{{"title":"{}","progress":{}}}"#, a.replace('"', "\\\""), frac)
                } else {
                    format!(r#"{{"title":"{}"}}"#, a.replace('"', "\\\""))
                }
            } else {
                String::new()
            };

            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(r"\\.\pipe\nuva_notch") {
                let _ = f.write_all(payload.as_bytes());
            }
            return;
        }

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
