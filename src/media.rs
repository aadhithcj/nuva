//! Now-playing state from Windows' Global System Media Transport Controls (any SMTC player).

use crate::events::{self, Event};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaState {
    pub active: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    /// Position in seconds at `stamp` (UTC ticks).
    pub pos: f64,
    pub dur: f64,
    pub stamp: i64,
    pub can_seek: bool,
}

impl MediaState {
    /// Current position, interpolated while playing.
    pub fn position(&self) -> f64 {
        if self.playing && self.stamp > 0 {
            let p = self.pos + (crate::time::utc_ticks() - self.stamp) as f64 / 1e7;
            if self.dur > 0.0 {
                p.clamp(0.0, self.dur)
            } else {
                p.max(0.0)
            }
        } else {
            self.pos
        }
    }

    pub fn key(&self) -> String {
        format!("{}\u{1}{}\u{1}{}", self.title, self.artist, self.album)
    }
}

pub enum MediaCmd {
    PlayPause,
    Next,
    Prev,
    Seek(f64),
}

enum Msg {
    Refresh,
    Cmd(MediaCmd),
}

pub struct Media {
    tx: Sender<Msg>,
}

impl Media {
    pub fn start() -> Media {
        let (tx, rx) = channel();
        let tx2 = tx.clone();
        std::thread::Builder::new()
            .name("media".into())
            .spawn(move || worker(tx2, rx))
            .ok();
        Media { tx }
    }

    pub fn send(&self, c: MediaCmd) {
        let _ = self.tx.send(Msg::Cmd(c));
    }
}

fn friendly_app(aumid: &str) -> String {
    let a = aumid.to_ascii_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("applemusic", "Apple Music"),
        ("zunemusic", "Media Player"),
        ("vlc", "VLC"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("youtube", "YouTube Music"),
        ("foobar", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("itunes", "iTunes"),
        ("discord", "Discord"),
    ];
    for (k, v) in known {
        if a.contains(k) {
            return v.into();
        }
    }
    let base = aumid.rsplit(['\\', '!']).next().unwrap_or(aumid);
    base.trim_end_matches(".exe").to_string()
}

fn read_thumb(r: &IRandomAccessStreamReference) -> windows::core::Result<Vec<u8>> {
    let stream = r.OpenReadAsync()?.get()?;
    let size = stream.Size()? as u32;
    if size == 0 || size > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    reader.LoadAsync(size)?.get()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(buf)
}

fn worker(tx: Sender<Msg>, rx: Receiver<Msg>) {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let mgr = loop {
        match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => break m,
            Err(_) => std::thread::sleep(Duration::from_secs(5)),
        }
    };
    {
        let t = tx.clone();
        let _ = mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }));
        let t = tx.clone();
        let _ = mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }));
    }
    let mut hooked: Option<GlobalSystemMediaTransportControlsSession> = None;
    let mut last = MediaState::default();
    let mut last_art_key = String::new();
    loop {
        let session = mgr.GetCurrentSession().ok();
        // Subscribe to per-session change events whenever the current session changes.
        let same = match (&session, &hooked) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(s) = &session {
                for kind in 0..3 {
                    let t = tx.clone();
                    let _ = match kind {
                        0 => s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = t.send(Msg::Refresh);
                            Ok(())
                        })),
                        1 => s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = t.send(Msg::Refresh);
                            Ok(())
                        })),
                        _ => s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                            let _ = t.send(Msg::Refresh);
                            Ok(())
                        })),
                    };
                }
            }
            hooked = session.clone();
        }

        let mut st = MediaState::default();
        let mut thumb = None;
        if let Some(s) = &session {
            st.active = true;
            st.app = s.SourceAppUserModelId().map(|h| friendly_app(&h.to_string())).unwrap_or_default();
            if let Ok(info) = s.GetPlaybackInfo() {
                st.playing = info.PlaybackStatus().map(|p| p == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing).unwrap_or(false);
                st.can_seek = info.Controls().and_then(|c| c.IsPlaybackPositionEnabled()).unwrap_or(false);
            }
            if let Ok(props) = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
                st.title = props.Title().map(|h| h.to_string()).unwrap_or_default();
                st.artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
                st.album = props.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
                thumb = props.Thumbnail().ok();
            }
            if let Ok(tl) = s.GetTimelineProperties() {
                let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
                let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
                let pos = tl.Position().map(|t| t.Duration).unwrap_or(0);
                st.dur = (end - start).max(0) as f64 / 1e7;
                st.pos = (pos - start).max(0) as f64 / 1e7;
                st.stamp = tl.LastUpdatedTime().map(|t| t.UniversalTime).unwrap_or(0);
                if st.stamp <= 0 || st.stamp > crate::time::utc_ticks() {
                    st.stamp = crate::time::utc_ticks();
                }
            }
            if st.title.is_empty() && st.artist.is_empty() {
                st.active = false;
            }
        }

        let drift = (st.position() - last.position()).abs();
        let changed = st.active != last.active
            || st.playing != last.playing
            || st.title != last.title
            || st.artist != last.artist
            || st.album != last.album
            || (st.dur - last.dur).abs() > 0.5
            || drift > 1.5;
        if changed {
            events::push(Event::Media(st.clone()));
        }
        let key = st.key();
        if st.active && key != last_art_key {
            if let Some(t) = &thumb {
                if let Ok(bytes) = read_thumb(t) {
                    if !bytes.is_empty() {
                        last_art_key = key.clone();
                        events::push(Event::MediaArt(key, bytes));
                    }
                }
            }
        }
        last = st;

        // Wait for an event, a command, or a slow safety refresh.
        let mut first = Some(rx.recv_timeout(Duration::from_millis(2500)));
        loop {
            let m = match first.take() {
                Some(r) => r,
                None => rx.recv_timeout(Duration::from_millis(60)),
            };
            match m {
                Ok(Msg::Refresh) => continue,
                Ok(Msg::Cmd(c)) => {
                    if let Some(s) = &session {
                        let _ = match c {
                            MediaCmd::PlayPause => s.TryTogglePlayPauseAsync().and_then(|o| o.get()),
                            MediaCmd::Next => s.TrySkipNextAsync().and_then(|o| o.get()),
                            MediaCmd::Prev => s.TrySkipPreviousAsync().and_then(|o| o.get()),
                            MediaCmd::Seek(secs) => {
                                let start = s.GetTimelineProperties().and_then(|t| t.StartTime()).map(|t| t.Duration).unwrap_or(0);
                                s.TryChangePlaybackPositionAsync(start + (secs * 1e7) as i64).and_then(|o| o.get())
                            }
                        };
                    }
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}
