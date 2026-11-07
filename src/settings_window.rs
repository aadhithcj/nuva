//! The settings window: a normal, resizable window drawn with the same canvas as the notch.

use crate::app::{self, App};
use crate::events::WM_MOUSELEAVE;
use crate::gfx::{ico, pal, Align, Canvas, Gfx, Rect, W};
use crate::settings;
use crate::system;
use crate::time;
use crate::ui::Action;
use windows::core::{w, Interface, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_UNKNOWN, D2D1_PIXEL_FORMAT, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const SECTIONS: [(&str, char); 11] = [
    ("General", ico::SETTINGS),
    ("Pages", ico::CHECKLIST),
    ("Appearance", ico::ART),
    ("Music and lyrics", ico::MUSIC),
    ("To-do & calendar", ico::CALENDAR),
    ("Pomodoro", ico::TIMER),
    ("Water", ico::DROP),
    ("Shelf & clipboard", ico::FOLDER),
    ("HUD & popups", ico::VOLUME),
    ("Notifications & stats", ico::BELL),
    ("About", ico::INFO),
];

pub struct SettingsWindow {
    pub hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    gfx: Gfx,
    pub section: usize,
    scroll: f32,
    content_h: f32,
    hits: Vec<(Rect, Action)>,
    mouse: Option<(f32, f32)>,
    pressed: Option<Action>,
    scale: f32,
    tracking: bool,
}

const SIDEBAR: f32 = 220.0;

impl SettingsWindow {
    pub fn open(gfx: &Gfx) -> Option<SettingsWindow> {
        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            static REG: std::sync::Once = std::sync::Once::new();
            REG.call_once(|| {
                let icon_big = crate::tray::app_icon(false);
                let icon_sm = crate::tray::app_icon(true);
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wndproc),
                    hInstance: hinst.into(),
                    hIcon: icon_big,
                    hIconSm: icon_sm,
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    lpszClassName: w!("NuvaSettings"),
                    ..Default::default()
                };
                RegisterClassExW(&wc);
            });
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("NuvaSettings"),
                w!("Nuva Settings"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                100,
                100,
                None,
                None,
                hinst,
                None,
            )
            .map_err(|e| crate::store::debug(&format!("settings CreateWindowExW failed: {e}")))
            .ok()?;
            let icon_big = crate::tray::app_icon(false);
            let icon_sm = crate::tray::app_icon(true);
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as _), LPARAM(icon_big.0 as _));
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as _), LPARAM(icon_sm.0 as _));
            let dark: BOOL = true.into();
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as *const _, 4);
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            let (w, h) = ((900.0 * scale) as i32, (640.0 * scale) as i32);
            let sx = GetSystemMetrics(SM_CXSCREEN);
            let sy = GetSystemMetrics(SM_CYSCREEN);
            let _ = SetWindowPos(hwnd, None, (sx - w) / 2, (sy - h) / 2, w, h, SWP_NOZORDER);
            let me = SettingsWindow {
                hwnd,
                rt: None,
                gfx: gfx.clone(),
                section: 0,
                scroll: 0.0,
                content_h: 0.0,
                hits: Vec::new(),
                mouse: None,
                pressed: None,
                scale,
                tracking: false,
            };
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            Some(me)
        }
    }

    pub fn focus(&self) {
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(self.hwnd, None, false);
        }
    }

    fn client(&self) -> (u32, u32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
        }
        ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32)
    }

    fn ensure_rt(&mut self) -> Option<ID2D1HwndRenderTarget> {
        if self.rt.is_none() {
            let (w, h) = self.client();
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_UNKNOWN, alphaMode: D2D1_ALPHA_MODE_UNKNOWN },
                ..Default::default()
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: self.hwnd, pixelSize: D2D_SIZE_U { width: w, height: h }, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
            self.rt = unsafe { self.gfx.d2d.CreateHwndRenderTarget(&props, &hp) }.ok();
        }
        self.rt.clone()
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = app::with(|a| {
        let Some(mut sw) = a.settings_win.take() else { return None };
        if sw.hwnd != hwnd {
            a.settings_win = Some(sw);
            return None;
        }
        let r = handle(a, &mut sw, msg, wp, lp);
        // Keep the window unless it was destroyed (possibly re-entrantly, e.g. Esc).
        let alive = msg != WM_DESTROY && unsafe { IsWindow(hwnd) }.as_bool();
        if alive && a.settings_win.is_none() {
            a.settings_win = Some(sw);
        }
        r
    })
    .flatten();
    match handled {
        Some(r) => r,
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn lo(v: isize) -> f32 {
    (v & 0xffff) as i16 as f32
}
fn hi(v: isize) -> f32 {
    ((v >> 16) & 0xffff) as i16 as f32
}

fn handle(a: &mut App, sw: &mut SettingsWindow, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(sw.hwnd, &mut ps);
            }
            paint(a, sw);
            unsafe {
                let _ = EndPaint(sw.hwnd, &ps);
            }
        }
        WM_SIZE => {
            if let Some(rt) = &sw.rt {
                let (w, h) = sw.client();
                unsafe {
                    let _ = rt.Resize(&D2D_SIZE_U { width: w, height: h });
                }
            }
            sw.invalidate();
        }
        WM_DPICHANGED => {
            sw.scale = (wp.0 & 0xffff) as f32 / 96.0;
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(sw.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            sw.rt = None;
            sw.invalidate();
        }
        WM_GETMINMAXINFO => {
            let mm = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
            mm.ptMinTrackSize = POINT { x: (720.0 * sw.scale) as i32, y: (480.0 * sw.scale) as i32 };
        }
        WM_ERASEBKGND => return Some(LRESULT(1)),
        WM_MOUSEMOVE => {
            sw.mouse = Some((lo(lp.0) / sw.scale, hi(lp.0) / sw.scale));
            if !sw.tracking {
                sw.tracking = true;
                let mut t = windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<windows::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT>() as u32,
                    dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                    hwndTrack: sw.hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut t);
                }
            }
            sw.invalidate();
        }
        WM_MOUSELEAVE => {
            sw.tracking = false;
            sw.mouse = None;
            sw.invalidate();
        }
        WM_LBUTTONDOWN => {
            let p = (lo(lp.0) / sw.scale, hi(lp.0) / sw.scale);
            sw.pressed = sw.hits.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, a)| a.clone());
            unsafe {
                SetCapture(sw.hwnd);
            }
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            let p = (lo(lp.0) / sw.scale, hi(lp.0) / sw.scale);
            let hit = sw.hits.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, a)| a.clone());
            if let (Some(x), Some(y)) = (sw.pressed.take(), hit) {
                if x == y {
                    match x {
                        Action::SetSection(i) => {
                            sw.section = i;
                            sw.scroll = 0.0;
                        }
                        other => apply(a, other),
                    }
                }
            }
            sw.invalidate();
        }
        WM_MOUSEWHEEL => {
            let d = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            let (_, h) = sw.client();
            let max = (sw.content_h - h as f32 / sw.scale + 24.0).max(0.0);
            sw.scroll = (sw.scroll - d * 48.0).clamp(0.0, max);
            sw.invalidate();
        }
        WM_KEYDOWN => {
            if wp.0 == 0x1B {
                unsafe {
                    let _ = DestroyWindow(sw.hwnd);
                }
            }
        }
        WM_DESTROY => {
            sw.rt = None;
            a.settings_win = None;
            return Some(LRESULT(0));
        }
        _ => return None,
    }
    Some(LRESULT(0))
}

