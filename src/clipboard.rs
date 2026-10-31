//! Clipboard history (text and images). Respects apps that opt out of clipboard monitoring.

use crate::store;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};

const MAX_ITEMS: usize = 40;

pub enum ClipData {
    Text(String),
    /// Thumbnail as premultiplied BGRA plus the original CF_DIB bytes for re-copying.
    Image { w: u32, h: u32, thumb: Vec<u8>, dib: Vec<u8>, size: (u32, u32) },
}

pub struct ClipItem {
    pub data: ClipData,
    pub pinned: bool,
    pub id: u64,
}

pub struct Clipboard {
    pub items: Vec<ClipItem>,
    skip_next: bool,
    next_id: u64,
    persist: bool,
}

impl Clipboard {
    pub fn new(persist: bool) -> Clipboard {
        let mut c = Clipboard { items: Vec::new(), skip_next: false, next_id: 1, persist };
        if persist {
            for line in store::read("clipboard.txt", false).lines() {
                let (pinned, text) = match line.split_once('\t') {
                    Some(("p", t)) => (true, t),
                    Some((_, t)) => (false, t),
                    None => continue,
                };
                let id = c.next_id;
                c.next_id += 1;
                c.items.push(ClipItem { data: ClipData::Text(store::unesc(text)), pinned, id });
            }
        }
        c
    }

    pub fn set_persist(&mut self, on: bool) {
        self.persist = on;
        if on {
            self.save();
        } else {
            store::write("clipboard.txt", false, "");
        }
    }

    fn save(&self) {
        if !self.persist {
            return;
        }
        let mut s = String::new();
        for it in &self.items {
            if let ClipData::Text(t) = &it.data {
                s += &format!("{}\t{}\n", if it.pinned { "p" } else { "u" }, store::esc(t));
            }
        }
        store::write("clipboard.txt", false, &s);
    }

