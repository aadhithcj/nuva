//! Drawing for the notch: pill, toasts, alerts and every page.

use crate::app::{App, Page, Pet, View, FLARE, WIN_W};
use crate::clipboard::ClipData;
use crate::gfx::{ico, pal, Align, Canvas, Color, Rect, W};
use crate::pomodoro::Mode;
use crate::shelf::Kind;
use crate::stats;
use crate::time;
use crate::ui::Action;

pub const PILL_H: f32 = 30.0;

fn ago(ticks: i64) -> String {
    let s = ((time::utc_ticks() - ticks) / 10_000_000).max(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl App {
    pub fn view_size(&self, v: View) -> (f32, f32, f32) {
        match v {
            View::Pill => {
                let w = if self.pill_lyric().is_some() {
                    460.0
                } else if self.pill_has_media() {
                    if self.pomo.running {
                        280.0
                    } else {
                        250.0
                    }
                } else if self.pomo.active(&self.settings) {
                    220.0
                } else if self.pet != Pet::Off {
                    190.0
                } else if self.settings.minimal {
                    128.0
                } else {
                    190.0
                };
                (w, PILL_H, 12.0)
            }
            View::Toast => (380.0, 36.0, 15.0),
            View::Alert => (450.0, 134.0, 26.0),
            View::Open(p) => {
                let (w, h) = match p {
                    Page::Home => {
                        if self.media.active && self.show_split && self.settings.lyrics {
                            (780.0, 220.0)
                        } else {
                            (540.0, 180.0)
                        }
                    }
                    Page::Timer => (460.0, 196.0),
                    Page::Water => (540.0, 206.0),
                    Page::Todo => (520.0, 270.0),
                    Page::Calendar => (580.0, 300.0),
                    Page::Shelf => (560.0, 214.0),
                    Page::Clipboard => (520.0, 290.0),
                    Page::Notifs => (520.0, 290.0),
                    Page::Stats => (580.0, 216.0),
                };
                (w, h, 26.0)
            }
        }
    }

    pub fn draw(&mut self, c: &mut Canvas, v: View) {
        if self.sh.v <= 0.5 {
            return;
        }
        let cx = WIN_W / 2.0;
        let (sw, sh, sr) = (self.sw.v, self.sh.v, self.sr.v);
        c.notch_shape(cx - sw / 2.0, cx + sw / 2.0, sh, sr, FLARE, pal::BLACK);
        let (tw, th, _) = self.view_size(v);
        let close = 1.0 - ((sw - tw).abs() / 90.0 + (sh - th).abs() / 60.0).min(1.0);
        c.alpha = (self.fade.v.clamp(0.0, 1.0) * close).clamp(0.0, 1.0);
        if c.alpha < 0.03 || sh < 4.0 {
            c.alpha = 1.0;
            return;
        }
        let area = Rect::new(cx - tw / 2.0, 0.0, tw, th);
        c.clip(Rect::new(cx - sw / 2.0, 0.0, sw, sh));
        match v {
            View::Pill => self.draw_pill(c, area),
            View::Toast => self.draw_toast(c, area),
            View::Alert => self.draw_alert(c, area),
            View::Open(p) => self.draw_open(c, area, p),
        }
        c.unclip();
        c.alpha = 1.0;
    }

    // ------------------------------------------------ pill

    fn draw_pill(&mut self, c: &mut Canvas, a: Rect) {
        let cy = a.cy();
        if self.pill_has_media() {
            let art = Rect::new(a.x + 12.0, cy - 10.0, 20.0, 20.0);
            let rt = c.rt.clone();
            match self.art(&rt) {
                Some(b) => c.image(&b, art, 5.0),
                None => c.icon(ico::MUSIC, art, 12.0, pal::DIM),
            }
            if let Some(line) = self.pill_lyric() {
                c.text(&line, Rect::new(art.r() + 10.0, 0.0, a.w - 90.0, a.h), 12.5, W::Medium, pal::TEXT, Align::Center);
            }
            if self.pomo.running {
                c.text(&time::mmss(self.pomo.left()), Rect::new(a.r() - 62.0, 0.0, 50.0, a.h), 12.5, W::Semi, pal::ORANGE, Align::Right);
            } else {
                self.eq_bars(c, a.r() - 30.0, cy);
            }
            return;
        }
        if self.pomo.active(&self.settings) {
            let col = if self.pomo.mode == Mode::Focus { pal::ORANGE } else { pal::GREEN };
            c.ring(a.x + 22.0, cy, 7.0, 2.0, pal::CARD_HI);
            c.arc(a.x + 22.0, cy, 7.0, 2.0, self.pomo.frac(&self.settings), col);
            let label = if self.pomo.running { self.pomo.mode.label() } else { "Paused" };
            c.text(label, Rect::new(a.x + 36.0, 0.0, 100.0, a.h), 11.5, W::Medium, pal::DIM, Align::Left);
            c.text(&time::mmss(self.pomo.left()), Rect::new(a.r() - 72.0, 0.0, 60.0, a.h), 13.0, W::Semi, col, Align::Right);
            return;
        }
        match self.pet {
            Pet::Off => {
                let now = time::now();
                c.text(&time::hm_short(now.minute_of_day(), self.h24), a, 12.5, W::Semi, pal::TEXT, Align::Center);
            }
            p => self.draw_mascot(c, a.cx(), cy, p),
        }
    }

    fn eq_bars(&self, c: &Canvas, x: f32, cy: f32) {
        let col = if self.media.playing { pal::ACCENT } else { pal::FAINT };
        let hs: [f32; 4] = if self.media.playing { [7.0, 12.0, 9.0, 13.0] } else { [4.0, 4.0, 4.0, 4.0] };
        for (i, h) in hs.iter().enumerate() {
            c.rrect(Rect::new(x + i as f32 * 4.5, cy - h / 2.0, 2.5, *h), 1.25, col);
        }
    }

    fn draw_mascot(&self, c: &Canvas, cx: f32, cy: f32, p: Pet) {
        let blink = self.blink_until.is_some();
        let (l, r) = (cx - 11.0, cx + 11.0);
        match p {
            Pet::Dozing => {
                for x in [l, r] {
                    c.line(x - 4.0, cy + 1.0, x + 4.0, cy + 1.0, pal::DIM, 2.0);
                }
                c.text("z", Rect::new(cx + 22.0, cy - 13.0, 12.0, 12.0), 10.0, W::Semi, pal::DIM, Align::Center);
                c.text("z", Rect::new(cx + 30.0, cy - 19.0, 12.0, 12.0), 8.0, W::Semi, pal::FAINT, Align::Center);
            }
            Pet::Waving => {
                for x in [l, r] {
                    c.polyline(&[(x - 4.0, cy + 2.0), (x, cy - 3.0), (x + 4.0, cy + 2.0)], pal::TEXT, 2.0);
                }
                // A little hand waving beside the face.
                let t = (time::utc_ticks() / 10_000 % 600) as f32 / 600.0;
                let ang = (t * std::f32::consts::TAU).sin() * 0.5;
                let (hx, hy) = (cx + 28.0, cy + 2.0);
                c.line(hx, hy + 5.0, hx + 6.0 * ang.sin(), hy - 5.0 * ang.cos(), pal::YELLOW, 3.0);
            }
            _ => {
                for x in [l, r] {
                    if blink {
                        c.line(x - 4.0, cy, x + 4.0, cy, pal::TEXT, 2.0);
                    } else {
                        c.ellipse(x, cy, 3.6, 5.2, pal::TEXT);
                        c.circle(x + 1.0, cy - 1.6, 1.1, pal::BLACK);
                    }
                }
            }
        }
    }

    // ------------------------------------------------ toast & alert

    fn draw_toast(&mut self, c: &mut Canvas, a: Rect) {
        let Some(t) = &self.toast else { return };
        let cy = a.cy();
        c.icon(t.icon, Rect::new(a.x + 12.0, 0.0, 26.0, a.h), 15.0, t.color);
        c.text(&t.title, Rect::new(a.x + 44.0, 0.0, 150.0, a.h), 12.5, W::Semi, pal::TEXT, Align::Left);
        if let Some(level) = t.level {
            let bar = Rect::new(a.r() - 168.0, cy - 2.5, 120.0, 5.0);
            c.bar(bar, level, pal::CARD_HI, if t.color == pal::WHITE { pal::TEXT } else { t.color });
            c.text(&t.sub, Rect::new(a.r() - 44.0, 0.0, 32.0, a.h), 11.5, W::Medium, pal::DIM, Align::Right);
        } else {
            c.text(&t.sub, Rect::new(a.x + 190.0, 0.0, a.w - 204.0, a.h), 11.5, W::Regular, pal::DIM, Align::Right);
        }
    }

    fn draw_alert(&mut self, c: &mut Canvas, a: Rect) {
        let Some(al) = self.alerts.front() else { return };
        let (icon, color, title, sub) = (al.icon, al.color, al.title.clone(), al.sub.clone());
        let buttons: Vec<String> = al.buttons.iter().map(|b| b.0.clone()).collect();
        let inner = a.inset(22.0, 0.0);
        let circle = (inner.x + 24.0, 58.0);
        c.circle(circle.0, circle.1, 24.0, color.a(0.18));
        c.icon(icon, Rect::new(circle.0 - 24.0, circle.1 - 24.0, 48.0, 48.0), 20.0, color);
        c.text(&title, Rect::new(inner.x + 62.0, 34.0, inner.w - 62.0, 24.0), 17.0, W::Semi, pal::TEXT, Align::Left);
        c.text(&sub, Rect::new(inner.x + 62.0, 58.0, inner.w - 62.0, 20.0), 12.5, W::Regular, pal::DIM, Align::Left);
        let mut x = inner.r();
        for (i, label) in buttons.iter().enumerate().rev() {
            let w = c.measure(label, 12.5, W::Semi) + 30.0;
            x -= w;
            let r = Rect::new(x, a.b() - 44.0, w, 30.0);
            let fill = if i == 0 { color } else { pal::CARD_HI };
            let fg = if i == 0 { pal::BLACK } else { pal::TEXT };
            c.button(r, label, Action::AlertButton(i), fill, fg);
            x -= 8.0;
        }
        if self.alerts.len() > 1 {
            c.text(&format!("+{} more", self.alerts.len() - 1), Rect::new(inner.x, a.b() - 44.0, 120.0, 30.0), 11.0, W::Regular, pal::FAINT, Align::Left);
        }
    }

    // ------------------------------------------------ open notch frame

    fn draw_open(&mut self, c: &mut Canvas, a: Rect, page: Page) {
        // Header
        let hdr = Rect::new(a.x + 22.0, 4.0, a.w - 44.0, 26.0);
        c.icon(page.icon(), Rect::new(hdr.x, hdr.y, 16.0, hdr.h), 11.0, pal::DIM);
        c.text(page.title(), Rect::new(hdr.x + 22.0, hdr.y, 160.0, hdr.h), 12.0, W::Semi, pal::DIM, Align::Left);
        let now = time::now();
        let right = Rect::new(hdr.r() - 140.0, hdr.y, 112.0, hdr.h);
        c.text(&time::hm(now.minute_of_day(), self.h24), right, 12.0, W::Medium, pal::DIM, Align::Right);
        c.icon_button(Rect::new(hdr.r() - 22.0, hdr.y + 2.0, 22.0, 22.0), ico::SETTINGS, 10.5, Action::OpenSettings, pal::DIM);

        let content = Rect::new(a.x + 20.0, 36.0, a.w - 40.0, a.h - 36.0 - 28.0);
        match page {
            Page::Home => {
                if self.media.active {
                    self.draw_media(c, content);
                } else {
                    self.draw_home(c, content);
                }
            }
            Page::Timer => self.draw_timer(c, content),
            Page::Water => self.draw_water(c, content),
            Page::Todo => self.draw_todo(c, content),
            Page::Calendar => self.draw_calendar(c, content),
            Page::Shelf => self.draw_shelf(c, content),
            Page::Clipboard => self.draw_clipboard(c, content),
            Page::Notifs => self.draw_notifs(c, content),
            Page::Stats => self.draw_stats(c, content),
        }
        self.draw_dots(c, a, page);
    }

    fn draw_dots(&self, c: &mut Canvas, a: Rect, page: Page) {
        let pages = self.pages();
        let row = Rect::new(a.x + 40.0, a.b() - 26.0, a.w - 80.0, 22.0);
        let icons = self.settings.page_icons && c.hover(row);
        let step = if icons { 26.0 } else { 14.0 };
        let total = step * pages.len() as f32;
        let mut x = row.cx() - total / 2.0;
        for (i, p) in pages.iter().enumerate() {
            let cell = Rect::new(x, row.y, step, row.h);
            let hov = c.hit(cell, Action::Page(i));
            let active = *p == page;
            if icons {
                let col = if active { pal::TEXT } else if hov { pal::DIM } else { pal::FAINT };
                c.icon(p.icon(), cell, 11.0, col);
            } else if active {
                c.rrect(Rect::new(cell.cx() - 7.0, cell.cy() - 3.0, 14.0, 6.0), 3.0, pal::TEXT);
            } else {
                c.circle(cell.cx(), cell.cy(), 3.0, if hov { pal::DIM } else { pal::FAINT });
            }
            x += step;
        }
    }

    // ------------------------------------------------ home / media

    fn draw_home(&mut self, c: &mut Canvas, r: Rect) {
        let now = time::now();
        c.text(&time::hm(now.minute_of_day(), self.h24), Rect::new(r.x, r.y + 2.0, 220.0, 46.0), 36.0, W::Semi, pal::TEXT, Align::Left);
        let day = now.day_num();
        let date = format!("{}, {} {}", ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][time::weekday(day) as usize], now.day, time::MONTHS[now.month as usize - 1]);
        c.text(&date, Rect::new(r.x, r.y + 50.0, 220.0, 20.0), 13.0, W::Regular, pal::DIM, Align::Left);
        c.text("Nothing playing", Rect::new(r.x, r.y + 74.0, 220.0, 18.0), 11.5, W::Regular, pal::FAINT, Align::Left);

        // Summary cards
        let cw = 92.0;
        let mut x = r.r() - cw * 3.0 - 16.0;
        let cards = [Page::Todo, Page::Water, Page::Timer];
        for p in cards {
            let card = Rect::new(x, r.y + 2.0, cw, r.h - 6.0);
            let pages = self.pages();
            let idx = pages.iter().position(|q| *q == p);
            if idx.is_none() {
                x += cw + 8.0;
                continue;
            }
            let hov = c.hit(card, Action::Page(idx.unwrap()));
            c.rrect(card, 14.0, if hov { pal::CARD_HI } else { pal::CARD });
            let inner = card.inset(10.0, 8.0);
            match p {
                Page::Todo => {
                    c.icon(ico::CHECKLIST, Rect::new(inner.x, inner.y, 14.0, 16.0), 11.0, pal::ACCENT);
                    let tasks = self.todo.for_day(day);
                    let next = tasks.iter().find(|t| !t.done);
                    let (v, s) = match next {
                        Some(t) => (t.title.clone(), time::hm(t.start, self.h24)),
                        None => ("All clear".to_string(), format!("{} done", self.todo.done_today())),
                    };
                    c.text("Next", Rect::new(inner.x + 18.0, inner.y, 60.0, 16.0), 10.5, W::Medium, pal::DIM, Align::Left);
                    c.text_wrap(&v, Rect::new(inner.x, inner.y + 22.0, inner.w, 34.0), 12.0, W::Semi, pal::TEXT, Align::Left);
                    c.text(&s, Rect::new(inner.x, inner.b() - 16.0, inner.w, 16.0), 10.5, W::Regular, pal::DIM, Align::Left);
                }
                Page::Water => {
                    let frac = self.water.taken as f32 / self.water.goal.max(1) as f32;
                    let (ccx, ccy) = (inner.cx(), inner.y + 30.0);
                    c.ring(ccx, ccy, 22.0, 4.0, pal::CARD_HI);
                    c.arc(ccx, ccy, 22.0, 4.0, frac, pal::WATER);
                    c.icon(ico::DROP, Rect::new(ccx - 12.0, ccy - 12.0, 24.0, 24.0), 11.0, pal::WATER);
                    let s = format!("{:.1} / {:.1} L", self.water.taken as f32 / 1000.0, self.water.goal as f32 / 1000.0);
                    c.text(&s, Rect::new(inner.x, inner.b() - 16.0, inner.w, 16.0), 10.5, W::Medium, pal::DIM, Align::Center);
                }
                _ => {
                    let col = if self.pomo.mode == Mode::Focus { pal::ORANGE } else { pal::GREEN };
                    c.icon(ico::TIMER, Rect::new(inner.x, inner.y, 14.0, 16.0), 11.0, col);
                    c.text(self.pomo.mode.label(), Rect::new(inner.x + 18.0, inner.y, inner.w - 18.0, 16.0), 10.5, W::Medium, pal::DIM, Align::Left);
                    c.text(&time::mmss(self.pomo.left()), Rect::new(inner.x, inner.y + 22.0, inner.w, 28.0), 20.0, W::Semi, pal::TEXT, Align::Left);
                    let s = if self.pomo.running { "Running" } else if self.pomo.active(&self.settings) { "Paused" } else { "Ready" };
                    c.text(s, Rect::new(inner.x, inner.b() - 16.0, inner.w, 16.0), 10.5, W::Regular, pal::DIM, Align::Left);
                }
            }
            x += cw + 8.0;
        }
    }

    fn draw_media(&mut self, c: &mut Canvas, r: Rect) {
        let split = self.show_split && self.settings.lyrics;
        let (left, right) = if split { r.split_left(360.0) } else { (r, Rect::default()) };
        let art = Rect::new(left.x, left.y + 2.0, 104.0, 104.0).inset(0.0, 0.0);
        let art = if left.h < 112.0 { Rect::new(art.x, art.y, left.h - 6.0, left.h - 6.0) } else { art };
        let rt = c.rt.clone();
        match self.art(&rt) {
            Some(b) => c.image(&b, art, 14.0),
            None => {
                c.rrect(art, 14.0, pal::CARD);
                c.icon(ico::MUSIC, art, 26.0, pal::FAINT);
            }
        }
        let x = art.r() + 16.0;
        let w = left.r() - x;
        let m = self.media.clone();
        c.text(if m.title.is_empty() { "Unknown title" } else { &m.title }, Rect::new(x, left.y + 2.0, w - 30.0, 22.0), 15.5, W::Semi, pal::TEXT, Align::Left);
        c.text(&m.artist, Rect::new(x, left.y + 24.0, w - 30.0, 18.0), 12.5, W::Regular, pal::DIM, Align::Left);
        if !m.app.is_empty() {
            c.text(&m.app, Rect::new(x, left.y + 42.0, w, 16.0), 10.5, W::Regular, pal::FAINT, Align::Left);
        }
        if self.settings.lyrics {
            let lr = Rect::new(left.r() - 28.0, left.y, 28.0, 28.0);
            let col = if split { pal::ACCENT } else { pal::DIM };
            c.icon_button(lr, ico::MIC, 12.0, Action::ToggleLyrics, col);
        }

        // Progress
        let py = left.y + 66.0;
        let pos = self.seeking.unwrap_or_else(|| m.position());
        let frac = if m.dur > 0.0 { (pos / m.dur) as f32 } else { 0.0 };
        let bar = Rect::new(x, py, w, 4.0);
        if m.can_seek && m.dur > 0.0 {
            c.hit(Rect::new(x, py - 6.0, w, 16.0), Action::SeekBar);
        }
        c.bar(bar, frac, pal::CARD_HI, pal::TEXT.a(0.9));
        if m.dur > 0.0 {
            c.text(&time::mmss(pos), Rect::new(x, py + 6.0, 60.0, 14.0), 10.0, W::Regular, pal::FAINT, Align::Left);
            c.text(&format!("-{}", time::mmss(m.dur - pos)), Rect::new(x + w - 60.0, py + 6.0, 60.0, 14.0), 10.0, W::Regular, pal::FAINT, Align::Right);
        }

        // Controls
        let cy = (py + 20.0 + left.b()) / 2.0 + 2.0;
        let mid = x + w / 2.0;
        c.icon_button(Rect::new(mid - 64.0, cy - 16.0, 32.0, 32.0), ico::PREV, 14.0, Action::Prev, pal::TEXT);
        c.icon_button(Rect::new(mid - 19.0, cy - 19.0, 38.0, 38.0), if m.playing { ico::PAUSE } else { ico::PLAY }, 18.0, Action::PlayPause, pal::TEXT);
        c.icon_button(Rect::new(mid + 32.0, cy - 16.0, 32.0, 32.0), ico::NEXT, 14.0, Action::Next, pal::TEXT);

        if split {
            self.draw_lyrics(c, Rect::new(right.x + 18.0, right.y - 6.0, right.w - 18.0, right.h + 10.0));
        }
    }

    fn draw_lyrics(&mut self, c: &mut Canvas, r: Rect) {
        let Some(l) = self.lyrics.clone() else {
            let msg = if self.lyrics_loading { "Loading lyrics" } else { "Lyrics not available" };
            c.text(msg, r, 12.5, W::Regular, pal::FAINT, Align::Center);
            return;
        };
        c.clip(r);
        let lh = 26.0;
        if l.synced {
            let pos = self.media.position();
            let cur = l.current(pos);
            self.lyric_scroll.set(cur.unwrap_or(0) as f32);
            let s = self.lyric_scroll.v;
            for (i, line) in l.lines.iter().enumerate() {
                let y = r.cy() + (i as f32 - s) * lh - lh / 2.0;
                if y < r.y - lh || y > r.b() {
                    continue;
                }
                let row = Rect::new(r.x, y, r.w, lh);
                let d = (i as f32 - s).abs();
                let text = if line.text.is_empty() { "♪" } else { &line.text };
                let hov = c.hit(row, Action::LyricSeek(line.t));
                if Some(i) == cur {
                    c.text(text, row, 15.0, W::Semi, pal::TEXT, Align::Left);
                } else {
                    let a = (1.0 - d * 0.22).clamp(0.18, 0.7);
                    c.text(text, row, 13.0, W::Regular, if hov { pal::TEXT.a(0.85) } else { pal::TEXT.a(a) }, Align::Left);
                }
            }
        } else {
            let off = *self.scroll.get(&Page::Home).unwrap_or(&0.0);
            for (i, line) in l.lines.iter().enumerate() {
                let y = r.y + i as f32 * 20.0 - off;
                if y < r.y - 20.0 || y > r.b() {
                    continue;
                }
                c.text(&line.text, Rect::new(r.x, y, r.w, 20.0), 12.5, W::Regular, pal::DIM, Align::Left);
            }
        }
        c.unclip();
    }

    // ------------------------------------------------ timer

    fn draw_timer(&mut self, c: &mut Canvas, r: Rect) {
        let col = if self.pomo.mode == Mode::Focus { pal::ORANGE } else { pal::GREEN };
        let (ccx, ccy, rad) = (r.x + 62.0, r.cy(), 54.0);
        c.ring(ccx, ccy, rad, 7.0, pal::CARD_HI);
        c.arc(ccx, ccy, rad, 7.0, self.pomo.frac(&self.settings), col);
        c.text(&time::mmss(self.pomo.left()), Rect::new(ccx - 50.0, ccy - 18.0, 100.0, 30.0), 24.0, W::Semi, pal::TEXT, Align::Center);
        c.text(self.pomo.mode.label(), Rect::new(ccx - 50.0, ccy + 12.0, 100.0, 16.0), 11.0, W::Regular, pal::DIM, Align::Center);

        let x = r.x + 150.0;
        let w = r.r() - x;
        let sel = match self.pomo.mode {
            Mode::Focus => 0,
            Mode::Short => 1,
            Mode::Long => 2,
        };
        c.segmented(Rect::new(x, r.y + 4.0, w, 30.0), &["Focus", "Short break", "Long break"], sel, &|i| Action::PomoMode(i as u8));
        let by = r.y + 48.0;
        let label = if self.pomo.running { "Pause" } else if self.pomo.active(&self.settings) { "Resume" } else { "Start" };
        c.button(Rect::new(x, by, 104.0, 34.0), label, Action::PomoStartPause, col, pal::BLACK);
        c.button(Rect::new(x + 112.0, by, 84.0, 34.0), "Reset", Action::PomoReset, pal::CARD_HI, pal::TEXT);
        c.button(Rect::new(x + 204.0, by, 84.0, 34.0), "Skip", Action::PomoSkip, pal::CARD_HI, pal::TEXT);
        let done = self.pomo.done_focus % 4;
        c.text(&format!("Session {} of 4", done + 1), Rect::new(x, r.b() - 22.0, 120.0, 18.0), 11.0, W::Regular, pal::DIM, Align::Left);
        for i in 0..4 {
            let filled = i < done;
            let dx = x + 110.0 + i as f32 * 14.0;
            if filled {
                c.circle(dx, r.b() - 13.0, 4.0, col);
            } else {
                c.ring(dx, r.b() - 13.0, 3.5, 1.2, pal::FAINT);
            }
        }
    }

    // ------------------------------------------------ water

    fn draw_water(&mut self, c: &mut Canvas, r: Rect) {
        let wtr = &self.water;
        let frac = wtr.taken as f32 / wtr.goal.max(1) as f32;
        let (ccx, ccy, rad) = (r.x + 56.0, r.cy(), 50.0);
        c.ring(ccx, ccy, rad, 7.0, pal::CARD_HI);
        c.arc(ccx, ccy, rad, 7.0, frac, pal::WATER);
        c.text(&format!("{:.1} L", wtr.taken as f32 / 1000.0), Rect::new(ccx - 44.0, ccy - 16.0, 88.0, 26.0), 20.0, W::Semi, pal::TEXT, Align::Center);
        c.text(&format!("of {:.1} L", wtr.goal as f32 / 1000.0), Rect::new(ccx - 44.0, ccy + 10.0, 88.0, 16.0), 11.0, W::Regular, pal::DIM, Align::Center);

        let x = r.x + 132.0;
        let next = if !self.settings.water {
            "Reminders off".to_string()
        } else if wtr.taken >= wtr.goal {
            "Goal reached today".to_string()
        } else {
            match wtr.next_reminder() {
                Some(m) => format!("Next {} · {} ml each", time::hm(m, self.h24), wtr.per()),
                None => "Done for today".to_string(),
            }
        };
        c.text("Today's progress", Rect::new(x, r.y + 2.0, 200.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        c.text(&next, Rect::new(x, r.y + 20.0, 220.0, 18.0), 12.5, W::Regular, pal::TEXT, Align::Left);
        let left_in_bottle = wtr.bottle.saturating_sub(wtr.since_refill);
        c.text(&format!("Bottle · {} ml left", left_in_bottle), Rect::new(x, r.y + 44.0, 200.0, 16.0), 11.0, W::Regular, pal::DIM, Align::Left);
        c.bar(Rect::new(x, r.y + 64.0, 200.0, 5.0), left_in_bottle as f32 / wtr.bottle.max(1) as f32, pal::CARD_HI, pal::WATER.a(0.7));
        let by = r.b() - 36.0;
        let per = wtr.per();
        let can_undo = wtr.can_undo();
        c.button(Rect::new(x, by, 110.0, 32.0), &format!("Drank {per} ml"), Action::WaterDrank, pal::WATER, pal::BLACK);
        c.button(Rect::new(x + 118.0, by, 80.0, 32.0), "Refilled", Action::WaterRefilled, pal::CARD_HI, pal::TEXT);
        if can_undo {
            c.icon_button(Rect::new(x + 204.0, by + 1.0, 30.0, 30.0), '\u{E7A7}', 12.0, Action::WaterUndo, pal::DIM);
        }

        // 7-day chart
        let chart = Rect::new(r.r() - 160.0, r.y + 4.0, 160.0, r.h - 8.0);
        let week = self.water.week();
        let goal = self.water.goal.max(1) as f32;
        let bw = 14.0;
        let gap = (chart.w - bw * 7.0) / 6.0;
        let ch = chart.h - 22.0;
        for (i, (d, ml)) in week.iter().enumerate() {
            let bx = chart.x + i as f32 * (bw + gap);
            let f = (*ml as f32 / goal).min(1.2) / 1.2;
            c.rrect(Rect::new(bx, chart.y, bw, ch), 5.0, pal::CARD);
            if f > 0.0 {
                let hh = (ch * f).max(6.0);
                c.rrect(Rect::new(bx, chart.y + ch - hh, bw, hh), 5.0, if i == 6 { pal::WATER } else { pal::WATER.a(0.45) });
            }
            c.text(time::DAYS_LETTER[time::weekday(*d) as usize], Rect::new(bx - 4.0, chart.b() - 16.0, bw + 8.0, 16.0), 10.0, W::Medium, if i == 6 { pal::TEXT } else { pal::FAINT }, Align::Center);
        }
    }

    // ------------------------------------------------ to-do

    fn draw_todo(&mut self, c: &mut Canvas, r: Rect) {
        if self.editor.is_some() {
            self.draw_editor(c, r);
            return;
        }
        let today = time::today();
        let tasks = self.todo.for_day(today);
        let done = tasks.iter().filter(|t| t.done).count();
        c.text("Today", Rect::new(r.x, r.y, 80.0, 22.0), 14.0, W::Semi, pal::TEXT, Align::Left);
        c.text(&format!("{done} of {} done", tasks.len()), Rect::new(r.x + 56.0, r.y, 140.0, 22.0), 11.5, W::Regular, pal::DIM, Align::Left);
        c.button(Rect::new(r.r() - 96.0, r.y - 2.0, 96.0, 26.0), "+ Add Task", Action::TodoAdd, pal::ACCENT, pal::BLACK);
        let list = Rect::new(r.x, r.y + 30.0, r.w, r.h - 30.0);
        if tasks.is_empty() {
            c.text("No tasks.", list, 12.5, W::Regular, pal::FAINT, Align::Center);
            return;
        }
        let rh = 32.0;
        let max_scroll = (tasks.len() as f32 * rh - list.h).max(0.0);
        let off = self.scroll.entry(Page::Todo).or_insert(0.0);
        *off = off.min(max_scroll);
        let off = *off;
        c.clip(list);
        let now = time::now().minute_of_day();
        for (i, t) in tasks.iter().enumerate() {
            let row = Rect::new(list.x, list.y + i as f32 * rh - off, list.w, rh - 2.0);
            if row.b() < list.y || row.y > list.b() {
                continue;
            }
            let hov = c.hover(row);
            if hov {
                c.rrect(row, 10.0, pal::CARD);
            }
            let cb = Rect::new(row.x + 6.0, row.y + 6.0, 18.0, 18.0);
            c.hit(cb.inset(-4.0, -4.0), Action::TodoToggle(t.id));
            if t.done {
                c.circle(cb.cx(), cb.cy(), 9.0, pal::GREEN);
                c.icon(ico::CHECK, cb, 9.0, pal::BLACK);
            } else {
                let current = now >= t.start && now < t.end().max(t.start + 1);
                c.ring(cb.cx(), cb.cy(), 8.0, 1.5, if current { pal::ACCENT } else { pal::FAINT });
            }
            let span = if t.dur > 0 { format!("{}–{}", time::hm(t.start, self.h24), time::hm(t.end() % 1440, self.h24)) } else { time::hm(t.start, self.h24) };
            c.text(&span, Rect::new(row.x + 34.0, row.y, 120.0, row.h), 11.5, W::Regular, pal::DIM, Align::Left);
            let tc = if t.done { pal::FAINT } else { pal::TEXT };
            c.text(&t.title, Rect::new(row.x + 154.0, row.y, row.w - 154.0 - if hov { 64.0 } else { 8.0 }, row.h), 12.5, W::Medium, tc, Align::Left);
            if hov {
                c.icon_button(Rect::new(row.r() - 60.0, row.y + 3.0, 26.0, 24.0), ico::EDIT, 10.5, Action::TodoEdit(t.id), pal::DIM);
                c.icon_button(Rect::new(row.r() - 30.0, row.y + 3.0, 26.0, 24.0), ico::DELETE, 10.5, Action::TodoDelete(t.id), pal::DIM);
            }
        }
        c.unclip();
    }

    fn draw_editor(&mut self, c: &mut Canvas, r: Rect) {
        let Some(e) = &self.editor else { return };
        let (title, day, start, dur, focused, editing) = (e.title.clone(), e.day, e.start, e.dur, e.focused, e.id.is_some());
        c.text(if editing { "Edit Task" } else { "Add Task" }, Rect::new(r.x, r.y, 200.0, 22.0), 14.0, W::Semi, pal::TEXT, Align::Left);
        let field = Rect::new(r.x, r.y + 30.0, r.w, 38.0);
        c.hit(field, Action::TodoField);
        c.rrect(field, 10.0, pal::CARD);
        if focused {
            c.rrect_stroke(field, 10.0, pal::ACCENT, 1.5);
        }
        let tr = field.inset(12.0, 0.0);
        if title.is_empty() {
            c.text("Task title", tr, 13.0, W::Regular, pal::FAINT, Align::Left);
        } else {
            c.text(&title, tr, 13.0, W::Regular, pal::TEXT, Align::Left);
        }
        if focused {
            let cw = c.measure(&title, 13.0, W::Regular).min(tr.w - 2.0);
            c.line(tr.x + cw + 1.0, field.y + 10.0, tr.x + cw + 1.0, field.b() - 10.0, pal::ACCENT, 1.5);
        }
        let ry = r.y + 82.0;
        let col = (r.w - 24.0) / 3.0;
        let labels = ["Day", "Starts", "Duration"];
        for (i, l) in labels.iter().enumerate() {
            c.text(l, Rect::new(r.x + i as f32 * (col + 12.0), ry, col, 16.0), 10.5, W::Medium, pal::DIM, Align::Left);
        }
        let sy = ry + 20.0;
        c.stepper(Rect::new(r.x, sy, col, 32.0), &time::day_label(day), Action::TodoDay(-1), Action::TodoDay(1));
        c.stepper(Rect::new(r.x + col + 12.0, sy, col, 32.0), &time::hm(start, self.h24), Action::TodoStart(-15), Action::TodoStart(15));
        let dl = if dur == 0 { "No end".to_string() } else if dur % 60 == 0 { format!("{} h", dur / 60) } else if dur > 60 { format!("{} h {} min", dur / 60, dur % 60) } else { format!("{dur} min") };
        c.stepper(Rect::new(r.x + 2.0 * (col + 12.0), sy, col, 32.0), &dl, Action::TodoDur(-5), Action::TodoDur(5));
        let by = r.b() - 34.0;
        c.button(Rect::new(r.r() - 196.0, by, 92.0, 32.0), "Cancel", Action::TodoCancel, pal::CARD_HI, pal::TEXT);
        c.button(Rect::new(r.r() - 96.0, by, 96.0, 32.0), "Save", Action::TodoSave, pal::ACCENT, pal::BLACK);
        c.text("Enter to save · Esc to cancel", Rect::new(r.x, by, 220.0, 32.0), 10.5, W::Regular, pal::FAINT, Align::Left);
    }

    // ------------------------------------------------ calendar

    fn draw_calendar(&mut self, c: &mut Canvas, r: Rect) {
        let today = time::today();
        let gw = 320.0;
        let grid = Rect::new(r.x, r.y, gw, r.h);
        let (y, m, _) = time::civil_from_days(if self.cal_week { self.cal_anchor + 3 } else { self.cal_anchor });
        c.text(&format!("{} {}", time::MONTHS[m as usize - 1], y), Rect::new(grid.x, grid.y, 200.0, 24.0), 14.0, W::Semi, pal::TEXT, Align::Left);
        c.icon_button(Rect::new(grid.r() - 56.0, grid.y, 26.0, 24.0), ico::LEFT, 10.0, Action::CalShift(-1), pal::DIM);
        c.icon_button(Rect::new(grid.r() - 28.0, grid.y, 26.0, 24.0), ico::RIGHT, 10.0, Action::CalShift(1), pal::DIM);
        let cell_w = gw / 7.0;
        let first_wd = if self.settings.week_monday { 1 } else { 0 };
        for i in 0..7 {
            let wd = (i + first_wd) % 7;
            c.text(time::DAYS_LETTER[wd], Rect::new(grid.x + i as f32 * cell_w, grid.y + 28.0, cell_w, 16.0), 10.0, W::Medium, pal::FAINT, Align::Center);
        }
        let top = grid.y + 46.0;
        let (start, rows) = if self.cal_week { (self.cal_anchor, 1) } else { (self.week_start(self.cal_anchor), 6) };
        let cell_h = if self.cal_week { 64.0 } else { (grid.b() - top) / 6.0 };
        for row in 0..rows {
            for col in 0..7 {
                let day = start + (row * 7 + col) as i64;
                let cell = Rect::new(grid.x + col as f32 * cell_w, top + row as f32 * cell_h, cell_w, cell_h);
                let (_, dm, dd) = time::civil_from_days(day);
                let hov = c.hit(cell, Action::CalDay(day));
                let in_month = self.cal_week || dm == m;
                let center = (cell.cx(), cell.y + cell_h.min(30.0) / 2.0);
                if day == today {
                    c.circle(center.0, center.1, 12.0, pal::ACCENT);
                } else if day == self.cal_sel {
                    c.ring(center.0, center.1, 11.5, 1.5, pal::DIM);
                } else if hov {
                    c.circle(center.0, center.1, 12.0, pal::WHITE.a(0.08));
                }
                let col_txt = if day == today { pal::BLACK } else if in_month { pal::TEXT } else { pal::FAINT };
                c.text(&dd.to_string(), Rect::new(cell.x, center.1 - 10.0, cell.w, 20.0), 12.0, W::Medium, col_txt, Align::Center);
                if self.todo.has_tasks(day) {
                    c.circle(center.0, center.1 + 15.0, 2.0, if day == today { pal::TEXT } else { pal::ACCENT });
                }
                if self.cal_week {
                    let n = self.todo.for_day(day).len();
                    if n > 0 {
                        c.text(&format!("{n} task{}", if n == 1 { "" } else { "s" }), Rect::new(cell.x, cell.y + 40.0, cell.w, 16.0), 9.5, W::Regular, pal::DIM, Align::Center);
                    }
                }
            }
        }

        // Side panel: the selected day's tasks.
        let side = Rect::new(r.x + gw + 24.0, r.y, r.w - gw - 24.0, r.h);
        c.segmented(Rect::new(side.x, side.y, 140.0, 26.0), &["Month", "Week"], if self.cal_week { 1 } else { 0 }, &|i| Action::CalView(i == 1));
        c.icon_button(Rect::new(side.r() - 26.0, side.y, 26.0, 26.0), ico::ADD, 11.0, Action::TodoAdd, pal::TEXT);
        c.text(&time::day_label(self.cal_sel), Rect::new(side.x, side.y + 36.0, side.w, 20.0), 13.0, W::Semi, pal::TEXT, Align::Left);
        let tasks = self.todo.for_day(self.cal_sel);
        if tasks.is_empty() {
            c.text("Nothing scheduled", Rect::new(side.x, side.y + 60.0, side.w, 20.0), 11.5, W::Regular, pal::FAINT, Align::Left);
        }
        for (i, t) in tasks.iter().take(6).enumerate() {
            let ry = side.y + 62.0 + i as f32 * 28.0;
            c.rrect(Rect::new(side.x, ry + 4.0, 3.0, 20.0), 1.5, if t.done { pal::FAINT } else { pal::ACCENT });
            c.text(&time::hm(t.start, self.h24), Rect::new(side.x + 10.0, ry, 70.0, 28.0), 11.0, W::Regular, pal::DIM, Align::Left);
            c.text(&t.title, Rect::new(side.x + 80.0, ry, side.w - 80.0, 28.0), 12.0, W::Medium, if t.done { pal::FAINT } else { pal::TEXT }, Align::Left);
        }
    }

    // ------------------------------------------------ shelf

    fn draw_shelf(&mut self, c: &mut Canvas, r: Rect) {
        let n = self.shelf.items.len();
        if n == 0 || self.drag_over {
            let zone = if n == 0 { r.inset(0.0, 2.0) } else { r.inset(0.0, 2.0) };
            c.rrect(zone, 16.0, if self.drag_over { pal::ACCENT.a(0.12) } else { pal::CARD });
            c.rrect_stroke(zone, 16.0, if self.drag_over { pal::ACCENT } else { pal::LINE }, 1.5);
            let title = if self.drag_over { "Release to add" } else { "Drop files here" };
            c.icon(ico::FOLDER, Rect::new(zone.x, zone.cy() - 40.0, zone.w, 30.0), 20.0, if self.drag_over { pal::ACCENT } else { pal::DIM });
            c.text(title, Rect::new(zone.x, zone.cy() - 8.0, zone.w, 22.0), 14.0, W::Semi, pal::TEXT, Align::Center);
            c.text("Drag out anytime · click to open or copy", Rect::new(zone.x, zone.cy() + 14.0, zone.w, 18.0), 11.0, W::Regular, pal::DIM, Align::Center);
            if n == 0 {
                return;
            }
            return;
        }
        c.text(&format!("{n} item{}", if n == 1 { "" } else { "s" }), Rect::new(r.x, r.y, 120.0, 20.0), 11.5, W::Medium, pal::DIM, Align::Left);
        c.button(Rect::new(r.r() - 72.0, r.y - 2.0, 72.0, 24.0), "Clear all", Action::ShelfClear, pal::CARD_HI, pal::TEXT);
        let area = Rect::new(r.x, r.y + 26.0, r.w, r.h - 26.0);
        let (tw, gap) = (88.0, 8.0);
        let max_scroll = (n as f32 * (tw + gap) - area.w).max(0.0);
        let off = self.scroll.entry(Page::Shelf).or_insert(0.0);
        *off = off.min(max_scroll);
        let off = *off;
        c.clip(area);
        let rt = c.rt.clone();
        for i in 0..n {
            let tile = Rect::new(area.x + i as f32 * (tw + gap) - off, area.y, tw, area.h);
            if tile.r() < area.x || tile.x > area.r() {
                continue;
            }
            let hov = c.hit(tile, Action::ShelfItem(i));
            if hov {
                c.rrect(tile, 12.0, pal::CARD);
            }
            let it = self.shelf.items[i].clone();
            let icon = Rect::new(tile.cx() - 24.0, tile.y + 8.0, 48.0, 48.0);
            let bmp = if it.kind == Kind::File { self.shelf_icon(&rt, &it.path) } else { None };
            match bmp {
                Some(b) => c.image(&b, icon, 0.0),
                None => {
                    c.rrect(icon.inset(4.0, 2.0), 8.0, pal::CARD_HI);
                    let g = match it.kind {
                        Kind::Text => ico::TEXT,
                        Kind::Link => ico::LINK,
                        Kind::File => ico::FOLDER,
                    };
                    c.icon(g, icon, 16.0, pal::DIM);
                }
            }
            c.text_wrap(&it.label, Rect::new(tile.x + 4.0, icon.b() + 6.0, tile.w - 8.0, 30.0), 10.5, W::Regular, pal::TEXT, Align::Center);
            if hov {
                c.icon_button(Rect::new(tile.r() - 24.0, tile.y + 2.0, 22.0, 22.0), ico::CLOSE, 8.0, Action::ShelfRemove(i), pal::DIM);
            }
        }
        c.unclip();
    }

    // ------------------------------------------------ clipboard

    fn draw_clipboard(&mut self, c: &mut Canvas, r: Rect) {
        let n = self.clip.items.len();
        c.text(&format!("{n} recent"), Rect::new(r.x, r.y, 120.0, 20.0), 11.5, W::Medium, pal::DIM, Align::Left);
        if n > 0 {
            c.button(Rect::new(r.r() - 60.0, r.y - 2.0, 60.0, 24.0), "Clear", Action::ClipClear, pal::CARD_HI, pal::TEXT);
        }
        let list = Rect::new(r.x, r.y + 26.0, r.w, r.h - 26.0);
        if n == 0 {
            c.text("Copy something and it shows up here", list, 12.5, W::Regular, pal::FAINT, Align::Center);
            return;
        }
        let rh = 38.0;
        let max_scroll = (n as f32 * rh - list.h).max(0.0);
        let off = self.scroll.entry(Page::Clipboard).or_insert(0.0);
        *off = off.min(max_scroll);
        let off = *off;
        c.clip(list);
        let rt = c.rt.clone();
        for i in 0..n {
            let row = Rect::new(list.x, list.y + i as f32 * rh - off, list.w, rh - 3.0);
            if row.b() < list.y || row.y > list.b() {
                continue;
            }
            let hov = c.hit(row, Action::ClipCopy(i));
            c.rrect(row, 10.0, if hov { pal::CARD_HI } else { pal::CARD });
            let pinned = self.clip.items[i].pinned;
            let tx = row.x + 12.0;
            let tw = row.w - 24.0 - if hov { 60.0 } else { 0.0 } - if pinned { 18.0 } else { 0.0 };
            match &self.clip.items[i].data {
                ClipData::Text(t) => {
                    let s = one_line(t);
                    c.text(&s, Rect::new(tx, row.y, tw, row.h), 12.0, W::Regular, pal::TEXT, Align::Left);
                }
                ClipData::Image { size, .. } => {
                    let label = format!("Image · {} × {}", size.0, size.1);
                    let th = Rect::new(tx, row.y + 4.0, 46.0, row.h - 8.0);
                    if let Some(b) = self.clip_thumb(&rt, i) {
                        c.image(&b, th, 4.0);
                    } else {
                        c.icon(ico::IMAGE, th, 14.0, pal::DIM);
                    }
                    c.text(&label, Rect::new(tx + 56.0, row.y, tw - 56.0, row.h), 12.0, W::Regular, pal::DIM, Align::Left);
                }
            }
            if pinned {
                c.icon(ico::PIN, Rect::new(row.r() - 24.0 - if hov { 60.0 } else { 0.0 }, row.y, 18.0, row.h), 10.0, pal::ACCENT);
            }
            if hov {
                c.icon_button(Rect::new(row.r() - 62.0, row.y + 5.0, 26.0, 25.0), ico::PIN, 10.0, Action::ClipPin(i), if pinned { pal::ACCENT } else { pal::DIM });
                c.icon_button(Rect::new(row.r() - 32.0, row.y + 5.0, 26.0, 25.0), ico::DELETE, 10.0, Action::ClipRemove(i), pal::DIM);
            }
        }
        c.unclip();
    }

    // ------------------------------------------------ notifications

    fn draw_notifs(&mut self, c: &mut Canvas, r: Rect) {
        let n = self.notifs.len();
        c.text(&format!("{n} in Action Center"), Rect::new(r.x, r.y, 200.0, 20.0), 11.5, W::Medium, pal::DIM, Align::Left);
        if n > 0 {
            c.button(Rect::new(r.r() - 72.0, r.y - 2.0, 72.0, 24.0), "Clear all", Action::NotifClear, pal::CARD_HI, pal::TEXT);
        }
        let list = Rect::new(r.x, r.y + 26.0, r.w, r.h - 26.0);
        if n == 0 {
            c.text("No notifications", Rect::new(list.x, list.cy() - 20.0, list.w, 20.0), 13.0, W::Semi, pal::DIM, Align::Center);
            c.text("Mail, Outlook, Teams, Discord and other app alerts appear here", Rect::new(list.x, list.cy() + 2.0, list.w, 18.0), 11.0, W::Regular, pal::FAINT, Align::Center);
            return;
        }
        let rh = 54.0;
        let max_scroll = (n as f32 * rh - list.h).max(0.0);
        let off = self.scroll.entry(Page::Notifs).or_insert(0.0);
        *off = off.min(max_scroll);
        let off = *off;
        c.clip(list);
        let rt = c.rt.clone();
        for i in 0..n {
            let row = Rect::new(list.x, list.y + i as f32 * rh - off, list.w, rh - 4.0);
            if row.b() < list.y || row.y > list.b() {
                continue;
            }
            let hov = c.hover(row);
            c.rrect(row, 12.0, if hov { pal::CARD_HI } else { pal::CARD });
            let nt = self.notifs[i].clone();
            let ic = Rect::new(row.x + 10.0, row.y + 10.0, 28.0, 28.0);
            match self.notif_icon(&rt, &nt.app) {
                Some(b) => c.image(&b, ic, 6.0),
                None => {
                    c.rrect(ic, 6.0, pal::CARD_HI);
                    c.icon(ico::BELL, ic, 12.0, pal::DIM);
                }
            }
            let tx = ic.r() + 10.0;
            let tw = row.r() - tx - 12.0;
            c.text(&nt.app, Rect::new(tx, row.y + 4.0, tw - 60.0, 16.0), 10.5, W::Medium, pal::DIM, Align::Left);
            c.text(&ago(nt.time), Rect::new(row.r() - 70.0 - if hov { 24.0 } else { 0.0 }, row.y + 4.0, 58.0, 16.0), 10.5, W::Regular, pal::FAINT, Align::Right);
            c.text(&one_line(&nt.title), Rect::new(tx, row.y + 18.0, tw, 16.0), 12.0, W::Semi, pal::TEXT, Align::Left);
            c.text(&one_line(&nt.body), Rect::new(tx, row.y + 33.0, tw, 15.0), 11.0, W::Regular, pal::DIM, Align::Left);
            if hov {
                c.icon_button(Rect::new(row.r() - 30.0, row.y + 2.0, 24.0, 22.0), ico::CLOSE, 8.0, Action::NotifRemove(i), pal::DIM);
            }
        }
        c.unclip();
    }

    // ------------------------------------------------ system stats

    fn draw_stats(&mut self, c: &mut Canvas, r: Rect) {
        let gap = 10.0;
        let tw = (r.w - gap * 3.0) / 4.0;
        let tile = |i: usize| Rect::new(r.x + i as f32 * (tw + gap), r.y, tw, r.h);
        let st = &self.stats;
        let last = |q: &std::collections::VecDeque<f32>| q.back().copied().unwrap_or(0.0);

        let spark = |c: &Canvas, t: Rect, q: Vec<f32>, max: f32, col: Color| {
            let g = Rect::new(t.x + 12.0, t.b() - 44.0, t.w - 24.0, 32.0);
            if q.len() < 2 {
                return;
            }
            let n = stats::HISTORY as f32 - 1.0;
            let off = stats::HISTORY - q.len();
            let pts: Vec<(f32, f32)> = q.iter().enumerate().map(|(i, v)| (g.x + (i + off) as f32 / n * g.w, g.b() - (v / max.max(0.0001)).clamp(0.0, 1.0) * g.h)).collect();
            let mut area = pts.clone();
            area.push((pts.last().unwrap().0, g.b()));
            area.push((pts[0].0, g.b()));
            c.fill_path(&area, col.a(0.18));
            c.polyline(&pts, col, 1.6);
        };

        // CPU
        let t = tile(0);
        c.rrect(t, 14.0, pal::CARD);
        c.text("CPU", Rect::new(t.x + 12.0, t.y + 8.0, 80.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        c.text(&format!("{:.0}%", last(&st.cpu)), Rect::new(t.x + 12.0, t.y + 26.0, t.w - 24.0, 28.0), 22.0, W::Semi, pal::TEXT, Align::Left);
        spark(c, t, st.cpu.iter().copied().collect(), 100.0, pal::ACCENT);

        // GPU
        let t = tile(1);
        c.rrect(t, 14.0, pal::CARD);
        c.text("GPU", Rect::new(t.x + 12.0, t.y + 8.0, 80.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        let gv = if st.gpu_ok { format!("{:.0}%", last(&st.gpu)) } else { "—".into() };
        c.text(&gv, Rect::new(t.x + 12.0, t.y + 26.0, t.w - 24.0, 28.0), 22.0, W::Semi, pal::TEXT, Align::Left);
        spark(c, t, st.gpu.iter().copied().collect(), 100.0, pal::GREEN);

        // Memory + disk
        let t = tile(2);
        c.rrect(t, 14.0, pal::CARD);
        c.text("Memory", Rect::new(t.x + 12.0, t.y + 8.0, 80.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        let gb = 1024.0 * 1024.0 * 1024.0;
        c.text(&format!("{:.1} GB", st.ram_used / gb), Rect::new(t.x + 12.0, t.y + 26.0, t.w - 24.0, 28.0), 22.0, W::Semi, pal::TEXT, Align::Left);
        c.text(&format!("of {:.0} GB", st.ram_total / gb), Rect::new(t.x + 12.0, t.y + 54.0, t.w - 24.0, 16.0), 11.0, W::Regular, pal::DIM, Align::Left);
        c.bar(Rect::new(t.x + 12.0, t.y + 76.0, t.w - 24.0, 5.0), (st.ram_used / st.ram_total.max(1.0)) as f32, pal::CARD_HI, pal::PINK);
        c.text("Disk C:", Rect::new(t.x + 12.0, t.b() - 40.0, t.w - 24.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        c.text(&format!("{} free", stats::bytes(st.disk_free)), Rect::new(t.x + 12.0, t.b() - 24.0, t.w - 24.0, 16.0), 11.5, W::Regular, pal::TEXT, Align::Left);

        // Network
        let t = tile(3);
        c.rrect(t, 14.0, pal::CARD);
        c.text("Network", Rect::new(t.x + 12.0, t.y + 8.0, 80.0, 16.0), 11.0, W::Medium, pal::DIM, Align::Left);
        let d = st.down.back().copied().unwrap_or(0.0);
        let u = st.up.back().copied().unwrap_or(0.0);
        c.text(&format!("↓ {}/s", stats::bytes(d)), Rect::new(t.x + 12.0, t.y + 26.0, t.w - 24.0, 22.0), 14.0, W::Semi, pal::TEXT, Align::Left);
        c.text(&format!("↑ {}/s", stats::bytes(u)), Rect::new(t.x + 12.0, t.y + 48.0, t.w - 24.0, 18.0), 12.0, W::Regular, pal::DIM, Align::Left);
        let max = st.down.iter().cloned().fold(1024.0f64, f64::max) as f32;
        spark(c, t, st.down.iter().map(|v| *v as f32).collect(), max, pal::WATER);
    }
}