/// Settings actions (shared with the notch's dispatcher).
pub fn apply(a: &mut App, act: Action) {
    match act {
        Action::SetToggle(k) => a.toggle_setting(k),
        Action::SetNum(k, v) => a.set_num(k, v),
        Action::SetStep(k, d) => {
            let (min, max) = match k {
                "focus" => (5, 120),
                "short_break" => (1, 60),
                _ => (5, 90),
            };
            let cur = a.settings.clone().num_mut(k).map(|v| *v).unwrap_or(0) as i32;
            a.set_num(k, (cur + d).clamp(min, max) as u32);
        }
        Action::SetStartup => {
            settings::set_startup(!settings::startup_enabled());
            a.settings_changed();
        }
        Action::SetDisplay(n) => a.set_num("display", n),
        Action::WaterGoal(ml) => {
            a.water.goal = ml;
            a.water.save();
            a.settings_changed();
        }
        Action::WaterStep(k, d) => {
            let wt = &mut a.water;
            match k {
                "bottle" => wt.bottle = (wt.bottle as i32 + d).clamp(100, 3000) as u32,
                "start" => wt.start = (wt.start as i32 + d).clamp(0, wt.end as i32 - 1) as u32,
                _ => wt.end = (wt.end as i32 + d).clamp(wt.start as i32 + 1, 24) as u32,
            }
            wt.save();
            a.settings_changed();
        }
        Action::OpenUrl(u) => unsafe {
            ShellExecuteW(None, w!("open"), &HSTRING::from(u), None, None, SW_SHOWNORMAL);
        },
        Action::OpenDataFolder => unsafe {
            let p = crate::store::data_dir();
            ShellExecuteW(None, w!("open"), &HSTRING::from(p.as_os_str()), None, None, SW_SHOWNORMAL);
        },
        Action::PageMoveUp(idx) => {
            let mut list = a.settings.page_order_list();
            if idx > 0 && idx < list.len() {
                list.swap(idx, idx - 1);
                a.settings.page_order = list.join(",");
                a.settings.save();
                a.settings_changed();
            }
        }
        Action::PageMoveDown(idx) => {
            let mut list = a.settings.page_order_list();
            if idx + 1 < list.len() {
                list.swap(idx, idx + 1);
                a.settings.page_order = list.join(",");
                a.settings.save();
                a.settings_changed();
            }
        }
        Action::PageResetOrder => {
            a.settings.page_order = crate::settings::DEFAULT_PAGE_ORDER.to_string();
            a.settings.save();
            a.settings_changed();
        }
        _ => {}
    }
}

