//! Focus timer: focus / short break / long break, long break every 4th focus.

use crate::settings::Settings;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Focus,
    Short,
    Long,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Focus => "Focus",
            Mode::Short => "Short break",
            Mode::Long => "Long break",
        }
    }
}

pub struct Pomodoro {
    pub mode: Mode,
    pub running: bool,
    /// Monotonic deadline while running; immune to wall-clock changes.
    ends_at: Option<Instant>,
    remaining: f64,
    pub done_focus: u32,
}

impl Pomodoro {
    pub fn new(s: &Settings) -> Self {
        Pomodoro { mode: Mode::Focus, running: false, ends_at: None, remaining: s.focus as f64 * 60.0, done_focus: 0 }
    }

    pub fn duration(&self, s: &Settings) -> f64 {
        60.0 * match self.mode {
            Mode::Focus => s.focus,
            Mode::Short => s.short_break,
            Mode::Long => s.long_break,
        } as f64
    }

    pub fn left(&self) -> f64 {
        match self.ends_at {
            Some(end) => end.saturating_duration_since(Instant::now()).as_secs_f64(),
            None => self.remaining,
        }
    }

    /// Running, or paused part-way through a session.
    pub fn active(&self, s: &Settings) -> bool {
        self.running || (self.remaining - self.duration(s)).abs() > 0.5
    }

    pub fn frac(&self, s: &Settings) -> f32 {
        let d = self.duration(s).max(1.0);
        (1.0 - self.left() / d).clamp(0.0, 1.0) as f32
    }

    pub fn start_pause(&mut self) {
        if self.running {
            self.remaining = self.left();
            self.ends_at = None;
            self.running = false;
        } else {
            self.ends_at = Some(Instant::now() + Duration::from_secs_f64(self.remaining.max(0.0)));
            self.running = true;
        }
    }

    pub fn reset(&mut self, s: &Settings) {
        self.running = false;
        self.ends_at = None;
        self.remaining = self.duration(s);
    }

    pub fn set_mode(&mut self, m: Mode, s: &Settings) {
        self.mode = m;
        self.reset(s);
    }

    fn advance(&mut self, s: &Settings) {
        let next = match self.mode {
            Mode::Focus => {
                self.done_focus += 1;
                if self.done_focus % 4 == 0 {
                    Mode::Long
                } else {
                    Mode::Short
                }
            }
            _ => Mode::Focus,
        };
        self.set_mode(next, s);
    }

    pub fn skip(&mut self, s: &Settings) {
        self.advance(s);
    }

    /// Durations changed in settings: refresh an idle timer.
    pub fn settings_changed(&mut self, s: &Settings) {
        if !self.running {
            self.remaining = self.duration(s);
        }
    }

    /// Returns the mode that just finished, if any.
    pub fn tick(&mut self, s: &Settings) -> Option<Mode> {
        if self.running && self.left() <= 0.0 {
            let done = self.mode;
            self.advance(s);
            return Some(done);
        }
        None
    }
}
