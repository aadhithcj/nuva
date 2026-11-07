//! Now-playing state from Windows' Global System Media Transport Controls (any SMTC player).

use crate::events::{self, Event};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

const WAVE_FORMAT_IEEE_FLOAT: u32 = 0x0003;
const WAVE_FORMAT_EXTENSIBLE: u32 = 0xFFFE;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows_core::GUID =
    windows_core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

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
        start_spectrum_worker();
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
            set_audio_playing(st.playing);
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

// ---------------------------------------------------------------- Audio Spectrum & Palette

static SPECTRUM_NOTIFY: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
pub static AUDIO_PLAYING: AtomicBool = AtomicBool::new(false);

pub fn set_audio_playing(playing: bool) {
    let was = AUDIO_PLAYING.swap(playing, Ordering::AcqRel);
    if was != playing {
        let (lock, cvar) = &SPECTRUM_NOTIFY;
        let mut guard = lock.lock().unwrap();
        *guard = playing;
        cvar.notify_all();
    }
}

pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let sat = if max > 0.0 { delta / max } else { 0.0 };
    let val = max;
    let hue = if delta == 0.0 {
        0.0
    } else if (max - r).abs() < 1e-5 {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-5 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    (hue, sat, val)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - (((h / 60.0).rem_euclid(2.0)) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    ((r1 + m).clamp(0.0, 1.0), (g1 + m).clamp(0.0, 1.0), (b1 + m).clamp(0.0, 1.0))
}

pub fn extract_vibrant_accent(bgra: &[u8]) -> Option<(u8, u8, u8)> {
    if bgra.len() < 4 {
        return None;
    }
    // (total_weight, weighted_r, weighted_g, weighted_b)
    let mut bins = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); 12];

    for chunk in bgra.chunks_exact(4) {
        let (b, g, r) = (chunk[0] as f32 / 255.0, chunk[1] as f32 / 255.0, chunk[2] as f32 / 255.0);
        let (hue, sat, val) = rgb_to_hsv(r, g, b);

        // Filter out muddy grays, near-black, and blown-out white
        if sat < 0.15 || val < 0.15 || val > 0.92 {
            continue;
        }

        let bin_idx = ((hue / 30.0).floor() as usize).min(11);
        let weight = sat * val * val;
        bins[bin_idx].0 += weight;
        bins[bin_idx].1 += weight * chunk[2] as f32;
        bins[bin_idx].2 += weight * chunk[1] as f32;
        bins[bin_idx].3 += weight * chunk[0] as f32;
    }

    let best = bins.iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if best.0 <= 0.0 {
        return None;
    }
    let (w, rs, gs, bs) = *best;
    let (r, g, b) = (rs / w / 255.0, gs / w / 255.0, bs / w / 255.0);
    let (h, s, v) = rgb_to_hsv(r, g, b);
    let target_v = v.clamp(0.65, 0.95);
    let (r_out, g_out, b_out) = hsv_to_rgb(h, s, target_v);
    Some((
        (r_out * 255.0).round() as u8,
        (g_out * 255.0).round() as u8,
        (b_out * 255.0).round() as u8,
    ))
}