fn paint(a: &mut App, sw: &mut SettingsWindow) {
    let Some(hrt) = sw.ensure_rt() else { return };
    let Ok(rt) = hrt.cast::<ID2D1RenderTarget>() else { return };
    let (w, h) = sw.client();
    let (w, h) = (w as f32 / sw.scale, h as f32 / sw.scale);
    unsafe {
        rt.SetDpi(96.0 * sw.scale, 96.0 * sw.scale);
        rt.BeginDraw();
    }
    let Ok(mut c) = Canvas::new(rt.clone(), sw.gfx.clone(), sw.mouse) else {
        unsafe {
            let _ = rt.EndDraw(None, None);
        }
        return;
    };
    c.clear(crate::gfx::Color::hex(0x111113));

    // Sidebar
    c.rect(Rect::new(0.0, 0.0, SIDEBAR, h), crate::gfx::Color::hex(0x0b0b0c));
    c.rrect(Rect::new(24.0, 26.0, 34.0, 22.0), 8.0, pal::CARD_HI);
    c.rrect(Rect::new(31.0, 26.0, 20.0, 8.0), 4.0, pal::BLACK);
    c.text("Nuva", Rect::new(68.0, 22.0, 120.0, 30.0), 18.0, W::Semi, pal::TEXT, Align::Left);
    for (i, (name, icon)) in SECTIONS.iter().enumerate() {
        let r = Rect::new(12.0, 74.0 + i as f32 * 38.0, SIDEBAR - 24.0, 34.0);
        let hov = c.hit(r, Action::SetSection(i));
        if i == sw.section {
            c.rrect(r, 9.0, pal::CARD_HI);
        } else if hov {
            c.rrect(r, 9.0, pal::CARD);
        }
        c.icon(*icon, Rect::new(r.x + 8.0, r.y, 22.0, r.h), 12.0, if i == sw.section { pal::ACCENT } else { pal::DIM });
        c.text(name, Rect::new(r.x + 38.0, r.y, r.w - 40.0, r.h), 13.0, if i == sw.section { W::Semi } else { W::Regular }, pal::TEXT, Align::Left);
    }

    // Content
    let area = Rect::new(SIDEBAR, 0.0, w - SIDEBAR, h);
    c.clip(area);
    let x = SIDEBAR + 32.0;
    let cw = (w - SIDEBAR - 64.0).min(720.0);
    let mut y = 28.0 - sw.scroll;
    c.text(SECTIONS[sw.section].0, Rect::new(x, y, cw, 34.0), 22.0, W::Semi, pal::TEXT, Align::Left);
    y += 52.0;
    let mut f = Form { c: &mut c, x, w: cw, y };
    match sw.section {
        0 => general(a, &mut f),
        1 => pages_section(a, &mut f),
        2 => appearance(a, &mut f),
        3 => music(a, &mut f),
        4 => todo(a, &mut f),
        5 => pomodoro(a, &mut f),
        6 => water(a, &mut f),
        7 => shelf(a, &mut f),
        8 => hud(a, &mut f),
        9 => notifs(a, &mut f),
        _ => about(&mut f),
    }
    sw.content_h = f.y + sw.scroll;
    c.unclip();
    sw.hits = std::mem::take(&mut c.hits);
    let res = unsafe { rt.EndDraw(None, None) };
    if res.is_err() {
        sw.rt = None;
    }
}