    /// Called on WM_CLIPBOARDUPDATE. Returns true if the history changed.
    pub fn capture(&mut self, hwnd: HWND) -> bool {
        if self.skip_next {
            self.skip_next = false;
            return false;
        }
        let Some(data) = (unsafe { read_clipboard(hwnd) }) else { return false };
        if let ClipData::Text(t) = &data {
            if let Some(pos) = self.items.iter().position(|i| matches!(&i.data, ClipData::Text(x) if x == t)) {
                let it = self.items.remove(pos);
                self.items.insert(0, it);
                self.save();
                return true;
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(0, ClipItem { data, pinned: false, id });
        // Trim oldest unpinned items.
        while self.items.iter().filter(|i| !i.pinned).count() > MAX_ITEMS {
            if let Some(p) = self.items.iter().rposition(|i| !i.pinned) {
                self.items.remove(p);
            }
        }
        self.save();
        true
    }

    pub fn copy(&mut self, i: usize, hwnd: HWND) {
        let Some(it) = self.items.get(i) else { return };
        let ok = unsafe {
            match &it.data {
                ClipData::Text(t) => {
                    let u: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
                    let bytes = std::slice::from_raw_parts(u.as_ptr() as *const u8, u.len() * 2);
                    set_clipboard(hwnd, CF_UNICODETEXT.0 as u32, bytes)
                }
                ClipData::Image { dib, .. } => set_clipboard(hwnd, CF_DIB.0 as u32, dib),
            }
        };
        if ok {
            self.skip_next = true;
            let it = self.items.remove(i);
            self.items.insert(0, it);
            self.save();
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            self.items.remove(i);
            self.save();
        }
    }

    pub fn toggle_pin(&mut self, i: usize) {
        if let Some(it) = self.items.get_mut(i) {
            it.pinned = !it.pinned;
            self.save();
        }
    }

    pub fn clear(&mut self) {
        self.items.retain(|i| i.pinned);
        self.save();
    }
}

unsafe fn open(hwnd: HWND) -> bool {
    for _ in 0..5 {
        if OpenClipboard(hwnd).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

unsafe fn excluded() -> bool {
    let ex = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    let ignore = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
    if IsClipboardFormatAvailable(ex).is_ok() || IsClipboardFormatAvailable(ignore).is_ok() {
        return true;
    }
    let hist = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
    if IsClipboardFormatAvailable(hist).is_ok() {
        if let Ok(h) = GetClipboardData(hist) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u32;
            let deny = !p.is_null() && *p == 0;
            let _ = GlobalUnlock(g);
            return deny;
        }
    }
    false
}

unsafe fn read_clipboard(hwnd: HWND) -> Option<ClipData> {
    if !open(hwnd) {
        return None;
    }
    let out = (|| {
        if excluded() {
            return None;
        }
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
            let g = HGLOBAL(GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let max = GlobalSize(g) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(g);
            return (!s.trim().is_empty()).then_some(ClipData::Text(s));
        }
        if IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
            let g = HGLOBAL(GetClipboardData(CF_DIB.0 as u32).ok()?.0);
            let p = GlobalLock(g) as *const u8;
            if p.is_null() {
                return None;
            }
            let dib = std::slice::from_raw_parts(p, GlobalSize(g)).to_vec();
            let _ = GlobalUnlock(g);
            return dib_thumb(&dib, 160).map(|(w, h, thumb, size)| ClipData::Image { w, h, thumb, dib, size });
        }
        None
    })();
    let _ = CloseClipboard();
    out
}

unsafe fn set_clipboard(hwnd: HWND, cf: u32, bytes: &[u8]) -> bool {
    if !open(hwnd) {
        return false;
    }
    let _ = EmptyClipboard();
    let ok = (|| {
        let g = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let p = GlobalLock(g) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(g);
        SetClipboardData(cf, HANDLE(g.0)).ok()
    })()
    .is_some();
    let _ = CloseClipboard();
    ok
}

/// Decode a CF_DIB (BITMAPINFOHEADER + pixels) into a small premultiplied BGRA thumbnail.
fn dib_thumb(dib: &[u8], max: u32) -> Option<(u32, u32, Vec<u8>, (u32, u32))> {
    if dib.len() < 40 {
        return None;
    }
    let rd32 = |o: usize| i32::from_le_bytes(dib[o..o + 4].try_into().unwrap());
    let rd16 = |o: usize| u16::from_le_bytes(dib[o..o + 2].try_into().unwrap());
    let hsize = rd32(0) as usize;
    let (w, h) = (rd32(4), rd32(8));
    let bpp = rd16(14) as usize;
    let comp = rd32(16);
    let clr_used = rd32(32) as usize;
    if w <= 0 || h == 0 || !(bpp == 24 || bpp == 32) {
        return None;
    }
    let (w, top_down, ah) = (w as usize, h < 0, h.unsigned_abs() as usize);
    let mut off = hsize + clr_used * 4;
    if comp == 3 && hsize == 40 {
        off += 12;
    }
    let stride = (w * bpp / 8 + 3) & !3;
    if dib.len() < off + stride * ah {
        return None;
    }
    let s = (max as f32 / w.max(ah) as f32).min(1.0);
    let (tw, th) = (((w as f32 * s) as usize).max(1), ((ah as f32 * s) as usize).max(1));
    let alpha_used = bpp == 32 && (0..ah).any(|y| (0..w).any(|x| dib[off + y * stride + x * 4 + 3] != 0));
    let mut out = vec![0u8; tw * th * 4];
    for ty in 0..th {
        let sy = (ty * ah / th).min(ah - 1);
        let row = if top_down { sy } else { ah - 1 - sy };
        for tx in 0..tw {
            let sx = (tx * w / tw).min(w - 1);
            let p = off + row * stride + sx * bpp / 8;
            let (b, g, r) = (dib[p] as u32, dib[p + 1] as u32, dib[p + 2] as u32);
            let a = if alpha_used { dib[p + 3] as u32 } else { 255 };
            let o = (ty * tw + tx) * 4;
            out[o] = (b * a / 255) as u8;
            out[o + 1] = (g * a / 255) as u8;
            out[o + 2] = (r * a / 255) as u8;
            out[o + 3] = a as u8;
        }
    }
    Some((tw as u32, th as u32, out, (w as u32, ah as u32)))
}