pub fn start_spectrum_worker() {
    std::thread::Builder::new()
        .name("audio_spectrum".into())
        .spawn(|| {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            loop {
                // If audio is paused, park on the condvar with 0% CPU
                {
                    let (lock, cvar) = &SPECTRUM_NOTIFY;
                    let mut guard = lock.lock().unwrap();
                    while !*guard {
                        guard = cvar.wait(guard).unwrap();
                    }
                }

                // Run capture session; if device invalidates (e.g. Bluetooth toggle), catch error and retry
                if let Err(_e) = run_audio_capture_session() {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        })
        .ok();
}

fn run_audio_capture_session() -> windows_core::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        let pwfx = client.GetMixFormat()?;
        let sample_rate = (*pwfx).nSamplesPerSec.max(8000) as f32;
        let channels = (*pwfx).nChannels.max(1) as usize;
        let is_float = (*pwfx).wFormatTag == WAVE_FORMAT_IEEE_FLOAT as u16
            || ((*pwfx).wFormatTag == WAVE_FORMAT_EXTENSIBLE as u16
                && std::ptr::addr_of!((*(pwfx as *const WAVEFORMATEXTENSIBLE)).SubFormat).read_unaligned() == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
        let bits = (*pwfx).wBitsPerSample as usize;

        // 50ms buffer to prevent overruns
        let buffer_duration_100ns = 500_000;
        let init_res = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            buffer_duration_100ns,
            0,
            pwfx,
            None,
        );
        CoTaskMemFree(Some(pwfx as *const _ as *const std::ffi::c_void));
        init_res?;

        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;

        // Ring buffer of mono samples for Goertzel analysis (2048 samples at 48kHz resolves ~23 Hz bins)
        const N: usize = 2048;
        let mut ring = vec![0.0f32; N];
        let mut ring_pos = 0usize;
        let mut envelope = [0.0f32; 5];
        let target_freqs = [65.0f32, 180.0, 520.0, 1800.0, 5800.0];

        while AUDIO_PLAYING.load(Ordering::Relaxed) {
            let mut packet_size = capture.GetNextPacketSize()?;
            let mut had_samples = false;

            while packet_size > 0 {
                let mut pdata = std::ptr::null_mut();
                let mut num_frames = 0u32;
                let mut flags = 0u32;

                capture.GetBuffer(&mut pdata, &mut num_frames, &mut flags, None, None)?;

                if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !pdata.is_null() && num_frames > 0 {
                    had_samples = true;
                    if is_float && bits == 32 {
                        let slice = std::slice::from_raw_parts(pdata as *const f32, (num_frames as usize) * channels);
                        for frame in slice.chunks_exact(channels) {
                            let mut sum = 0.0f32;
                            for &s in frame {
                                sum += s;
                            }
                            ring[ring_pos] = sum / (channels as f32);
                            ring_pos = (ring_pos + 1) % N;
                        }
                    } else if bits == 16 {
                        let slice = std::slice::from_raw_parts(pdata as *const i16, (num_frames as usize) * channels);
                        for frame in slice.chunks_exact(channels) {
                            let mut sum = 0.0f32;
                            for &s in frame {
                                sum += s as f32 / 32768.0;
                            }
                            ring[ring_pos] = sum / (channels as f32);
                            ring_pos = (ring_pos + 1) % N;
                        }
                    }
                }

                capture.ReleaseBuffer(num_frames)?;
                packet_size = capture.GetNextPacketSize()?;
            }

            if had_samples {
                // Calculate 5-band Goertzel energy with Hann window
                for (i, &f) in target_freqs.iter().enumerate() {
                    let k = (N as f32 * f / sample_rate).round().max(1.0);
                    let omega = (std::f32::consts::TAU * k) / (N as f32);
                    let coeff = 2.0 * omega.cos();
                    let mut s1 = 0.0f32;
                    let mut s2 = 0.0f32;

                    for n in 0..N {
                        let idx = (ring_pos + n) % N;
                        let sample = ring[idx];
                        // Hann window
                        let w = 0.5 * (1.0 - ((std::f32::consts::TAU * n as f32) / (N as f32)).cos());
                        let s0 = sample * w + coeff * s1 - s2;
                        s2 = s1;
                        s1 = s0;
                    }

                    let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
                    let raw_mag = (power.sqrt() / (N as f32 * 0.25)).clamp(0.0, 1.0);

                    // Fast-attack, slow-release envelope smoothing
                    if raw_mag > envelope[i] {
                        envelope[i] = envelope[i] * 0.25 + raw_mag * 0.75;
                    } else {
                        envelope[i] = envelope[i] * 0.86 + raw_mag * 0.14;
                    }
                }

                crate::events::push(crate::events::Event::AudioSpectrum(envelope));
            }

            std::thread::sleep(Duration::from_millis(16));
        }

        let _ = client.Stop();
        Ok(())
    }
}
