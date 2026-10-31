//! Reminder chimes, synthesized once into in-memory WAV files.

use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

#[derive(Clone, Copy)]
pub enum Chime {
    Focus = 0,
    Task = 1,
    Water = 2,
}

static WAVS: OnceLock<[Vec<u8>; 3]> = OnceLock::new();

fn synth(notes: &[(f32, f32)], total: f32) -> Vec<u8> {
    const RATE: u32 = 22050;
    let n = (total * RATE as f32) as usize;
    let mut pcm = vec![0f32; n];
    for &(freq, start) in notes {
        let s0 = (start * RATE as f32) as usize;
        for i in s0..n {
            let t = (i - s0) as f32 / RATE as f32;
            let env = (-t * 4.5).exp() * (t * 400.0).min(1.0);
            let w = (std::f32::consts::TAU * freq * t).sin() + 0.25 * (std::f32::consts::TAU * freq * 2.0 * t).sin();
            pcm[i] += 0.22 * env * w;
        }
    }
    let data_len = (n * 2) as u32;
    let mut out = Vec::with_capacity(44 + n * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

fn wavs() -> &'static [Vec<u8>; 3] {
    WAVS.get_or_init(|| {
        [
            synth(&[(659.3, 0.0), (830.6, 0.16), (987.8, 0.32)], 1.6),
            synth(&[(880.0, 0.0), (1174.7, 0.14)], 1.2),
            synth(&[(523.3, 0.0), (784.0, 0.12)], 1.2),
        ]
    })
}

pub fn play(c: Chime) {
    let wav = &wavs()[c as usize];
    unsafe {
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}
