//! Notification-area icon and its menu.

use crate::events::WM_APP_TRAY;
use crate::settings::Settings;
use crate::system;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_SETTINGS: usize = 100;
pub const ID_STARTUP: usize = 101;
pub const ID_PET: usize = 102;
pub const ID_MINIMAL: usize = 103;
pub const ID_ABOUT: usize = 104;
pub const ID_QUIT: usize = 105;
pub const ID_SIZE_BASE: usize = 200; // + index into SIZES
pub const ID_DISPLAY_BASE: usize = 300; // + display value
pub const SIZES: [u32; 5] = [0, 100, 115, 130, 150];

/// Draw a 32x32 icon: a screen outline with a notch at the top.
fn make_icon() -> HICON {
    const S: i32 = 32;
    unsafe {
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: S,
                biHeight: -S,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0) else { return HICON::default() };
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (S * S) as usize);
        // Signed distance to a rounded rectangle, for anti-aliased edges.
        let rr = |x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32| -> f32 {
            let qx = (x - cx).abs() - (hw - r);
            let qy = (y - cy).abs() - (hh - r);
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
        };
        for y in 0..S {
            for x in 0..S {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let outer = rr(fx, fy, 16.0, 16.0, 14.5, 12.0, 4.0);
                let ring = (0.5 - (outer.abs() - 1.1)).clamp(0.0, 1.0);
                let notch = (0.5 - rr(fx, fy, 16.0, 6.5, 6.5, 2.75, 2.75)).clamp(0.0, 1.0);
                let a = ring.max(notch);
                let v = (a * 255.0) as u32;
                px[(y * S + x) as usize] = (v << 24) | (v << 16) | (v << 8) | v;
            }
        }
        let mask = CreateBitmap(S, S, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(mask);
        let _ = DeleteObject(color);
        icon
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

pub fn add(hwnd: HWND) {
    let mut d = data(hwnd);
    d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    d.uCallbackMessage = WM_APP_TRAY;
    d.hIcon = make_icon();
    for (i, c) in "Nuva".encode_utf16().enumerate() {
        d.szTip[i] = c;
    }
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &d).as_bool() {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }
}

pub fn remove(hwnd: HWND) {
    let d = data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &d);
    }
}

pub fn show_menu(hwnd: HWND, s: &Settings) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(crate::settings::startup_enabled()), ID_STARTUP, w!("Start with Windows"));
        let _ = AppendMenuW(menu, MF_STRING | check(s.pet), ID_PET, w!("Nuva Mascot"));
        let _ = AppendMenuW(menu, MF_STRING | check(s.minimal), ID_MINIMAL, w!("Minimal notch"));
        if let Ok(sub) = CreatePopupMenu() {
            for (i, v) in SIZES.iter().enumerate() {
                let label = if *v == 0 { "Auto".to_string() } else { format!("{v}%") };
                let _ = AppendMenuW(sub, MF_STRING | check(s.size == *v), ID_SIZE_BASE + i, &HSTRING::from(label));
            }
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Notch size"));
        }
        let mons = system::monitors();
        if mons.len() > 1 {
            if let Ok(sub) = CreatePopupMenu() {
                let _ = AppendMenuW(sub, MF_STRING | check(s.display == 0), ID_DISPLAY_BASE, w!("Main display"));
                for i in 0..mons.len() {
                    let label = format!("Display {}{}", i + 1, if mons[i].primary { " (main)" } else { "" });
                    let v = i as u32 + 1;
                    let _ = AppendMenuW(sub, MF_STRING | check(s.display == v), ID_DISPLAY_BASE + v as usize, &HSTRING::from(label));
                }
                let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Display"));
            }
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT, w!("About Nuva"));
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit Nuva"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
        let _ = PostMessageW(hwnd, WM_NULL, None, None);
        let _ = DestroyMenu(menu);
    }
}