// ---------------------------------------------------------------- form helpers

struct Form<'a> {
    c: &'a mut Canvas,
    x: f32,
    w: f32,
    y: f32,
}

const ROW_H: f32 = 58.0;

impl Form<'_> {
    fn group(&mut self, title: &str) {
        self.y += 6.0;
        self.c.text(title, Rect::new(self.x, self.y, self.w, 20.0), 12.0, W::Semi, pal::DIM, Align::Left);
        self.y += 26.0;
    }

    /// Draws a row card and returns the rect where its control goes (right side).
    fn row(&mut self, title: &str, desc: &str, control_w: f32) -> Rect {
        let r = Rect::new(self.x, self.y, self.w, ROW_H - 6.0);
        self.c.rrect(r, 12.0, pal::CARD);
        let text_w = r.w - control_w - 40.0;
        if desc.is_empty() {
            self.c.text(title, Rect::new(r.x + 18.0, r.y, text_w, r.h), 13.5, W::Medium, pal::TEXT, Align::Left);
        } else {
            self.c.text(title, Rect::new(r.x + 18.0, r.y + 8.0, text_w, 20.0), 13.5, W::Medium, pal::TEXT, Align::Left);
            self.c.text(desc, Rect::new(r.x + 18.0, r.y + 27.0, text_w, 17.0), 11.5, W::Regular, pal::DIM, Align::Left);
        }
        self.y += ROW_H;
        Rect::new(r.r() - control_w - 16.0, r.y, control_w, r.h)
    }

    fn toggle(&mut self, title: &str, desc: &str, on: bool, a: Action) {
        let start = self.y;
        let ctl = self.row(title, desc, 44.0);
        let whole = Rect::new(self.x, start, self.w, ROW_H - 6.0);
        self.c.hit(whole, a.clone());
        self.c.toggle(Rect::new(ctl.x, ctl.y, ctl.w, ctl.h), on, a);
    }

    fn seg(&mut self, title: &str, desc: &str, opts: &[&str], sel: usize, mk: &dyn Fn(usize) -> Action) {
        let cw = (opts.len() as f32 * 76.0).min(self.w * 0.55);
        let ctl = self.row(title, desc, cw);
        self.c.segmented(Rect::new(ctl.x, ctl.cy() - 15.0, ctl.w, 30.0), opts, sel, mk);
    }

    fn step(&mut self, title: &str, desc: &str, value: &str, dec: Action, inc: Action) {
        let ctl = self.row(title, desc, 150.0);
        self.c.stepper(Rect::new(ctl.x, ctl.cy() - 15.0, ctl.w, 30.0), value, dec, inc);
    }

    fn note(&mut self, text: &str) {
        let h = self.c.measure_h(text, self.w, 11.5, W::Regular) + 6.0;
        self.c.text_wrap(text, Rect::new(self.x + 4.0, self.y, self.w - 8.0, h + 4.0), 11.5, W::Regular, pal::FAINT, Align::Left);
        self.y += h + 10.0;
    }

    fn page_row(&mut self, idx: usize, total: usize, icon: char, title: &str, desc: &str, on: bool, toggle_key: &'static str) {
        let r = Rect::new(self.x, self.y, self.w, ROW_H - 6.0);
        self.c.rrect(r, 12.0, pal::CARD);

        // Up button
        let up_r = Rect::new(r.x + 8.0, r.y + 6.0, 24.0, 18.0);
        if idx > 0 {
            self.c.icon_button(up_r, ico::UP, 9.0, Action::PageMoveUp(idx), pal::DIM);
        } else {
            self.c.icon(ico::UP, up_r, 9.0, pal::FAINT);
        }

        // Down button
        let down_r = Rect::new(r.x + 8.0, r.y + 26.0, 24.0, 18.0);
        if idx + 1 < total {
            self.c.icon_button(down_r, ico::DOWN, 9.0, Action::PageMoveDown(idx), pal::DIM);
        } else {
            self.c.icon(ico::DOWN, down_r, 9.0, pal::FAINT);
        }

        // Page icon
        let icon_r = Rect::new(r.x + 38.0, r.y + 13.0, 26.0, 26.0);
        self.c.icon(icon, icon_r, 14.0, if on { pal::ACCENT } else { pal::DIM });

        // Title and description
        let text_w = r.w - 120.0;
        self.c.text(title, Rect::new(r.x + 70.0, r.y + 8.0, text_w, 20.0), 13.5, W::Medium, if on { pal::TEXT } else { pal::DIM }, Align::Left);
        self.c.text(desc, Rect::new(r.x + 70.0, r.y + 27.0, text_w, 17.0), 11.5, W::Regular, pal::DIM, Align::Left);

        // Toggle switch
        let ctl = Rect::new(r.r() - 60.0, r.y, 44.0, r.h);
        self.c.toggle(ctl, on, Action::SetToggle(toggle_key));

        self.y += ROW_H;
    }
}

