//! Water reminders spread across active hours, with a bottle-refill nudge and a 7-day log.

use crate::store;
use crate::time::{self, Now};

pub enum WaterAlert {
    Drink,
    Refill,
}

pub struct Water {
    pub bottle: u32,
    pub goal: u32,
    pub start: u32,
    pub end: u32,
    pub taken: u32,
    pub since_refill: u32,
    day: i64,
    /// Reminder slots already fired today.
    fired: u32,
    snooze_until: f64,
    /// (day, ml) for past days.
    pub history: Vec<(i64, u32)>,
    undo: Vec<u32>,
}

impl Water {
    pub fn load() -> Water {
        let mut w = Water {
            bottle: 500,
            goal: 2000,
            start: 8,
            end: 22,
            taken: 0,
            since_refill: 0,
            day: time::today(),
            fired: 0,
            snooze_until: 0.0,
            history: Vec::new(),
            undo: Vec::new(),
        };
        let text = store::read("water.txt", false);
        let mut saved_day = None;
        for (k, v) in store::kv(&text) {
            let n = v.parse::<u32>().ok();
            match (k, n) {
                ("bottle", Some(n)) => w.bottle = n.clamp(100, 3000),
                ("goal", Some(n)) => w.goal = n.clamp(500, 6000),
                ("start", Some(n)) => w.start = n.min(23),
                ("hours", Some(n)) => w.end = n.min(24),
                ("taken", Some(n)) => w.taken = n,
                ("refill", Some(n)) => w.since_refill = n,
                ("fired", Some(n)) => w.fired = n,
                ("day", _) => saved_day = time::parse_date(v),
                ("h", _) => {
                    if let Some((d, ml)) = v.split_once(':') {
                        if let (Some(d), Ok(ml)) = (time::parse_date(d), ml.parse()) {
                            w.history.push((d, ml));
                        }
                    }
                }
                _ => {}
            }
        }
        if w.end <= w.start {
            w.end = (w.start + 1).min(24);
        }
        if let Some(d) = saved_day {
            w.day = d;
        }
        w.roll(time::today());
        w
    }

    pub fn save(&self) {
        let mut s = format!(
            "bottle={}\ngoal={}\nstart={}\nhours={}\nday={}\ntaken={}\nrefill={}\nfired={}\n",
            self.bottle,
            self.goal,
            self.start,
            self.end,
            time::date_str(self.day),
            self.taken,
            self.since_refill,
            self.fired
        );
        for (d, ml) in &self.history {
            s += &format!("h={}:{}\n", time::date_str(*d), ml);
        }
        store::write("water.txt", false, &s);
    }

    fn roll(&mut self, today: i64) {
        if today != self.day {
            if self.taken > 0 {
                self.history.push((self.day, self.taken));
            }
            self.history.retain(|(d, _)| *d > today - 30);
            self.day = today;
            self.taken = 0;
            self.fired = 0;
            self.undo.clear();
            self.save();
        }
    }

    pub fn count(&self) -> u32 {
        (self.end - self.start).max(1)
    }

    /// Suggested amount per reminder, rounded to 10 ml.
    pub fn per(&self) -> u32 {
        ((self.goal as f32 / self.count() as f32 / 10.0).round() as u32 * 10).max(50)
    }

    fn slot(&self, i: u32) -> u32 {
        let span = (self.end - self.start) * 60;
        self.start * 60 + span * i / self.count()
    }

    pub fn next_reminder(&self) -> Option<u32> {
        (self.fired < self.count() && self.taken < self.goal).then(|| self.slot(self.fired))
    }

    pub fn tick(&mut self, now: &Now, enabled: bool) -> Option<WaterAlert> {
        self.roll(now.day_num());
        if !enabled || now.secs() < self.snooze_until || self.taken >= self.goal {
            return None;
        }
        let m = now.minute_of_day();
        if m < self.start * 60 || m >= self.end * 60 {
            return None;
        }
        let mut due = false;
        // Fire once for the latest passed slot; skip slots missed while away.
        while self.fired < self.count() && self.slot(self.fired) <= m {
            self.fired += 1;
            due = true;
        }
        if !due {
            return None;
        }
        self.save();
        Some(if self.since_refill + self.per() > self.bottle { WaterAlert::Refill } else { WaterAlert::Drink })
    }

    pub fn drank(&mut self) {
        self.roll(time::today());
        let n = self.per();
        self.taken += n;
        self.since_refill += n;
        self.undo.push(n);
        self.save();
    }

    pub fn undo(&mut self) {
        if let Some(n) = self.undo.pop() {
            self.taken = self.taken.saturating_sub(n);
            self.since_refill = self.since_refill.saturating_sub(n);
            self.save();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn refilled(&mut self) {
        self.since_refill = 0;
        self.save();
    }

    pub fn snooze(&mut self, mins: u32) {
        self.snooze_until = time::local_secs() + mins as f64 * 60.0;
    }

    /// The last 7 days including today, oldest first.
    pub fn week(&self) -> Vec<(i64, u32)> {
        let t = self.day;
        (0..7)
            .map(|i| {
                let d = t - 6 + i;
                let ml = if d == t { self.taken } else { self.history.iter().filter(|(x, _)| *x == d).map(|(_, m)| *m).sum() };
                (d, ml)
            })
            .collect()
    }
}
