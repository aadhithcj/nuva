//! Synced lyrics from LRCLIB (https://lrclib.net), fetched off the UI thread.

use crate::events::{self, Event};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::core::HSTRING;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

const UA: &str = "Nuva/1.2.0 (personal Windows build)";

/// The newest lyrics request; older worker threads drop their result.
static LATEST_REQ_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Line {
    pub t: f64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub synced: bool,
}

impl Lyrics {
    /// Index of the line being sung at `pos` seconds.
    pub fn current(&self, pos: f64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let mut idx = None;
        for (i, l) in self.lines.iter().enumerate() {
            if l.t <= pos + 0.25 {
                idx = Some(i);
            } else {
                break;
            }
        }
        idx
    }

    pub fn next_time(&self, pos: f64) -> Option<f64> {
        self.lines.iter().map(|l| l.t).find(|&t| t > pos + 0.25)
    }
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o += &format!("%{b:02X}"),
        }
    }
    o
}

/// Strip video-ish noise ("(Official Video)", "[Lyrics]", "- Topic") and split "Artist - Title".
pub fn clean(title: &str, artist: &str) -> (String, String) {
    let mut t = title.to_string();
    let mut a = artist.trim().trim_end_matches(" - Topic").trim_end_matches("VEVO").trim().to_string();
    let noise = ["official", "video", "lyric", "audio", "visualizer", "remaster", "hd", "4k", "mv", "live", "explicit"];
    for (open, close) in [('(', ')'), ('[', ']')] {
        while let (Some(s), Some(e)) = (t.find(open), t.rfind(close)) {
            if e <= s {
                break;
            }
            let inner = t[s + 1..e].to_ascii_lowercase();
            if noise.iter().any(|n| inner.contains(n)) {
                t.replace_range(s..=e, "");
            } else {
                break;
            }
        }
    }
    if let Some((x, y)) = t.split_once(" - ") {
        if a.is_empty() || x.trim().eq_ignore_ascii_case(&a) || y.to_ascii_lowercase().contains("remaster") {
            if a.is_empty() || x.trim().eq_ignore_ascii_case(&a) {
                a = x.trim().to_string();
                t = y.to_string();
            } else {
                t = x.to_string();
            }
        }
    }
    (t.trim().to_string(), a)
}

pub fn parse_lrc(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            rest = rest[end + 1..].trim_start();
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) {
                    let t = m * 60.0 + s;
                    if t.is_finite() && t >= 0.0 {
                        stamps.push(t);
                    }
                }
            }
        }
        for t in stamps {
            out.push(Line { t, text: rest.to_string() });
        }
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

fn get(client: &HttpClient, url: &str) -> Option<String> {
    let uri = Uri::CreateUri(&HSTRING::from(url)).ok()?;
    client.GetStringAsync(&uri).ok()?.get().ok().map(|h| h.to_string())
}

fn from_json(v: &serde_json::Value) -> Option<Lyrics> {
    if let Some(s) = v.get("syncedLyrics").and_then(|s| s.as_str()) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some(Lyrics { lines, synced: true });
        }
    }
    let plain = v.get("plainLyrics").and_then(|s| s.as_str())?;
    let lines: Vec<Line> = plain.lines().map(|l| Line { t: -1.0, text: l.trim().to_string() }).collect();
    (!lines.is_empty()).then_some(Lyrics { lines, synced: false })
}

fn fetch(title: &str, artist: &str, album: &str, dur: f64) -> Option<Lyrics> {
    let client = HttpClient::new().ok()?;
    if let Ok(h) = client.DefaultRequestHeaders() {
        let _ = h.UserAgent().and_then(|ua| ua.TryParseAdd(&HSTRING::from(UA)));
    }
    let (t, a) = clean(title, artist);
    if t.is_empty() {
        return None;
    }
    // Exact match first.
    if !a.is_empty() {
        let mut url = format!("https://lrclib.net/api/get?track_name={}&artist_name={}", enc(&t), enc(&a));
        if !album.is_empty() {
            url += &format!("&album_name={}", enc(album));
        }
        if dur > 0.0 {
            url += &format!("&duration={}", dur.round() as i64);
        }
        if let Some(body) = get(&client, &url) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(l) = from_json(&v) {
                    if l.synced {
                        return Some(l);
                    }
                }
            }
        }
    }
    // Fuzzy search, preferring synced results with a close duration.
    let mut url = format!("https://lrclib.net/api/search?track_name={}", enc(&t));
    if !a.is_empty() {
        url += &format!("&artist_name={}", enc(&a));
    }
    let body = get(&client, &url)?;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
    let score = |v: &serde_json::Value| -> f64 {
        let synced = v.get("syncedLyrics").and_then(|s| s.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        let d = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
        let dd = if dur > 0.0 && d > 0.0 { (d - dur).abs() } else { 30.0 };
        (if synced { 0.0 } else { 1000.0 }) + dd
    };
    let best = arr.iter().min_by(|x, y| score(x).total_cmp(&score(y)))?;
    if dur > 0.0 {
        if let Some(d) = best.get("duration").and_then(|d| d.as_f64()) {
            if (d - dur).abs() > 20.0 {
                return None;
            }
        }
    }
    from_json(best)
}

pub fn request(gen: u64, title: String, artist: String, album: String, dur: f64) {
    LATEST_REQ_ID.store(gen, Ordering::SeqCst);
    std::thread::Builder::new()
        .name("lyrics".into())
        .spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            // A newer track was selected while this worker was starting.
            if LATEST_REQ_ID.load(Ordering::SeqCst) != gen {
                return;
            }
            let r = fetch(&title, &artist, &album, dur);
            // Drop stale responses instead of flashing old lyrics over the new track.
            if LATEST_REQ_ID.load(Ordering::SeqCst) == gen {
                events::push(Event::Lyrics(gen, r));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_sorts_and_drops_invalid_timestamps() {
        let text = "\
[00:10.50]second
[NaN:00.00]not a time
[00:05.00]first
[-1:00.00]negative
[00:05.00]first again";
        let lines = parse_lrc(text);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].text, "first again");
        assert_eq!(lines[2].text, "second");
        assert!(lines.iter().all(|l| l.t.is_finite() && l.t >= 0.0));
        assert!(lines.windows(2).all(|w| w[0].t <= w[1].t));
    }

    #[test]
    fn lrc_nan_never_panics() {
        // Before the fix this input made `partial_cmp(...).unwrap()` panic.
        let lines = parse_lrc("[00:01.00]a\n[NaN:NaN]b\n[00:02.00]c");
        assert_eq!(lines.len(), 2);
    }
}