fn bool_seg(f: &mut Form, title: &str, desc: &str, opts: [&str; 2], key: &'static str, cur: bool) {
    f.seg(title, desc, &opts, cur as usize, &|i| if (i == 1) != cur { Action::SetToggle(key) } else { Action::None });
}

// ---------------------------------------------------------------- sections

fn general(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Startup");
    f.toggle("Start with Windows", "Launch Nuva when you sign in", settings::startup_enabled(), Action::SetStartup);
    f.group("Notch");
    let sizes = [0u32, 100, 115, 130, 150];
    let sel = sizes.iter().position(|v| *v == s.size).unwrap_or(0);
    f.seg("Size", "Scales the whole notch", &["Auto", "100%", "115%", "130%", "150%"], sel, &|i| Action::SetNum("size", sizes[i]));
    f.toggle("Minimal notch", "A smaller pill when nothing is playing", s.minimal, Action::SetToggle("minimal"));
    f.toggle("Reminder sounds", "Chimes for Pomodoro, To-do and Water", s.sound, Action::SetToggle("sound"));
    f.toggle("Hide in full screen", "Games, videos and slides that fill the screen", s.hide_fullscreen, Action::SetToggle("hide_fullscreen"));
    f.toggle("Auto-hide over windows", "Only show on top-center hover when a window is open", s.autohide_window, Action::SetToggle("autohide_window"));
    f.toggle("Exclude from screen capture", "Hide notch during screen sharing & recording", s.exclude_capture, Action::SetToggle("exclude_capture"));
    f.toggle("Scroll pill for volume", "Scroll up or down on the collapsed notch to change volume", s.scroll_volume, Action::SetToggle("scroll_volume"));

    f.group("Display");
    let mons = system::monitors();
    if mons.len() < 2 {
        f.note("Connect another display to choose where the notch appears.");
        return;
    }
    f.note("Click a screen to move the notch there.");
    // Draw the monitor layout to scale.
    let (minx, miny) = (mons.iter().map(|m| m.rect.left).min().unwrap_or(0), mons.iter().map(|m| m.rect.top).min().unwrap_or(0));
    let maxx = mons.iter().map(|m| m.rect.right).max().unwrap_or(1);
    let maxy = mons.iter().map(|m| m.rect.bottom).max().unwrap_or(1);
    let box_h = 150.0;
    let sc = (f.w / (maxx - minx) as f32).min(box_h / (maxy - miny) as f32);
    let ox = f.x + (f.w - (maxx - minx) as f32 * sc) / 2.0;
    let cur = if s.display == 0 { mons.iter().position(|m| m.primary).unwrap_or(0) } else { s.display as usize - 1 };
    for (i, m) in mons.iter().enumerate() {
        let r = Rect::new(
            ox + (m.rect.left - minx) as f32 * sc + 3.0,
            f.y + (m.rect.top - miny) as f32 * sc + 3.0,
            (m.rect.right - m.rect.left) as f32 * sc - 6.0,
            (m.rect.bottom - m.rect.top) as f32 * sc - 6.0,
        );
        let hov = f.c.hit(r, Action::SetDisplay(i as u32 + 1));
        f.c.rrect(r, 10.0, if i == cur { pal::ACCENT.a(0.25) } else if hov { pal::CARD_HI } else { pal::CARD });
        f.c.rrect_stroke(r, 10.0, if i == cur { pal::ACCENT } else { pal::LINE }, 1.5);
        f.c.rrect(Rect::new(r.cx() - 18.0, r.y + 1.5, 36.0, 7.0), 3.5, pal::BLACK);
        let label = format!("Display {}{}", i + 1, if m.primary { " · Main" } else { "" });
        f.c.text(&label, r, 12.0, W::Semi, pal::TEXT, Align::Center);
    }
    f.y += box_h + 16.0;
    f.toggle("Follow the main display", "Always use whichever screen is set as main", s.display == 0, Action::SetDisplay(0));
}

