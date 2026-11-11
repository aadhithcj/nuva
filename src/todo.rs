//! Tasks with a start time and duration; reminders when a task starts and when its time is up.

use crate::store;
use crate::time::{self, Now};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Task {
    pub id: u64,
    pub day: i64,
    /// Minute of day.
    pub start: u32,
    /// Minutes.
    pub dur: u32,
    pub done: bool,
    pub title: String,
    pub fired_start: bool,
    pub fired_end: bool,
    pub snooze_until: Option<Instant>,
}

impl Task {
    /// End minute relative to the task's day; may exceed 1440 for tasks crossing midnight.
    pub fn end(&self) -> u32 {
        self.start.saturating_add(self.dur)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskEvent {
    Start,
    End,
}

pub struct Todo {
    pub tasks: Vec<Task>,
    next_id: u64,
}

impl Todo {
    pub fn load() -> Todo {
        let mut tasks = Vec::new();
        for line in store::read("tasks.txt", false).lines() {
            let f: Vec<&str> = line.splitn(8, '\t').collect();
            if f.len() < 8 {
                continue;
            }
            let (Ok(id), Some(day), Ok(start), Ok(dur)) = (f[0].parse(), time::parse_date(f[1]), f[2].parse(), f[3].parse()) else {
                continue;
            };
            tasks.push(Task {
                id,
                day,
                start,
                dur,
                done: f[4] == "1",
                fired_start: f[5] == "1",
                fired_end: f[6] == "1",
                title: store::unesc(f[7]),
                snooze_until: None,
            });
        }
        // Keep two weeks of history.
        let t = time::today();
        tasks.retain(|k| k.day >= t - 14);
        let next_id = tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        Todo { tasks, next_id }
    }

    pub fn save(&self) {
        let mut s = String::new();
        for t in &self.tasks {
            s += &format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                t.id,
                time::date_str(t.day),
                t.start,
                t.dur,
                t.done as u8,
                t.fired_start as u8,
                t.fired_end as u8,
                store::esc(&t.title)
            );
        }
        store::write("tasks.txt", false, &s);
    }

    pub fn for_day(&self, day: i64) -> Vec<Task> {
        let mut v: Vec<Task> = self.tasks.iter().filter(|t| t.day == day).cloned().collect();
        v.sort_by_key(|t| (t.done, t.start));
        v
    }

    pub fn has_tasks(&self, day: i64) -> bool {
        self.tasks.iter().any(|t| t.day == day)
    }

    pub fn done_today(&self) -> usize {
        let t = time::today();
        self.tasks.iter().filter(|k| k.day == t && k.done).count()
    }

    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    pub fn upsert(&mut self, id: Option<u64>, title: String, day: i64, start: u32, dur: u32) {
        let now = time::now();
        let past = |day: i64, m: u32| day < now.day_num() || (day == now.day_num() && m <= now.minute_of_day());
        if let Some(t) = id.and_then(|id| self.get_mut(id)) {
            t.title = title;
            t.day = day;
            t.start = start;
            t.dur = dur;
            t.fired_start = past(day, start);
            t.fired_end = past(day, start.saturating_add(dur));
            t.snooze_until = None;
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.tasks.push(Task {
                id,
                day,
                start,
                dur,
                done: false,
                title,
                fired_start: past(day, start),
                fired_end: past(day, start.saturating_add(dur)),
                snooze_until: None,
            });
        }
        self.save();
    }

    pub fn toggle(&mut self, id: u64) {
        if let Some(t) = self.get_mut(id) {
            t.done = !t.done;
        }
        self.save();
    }

    pub fn delete(&mut self, id: u64) {
        self.tasks.retain(|t| t.id != id);
        self.save();
    }

    pub fn snooze(&mut self, id: u64, mins: u32, ev: TaskEvent) {
        if let Some(t) = self.get_mut(id) {
            t.snooze_until = Some(Instant::now() + Duration::from_secs(mins as u64 * 60));
            match ev {
                TaskEvent::Start => t.fired_start = false,
                TaskEvent::End => t.fired_end = false,
            }
        }
    }

    pub fn tick(&mut self, now: &Now, on_start: bool, on_end: bool) -> Vec<(u64, TaskEvent)> {
        let (day, m) = (now.day_num(), now.minute_of_day());
        let mut out = Vec::new();
        // Tasks from yesterday that run past midnight are still relevant for their end event.
        for t in self.tasks.iter_mut().filter(|t| {
            !t.done && (t.day == day || (t.day == day - 1 && t.end() >= 1440))
        }) {
            if let Some(until) = t.snooze_until {
                if Instant::now() < until {
                    continue;
                }
                t.snooze_until = None;
            }
            if t.day == day && !t.fired_start && m >= t.start {
                t.fired_start = true;
                if on_start && m < t.end().max(t.start + 1) {
                    out.push((t.id, TaskEvent::Start));
                }
            }
            let end_min = if t.day == day {
                if t.end() >= 1440 {
                    // The end reminder fires tomorrow at the wrapped minute.
                    continue;
                }
                t.end()
            } else {
                t.end().saturating_sub(1440)
            };
            if !t.fired_end && t.dur > 0 && m >= end_min {
                t.fired_end = true;
                if on_end {
                    out.push((t.id, TaskEvent::End));
                }
            }
        }
        if !out.is_empty() {
            self.save();
        }
        out
    }
}
