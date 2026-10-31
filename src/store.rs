//! Plain-text persistence in %APPDATA%\Nuva and %LOCALAPPDATA%\Nuva.

use std::fs;
use std::path::PathBuf;

fn dir_from(var: &str) -> PathBuf {
    let base = std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let old = base.join("Nusa");
    let p = base.join("Nuva");
    if !p.exists() && old.exists() {
        let _ = fs::rename(&old, &p);
    }
    let _ = fs::create_dir_all(&p);
    p
}

/// Roaming data: tasks, water log, shelf, clipboard.
pub fn data_dir() -> PathBuf {
    dir_from("APPDATA")
}

/// Machine-local data: settings and caches.
pub fn local_dir() -> PathBuf {
    dir_from("LOCALAPPDATA")
}

pub fn read(name: &str, local: bool) -> String {
    let p = if local { local_dir() } else { data_dir() }.join(name);
    fs::read_to_string(p).unwrap_or_default()
}

/// Write via a temp file and rename so a crash never leaves a half-written file.
pub fn write(name: &str, local: bool, contents: &str) {
    let dir = if local { local_dir() } else { data_dir() };
    let tmp = dir.join(format!("{name}.tmp"));
    if fs::write(&tmp, contents).is_ok() {
        let _ = fs::rename(&tmp, dir.join(name));
    }
}

/// Append a line to %LOCALAPPDATA%\Nuva\debug.log (debug builds only).
#[allow(dead_code)]
pub fn debug(msg: &str) {
    if cfg!(debug_assertions) {
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(local_dir().join("debug.log")) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

pub fn kv(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| {
        let l = l.trim();
        if l.starts_with('#') {
            return None;
        }
        let (k, v) = l.split_once('=')?;
        Some((k.trim(), v.trim()))
    })
}

/// Escape tabs/newlines so free text fits on one TSV line.
pub fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

pub fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}