fn pages_section(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Active Notch Pages & Order");
    
    // Reset order button aligned right
    let reset_r = Rect::new(f.x + f.w - 110.0, f.y - 28.0, 110.0, 24.0);
    f.c.button(reset_r, "Reset Order", Action::PageResetOrder, pal::CARD_HI, pal::TEXT);

    f.note("Reorder and toggle pages in the notch. Click ▲ and ▼ to change the order. Disabled pages are hidden from page dots and swipe gestures.");

    let order = s.page_order_list();
    let total = order.len();

    for (i, key) in order.iter().enumerate() {
        let (title, desc, icon, on, toggle_key) = match key.as_str() {
            "home" => ("Now Playing (Music)", "Media player controls and synced lyrics", ico::MUSIC, s.page_home, "page_home"),
            "timer" => ("Focus Timer", "Pomodoro with short and long breaks", ico::TIMER, s.page_timer, "page_timer"),
            "water" => ("Water Tracker", "Water intake tracker and reminders", ico::DROP, s.page_water, "page_water"),
            "todo" => ("To-do List", "Tasks with start and end reminders", ico::CHECKLIST, s.page_todo, "page_todo"),
            "calendar" => ("Calendar", "Month and week views with your tasks", ico::CALENDAR, s.page_calendar, "page_calendar"),
            "shelf" => ("File Shelf", "Drop files, links and snippets", ico::FOLDER, s.page_shelf, "page_shelf"),
            "clipboard" => ("Clipboard History", "Recent copies with pin and search", ico::CLIPBOARD, s.page_clipboard, "page_clipboard"),
            "notifs" => ("Notifications", "Windows notification mirror", ico::BELL, s.page_notifs, "page_notifs"),
            "stats" => ("System Stats", "Live CPU, GPU, RAM, disk and network graphs", ico::STATS, s.page_stats, "page_stats"),
            "calc" => ("Calculator & Search", "Live math calculator, unit converter and app launcher", ico::CALCULATOR, s.page_calc, "page_calc"),
            _ => continue,
        };
        f.page_row(i, total, icon, title, desc, on, toggle_key);
    }
}

