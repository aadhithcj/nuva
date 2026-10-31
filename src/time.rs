//! Local calendar/time helpers built on Win32 (no external date crate).

use windows::core::w;
use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_ITIME};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTimePreciseAsFileTime};

pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];
pub const DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const DAYS_LETTER: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub ms: u32,
}

impl Now {
    pub fn day_num(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
    pub fn minute_of_day(&self) -> u32 {
        self.hour * 60 + self.minute
    }
    /// Seconds on a local, monotonic-enough timeline (days since 1970 * 86400 + time of day).
    pub fn secs(&self) -> f64 {
        self.day_num() as f64 * 86400.0
            + (self.hour * 3600 + self.minute * 60 + self.second) as f64
            + self.ms as f64 / 1000.0
    }
}

pub fn now() -> Now {
    let t = unsafe { GetLocalTime() };
    Now {
        year: t.wYear as i32,
        month: t.wMonth as u32,
        day: t.wDay as u32,
        hour: t.wHour as u32,
        minute: t.wMinute as u32,
        second: t.wSecond as u32,
        ms: t.wMilliseconds as u32,
    }
}

pub fn local_secs() -> f64 {
    now().secs()
}

pub fn today() -> i64 {
    now().day_num()
}

/// 100ns ticks since 1601-01-01 UTC (same epoch as WinRT DateTime).
pub fn utc_ticks() -> i64 {
    let ft = unsafe { GetSystemTimePreciseAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// 0 = Sunday.
pub fn weekday(day: i64) -> u32 {
    (day + 4).rem_euclid(7) as u32
}

pub fn date_str(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

pub fn is_24h() -> bool {
    let mut buf = [0u16; 4];
    let n = unsafe { GetLocaleInfoEx(w!("!x-sys-default-locale"), LOCALE_ITIME, Some(&mut buf)) };
    n <= 0 || buf[0] == '1' as u16
}

/// Minutes-of-day to a clock string.
pub fn hm(min: u32, h24: bool) -> String {
    let (h, m) = ((min / 60) % 24, min % 60);
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
    }
}

/// Short clock for the pill (no AM/PM suffix).
pub fn hm_short(min: u32, h24: bool) -> String {
    let (h, m) = ((min / 60) % 24, min % 60);
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{}:{m:02}", if h % 12 == 0 { 12 } else { h % 12 })
    }
}

pub fn mmss(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn day_label(day: i64) -> String {
    let t = today();
    if day == t {
        "Today".into()
    } else if day == t + 1 {
        "Tomorrow".into()
    } else if day == t - 1 {
        "Yesterday".into()
    } else {
        let (_, m, d) = civil_from_days(day);
        format!("{} {} {}", DAYS_SHORT[weekday(day) as usize], &MONTHS[m as usize - 1][..3], d)
    }
}