fn appearance(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Hover");
    bool_seg(f, "Open the notch", "How the notch opens from the pill", ["Hover", "Click"], "open_click", s.open_click);
    f.toggle("Page icons", "Show page icons when you hover the page dots", s.page_icons, Action::SetToggle("page_icons"));
    f.group("Motion");
    bool_seg(f, "Animation speed", "", ["Normal", "Fast"], "fast_animation", s.fast_animation);
}

fn music(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Lyrics");
    f.toggle("Show lyrics", "Synced lyrics from LRCLIB in the notch", s.lyrics, Action::SetToggle("lyrics"));
    f.toggle("Lyric line in the pill", "Show the line being sung while the notch is closed", s.lyrics_pill, Action::SetToggle("lyrics_pill"));
    f.note("Works with any player that shows in Windows' media controls: Spotify, Apple Music, YouTube in a browser, VLC and more.");
}

fn todo(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Reminders");
    f.toggle("When a task starts", "", s.todo_start, Action::SetToggle("todo_start"));
    f.toggle("When time is up", "", s.todo_end, Action::SetToggle("todo_end"));
    let sn = [5u32, 10, 15];
    f.seg("Snooze for", "Used by \"Later\" on reminders", &["5 min", "10 min", "15 min"], sn.iter().position(|v| *v == s.snooze).unwrap_or(0), &|i| Action::SetNum("snooze", sn[i]));
    f.group("Calendar");
    bool_seg(f, "Week starts on", "", ["Sunday", "Monday"], "week_monday", s.week_monday);
    bool_seg(f, "Default view", "", ["Month", "Week"], "calendar_week", s.calendar_week);
}

fn pomodoro(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Durations");
    f.step("Focus", "", &format!("{} min", s.focus), Action::SetStep("focus", -5), Action::SetStep("focus", 5));
    f.step("Short break", "", &format!("{} min", s.short_break), Action::SetStep("short_break", -1), Action::SetStep("short_break", 1));
    f.step("Long break", "Every 4th break", &format!("{} min", s.long_break), Action::SetStep("long_break", -5), Action::SetStep("long_break", 5));
}

fn water(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Water reminders");
    f.toggle("Water reminders", "Spread across your active hours", s.water, Action::SetToggle("water"));
    f.group("Plan");
    let goals = [1500u32, 2000, 2500, 3000];
    let sel = goals.iter().position(|g| *g == a.water.goal).unwrap_or(1);
    f.seg("Daily goal", "", &["1.5 L", "2 L", "2.5 L", "3 L"], sel, &|i| Action::WaterGoal(goals[i]));
    f.step("Bottle size", "", &format!("{} ml", a.water.bottle), Action::WaterStep("bottle", -50), Action::WaterStep("bottle", 50));
    f.step("Active from", "", &time::hm(a.water.start * 60, a.h24), Action::WaterStep("start", -1), Action::WaterStep("start", 1));
    f.step("Active until", "", &time::hm((a.water.end % 24) * 60, a.h24), Action::WaterStep("end", -1), Action::WaterStep("end", 1));
    f.group("Today");
    let per = a.water.per();
    f.note(&format!(
        "{} ml of {} ml so far · {} reminders of {} ml each between {} and {}.",
        a.water.taken,
        a.water.goal,
        a.water.count(),
        per,
        time::hm(a.water.start * 60, a.h24),
        time::hm((a.water.end % 24) * 60, a.h24)
    ));
}

fn shelf(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Shelf");
    f.toggle("Start with an empty Shelf", "Files from your last session are cleared when Nuva starts", s.shelf_clear, Action::SetToggle("shelf_clear"));
    f.group("Clipboard");
    f.toggle("Clipboard history", "Keep recent copies in a notch page", s.clipboard, Action::SetToggle("clipboard"));
    f.toggle("Remember after restart", "Saves text items (not images) to your data folder", s.clip_persist, Action::SetToggle("clip_persist"));
    f.note("Password managers and apps that mark their clipboard data as private are skipped.");
}

fn hud(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Indicators");
    f.toggle("Volume", "", s.hud_volume, Action::SetToggle("hud_volume"));
    f.toggle("Brightness", "Laptop screens", s.hud_brightness, Action::SetToggle("hud_brightness"));
    f.toggle("Caps Lock", "", s.hud_caps, Action::SetToggle("hud_caps"));
    f.toggle("Input language", "When you switch keyboard layout", s.hud_language, Action::SetToggle("hud_language"));
    f.toggle("Microphone privacy & mute", "Orange dot when mic is in use; click to toggle mute", s.mic_privacy, Action::SetToggle("mic_privacy"));
    f.toggle("Download progress", "Glow line along notch bottom during downloads", s.downloads, Action::SetToggle("downloads"));
    f.group("Popups");
    f.toggle("Battery", "Charger, low battery and fully charged", s.popup_battery, Action::SetToggle("popup_battery"));
    f.toggle("Wi-Fi", "Connected, no internet, disconnected", s.popup_wifi, Action::SetToggle("popup_wifi"));
    f.toggle("Bluetooth headset", "When earbuds or headphones connect", s.popup_bluetooth, Action::SetToggle("popup_bluetooth"));
    f.group("Header Tools");
    f.toggle("Color picker button", "Eyedropper icon to pick pixel color from screen", s.tool_picker, Action::SetToggle("tool_picker"));
    f.toggle("Calculator & Search button", "Quick launcher button in the notch header", s.tool_search, Action::SetToggle("tool_search"));
    f.toggle("Audio output switcher", "Quick button to cycle playback devices", s.tool_audio, Action::SetToggle("tool_audio"));
}

fn notifs(a: &mut App, f: &mut Form) {
    let s = a.settings.clone();
    f.group("Notifications");
    f.toggle("Notification mirror", "A page listing your Windows notifications", s.notifications, Action::SetToggle("notifications"));
    f.toggle("Show new ones in the notch", "Mail, chat and other app alerts pop up briefly", s.notif_popup, Action::SetToggle("notif_popup"));
    f.note("Read locally through Windows' notification access. Nothing leaves your PC.");
    f.group("System");
    f.toggle("System stats page", "CPU, GPU, memory, disk and network", s.stats, Action::SetToggle("stats"));
}

fn about(f: &mut Form) {
    f.group("Info");
    f.row("Nuva 1.2.0", "A living notch for Windows", 0.0);
    f.note("Music, synced lyrics, focus timer, water reminder, to-do with reminders, calendar, file shelf, clipboard history, notification mirror and system stats. A personal rebuild; all data stays in your user folder.");
    let ctl = f.row("Data folder", "Tasks, water log, shelf and clipboard", 120.0);
    f.c.button(Rect::new(ctl.x, ctl.cy() - 15.0, ctl.w, 30.0), "Open", Action::OpenDataFolder, pal::CARD_HI, pal::TEXT);
    let ctl = f.row("Lyrics provided by LRCLIB", "lrclib.net", 120.0);
    f.c.button(Rect::new(ctl.x, ctl.cy() - 15.0, ctl.w, 30.0), "Website", Action::OpenUrl("https://lrclib.net"), pal::CARD_HI, pal::TEXT);
}
