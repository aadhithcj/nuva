//! File shelf: drop files, links or text onto the notch; drag them back out anywhere.

use crate::store;
use std::path::{Path, PathBuf};
use windows::core::{implement, w, Result, HRESULT, HSTRING};
use windows::Win32::Foundation::{
    BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND, POINTL, SIZE, S_OK,
};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDIBits, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MODIFIERKEYS_FLAGS, MK_LBUTTON};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DragQueryFileW, IShellItemImageFactory, ILFree, SHCreateDataObject, SHCreateItemFromParsingName,
    SHParseDisplayName, ShellExecuteW, HDROP, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Text,
    Link,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub path: PathBuf,
    pub label: String,
    /// For text/link items: the content (shown, and copied on click).
    pub content: String,
}

pub struct Shelf {
    pub items: Vec<Item>,
}

fn snippet_dir() -> PathBuf {
    let d = store::data_dir().join("shelf");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn file_label(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

impl Shelf {
    pub fn load(clear: bool) -> Shelf {
        let mut s = Shelf { items: Vec::new() };
        if clear {
            s.clear();
            return s;
        }
        for line in store::read("shelf.txt", false).lines() {
            let Some((k, p)) = line.split_once('\t') else { continue };
            let path = PathBuf::from(p);
            if !path.exists() {
                continue;
            }
            let kind = match k {
                "t" => Kind::Text,
                "l" => Kind::Link,
                _ => Kind::File,
            };
            let content = match kind {
                Kind::File => String::new(),
                Kind::Text => std::fs::read_to_string(&path).unwrap_or_default(),
                Kind::Link => std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .find_map(|l| l.strip_prefix("URL=").map(str::to_string))
                    .unwrap_or_default(),
            };
            s.items.push(Item { kind, label: label_for(kind, &path, &content), path, content });
        }
        s
    }

    fn save(&self) {
        let mut out = String::new();
        for it in &self.items {
            let k = match it.kind {
                Kind::File => "f",
                Kind::Text => "t",
                Kind::Link => "l",
            };
            out += &format!("{k}\t{}\n", it.path.display());
        }
        store::write("shelf.txt", false, &out);
    }

    pub fn add_file(&mut self, p: PathBuf) {
        if self.items.iter().any(|i| i.path == p) {
            return;
        }
        self.items.push(Item { kind: Kind::File, label: file_label(&p), path: p, content: String::new() });
        self.save();
    }

    pub fn add_text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let stamp = crate::time::utc_ticks();
        if is_url(text) {
            let host = text.split("://").nth(1).unwrap_or(text).split('/').next().unwrap_or("link");
            let safe: String = host.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect();
            let p = snippet_dir().join(format!("{safe} {}.url", stamp % 100000));
            let _ = std::fs::write(&p, format!("[InternetShortcut]\r\nURL={text}\r\n"));
            self.items.push(Item { kind: Kind::Link, label: label_for(Kind::Link, &p, text), path: p, content: text.into() });
        } else {
            let p = snippet_dir().join(format!("Snippet {}.txt", stamp % 100000));
            let _ = std::fs::write(&p, text);
            self.items.push(Item { kind: Kind::Text, label: label_for(Kind::Text, &p, text), path: p, content: text.into() });
        }
        self.save();
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            let it = self.items.remove(i);
            if it.kind != Kind::File {
                let _ = std::fs::remove_file(&it.path);
            }
            self.save();
        }
    }

    pub fn clear(&mut self) {
        for it in self.items.drain(..) {
            if it.kind != Kind::File {
                let _ = std::fs::remove_file(&it.path);
            }
        }
        // Remove any stray snippets from previous sessions.
        if let Ok(rd) = std::fs::read_dir(snippet_dir()) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        self.save();
    }

    pub fn convert_image(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.items.len() { return None; }
        let p = &self.items[i].path;
        let ext = p.extension()?.to_str()?.to_lowercase();
        let target_ext = if ext == "png" { "jpg" } else { "png" };
        let out_name = format!("{}_converted.{}", p.file_stem()?.to_str()?, target_ext);
        let out_path = p.parent()?.join(&out_name);

        let ps_cmd = format!(
            "Add-Type -AssemblyName System.Drawing; $b = [System.Drawing.Bitmap]::FromFile('{}'); $fmt = if ('{}' -eq 'jpg') {{ [System.Drawing.Imaging.ImageFormat]::Jpeg }} else {{ [System.Drawing.Imaging.ImageFormat]::Png }}; $b.Save('{}', $fmt); $b.Dispose()",
            p.display(), target_ext, out_path.display()
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .status()
            .ok()?;
        if status.success() && out_path.exists() {
            self.items.push(Item {
                kind: Kind::File,
                label: file_label(&out_path),
                path: out_path.clone(),
                content: String::new(),
            });
            self.save();
            Some(out_path)
        } else {
            None
        }
    }

    pub fn zip_item(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.items.len() { return None; }
        let p = &self.items[i].path;
        let out_name = format!("{}.zip", p.file_stem()?.to_str()?);
        let out_path = p.parent()?.join(&out_name);

        let ps_cmd = format!(
            "Compress-Archive -Path '{}' -DestinationPath '{}' -Force",
            p.display(), out_path.display()
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .status()
            .ok()?;
        if status.success() && out_path.exists() {
            self.items.push(Item {
                kind: Kind::File,
                label: file_label(&out_path),
                path: out_path.clone(),
                content: String::new(),
            });
            self.save();
            Some(out_path)
        } else {
            None
        }
    }

    pub fn open(&self, i: usize) {
        if let Some(it) = self.items.get(i) {
            let target = if it.kind == Kind::Link { HSTRING::from(it.content.as_str()) } else { HSTRING::from(it.path.as_os_str()) };
            unsafe {
                ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL);
            }
        }
    }
}

fn is_url(s: &str) -> bool {
    !s.contains(char::is_whitespace) && (s.starts_with("http://") || s.starts_with("https://") || s.starts_with("www."))
}

fn label_for(kind: Kind, path: &Path, content: &str) -> String {
    match kind {
        Kind::File => file_label(path),
        Kind::Link => content.split("://").nth(1).unwrap_or(content).trim_start_matches("www.").to_string(),
        Kind::Text => content.lines().next().unwrap_or("").chars().take(60).collect(),
    }
}

// ---------------------------------------------------------------- drop target

pub enum Dropped {
    Files(Vec<PathBuf>),
    Text(String),
}

fn fmt(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn url_format() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("UniformResourceLocatorW")) as u16 }
}

unsafe fn global_text(h: HGLOBAL) -> String {
    let p = GlobalLock(h) as *const u16;
    if p.is_null() {
        return String::new();
    }
    let max = GlobalSize(h) / 2;
    let mut n = 0;
    while n < max && *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
    let _ = GlobalUnlock(h);
    s
}

pub fn read_data(d: &IDataObject) -> Option<Dropped> {
    unsafe {
        let f = fmt(CF_HDROP.0);
        if d.QueryGetData(&f) == S_OK {
            if let Ok(mut m) = d.GetData(&f) {
                let hdrop = HDROP(m.u.hGlobal.0);
                let n = DragQueryFileW(hdrop, u32::MAX, None);
                let mut files = Vec::new();
                for i in 0..n {
                    let len = DragQueryFileW(hdrop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(hdrop, i, Some(&mut buf));
                    files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
                }
                ReleaseStgMedium(&mut m);
                return Some(Dropped::Files(files));
            }
        }
        for cf in [url_format(), CF_UNICODETEXT.0] {
            let f = fmt(cf);
            if d.QueryGetData(&f) == S_OK {
                if let Ok(mut m) = d.GetData(&f) {
                    let s = global_text(m.u.hGlobal);
                    ReleaseStgMedium(&mut m);
                    if !s.trim().is_empty() {
                        return Some(Dropped::Text(s));
                    }
                }
            }
        }
        None
    }
}

fn can_accept(d: &IDataObject) -> bool {
    unsafe { [CF_HDROP.0, url_format(), CF_UNICODETEXT.0].iter().any(|&cf| d.QueryGetData(&fmt(cf)) == S_OK) }
}

#[implement(IDropTarget)]
pub struct DropTarget {
    ok: std::cell::Cell<bool>,
}

impl DropTarget {
    pub fn new() -> IDropTarget {
        DropTarget { ok: std::cell::Cell::new(false) }.into()
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, d: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let ok = d.map(can_accept).unwrap_or(false);
        self.ok.set(ok);
        unsafe { *eff = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if ok {
            crate::app::with(|a| a.drag_enter());
        }
        Ok(())
    }
    fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        unsafe { *eff = if self.ok.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        crate::app::with(|a| a.drag_leave());
        Ok(())
    }
    fn Drop(&self, d: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let got = d.and_then(read_data);
        unsafe { *eff = if got.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        crate::app::with(|a| a.dropped(got));
        Ok(())
    }
}

// ---------------------------------------------------------------- drag out

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, esc: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if esc.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _e: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Start a shell drag of `paths`. Blocks until the drop finishes.
pub fn drag_out(paths: &[PathBuf]) -> bool {
    unsafe {
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for p in paths {
            let mut pidl = std::ptr::null_mut();
            if SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        if pidls.is_empty() {
            return false;
        }
        let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
        let res = match SHCreateDataObject::<_, IDataObject>(None, Some(&consts), None) {
            Ok(obj) => {
                let src: IDropSource = DropSource.into();
                let mut eff = DROPEFFECT_NONE;
                let hr = DoDragDrop(&obj, &src, DROPEFFECT_COPY | DROPEFFECT_LINK | DROPEFFECT_MOVE, &mut eff);
                hr == DRAGDROP_S_DROP
            }
            Err(_) => false,
        };
        for p in pidls {
            ILFree(Some(p));
        }
        res
    }
}

// ---------------------------------------------------------------- icons

/// Shell icon/thumbnail for a path as premultiplied BGRA pixels.
pub fn icon_pixels(path: &Path, px: i32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbmp = f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK).ok()?;
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        let dc = GetDC(None);
        // First call fills in the size.
        GetDIBits(dc, hbmp, 0, 0, None, &mut bi, DIB_RGB_COLORS);
        let (w, h) = (bi.bmiHeader.biWidth.unsigned_abs(), bi.bmiHeader.biHeight.unsigned_abs());
        if w == 0 || h == 0 || w > 1024 || h > 1024 {
            ReleaseDC(None, dc);
            let _ = DeleteObject(hbmp);
            return None;
        }
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        bi.bmiHeader.biHeight = -(h as i32);
        bi.bmiHeader.biPlanes = 1;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(dc, hbmp, 0, h, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        let _ = DeleteObject(hbmp);
        if lines == 0 {
            return None;
        }
        let any_alpha = buf.chunks(4).any(|p| p[3] != 0);
        let straight = buf.chunks(4).any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]);
        for p in buf.chunks_mut(4) {
            if !any_alpha {
                p[3] = 255;
            } else if straight {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = (*c as u32 * a / 255) as u8;
                }
            }
        }
        Some((w, h, buf))
    }
}

pub fn register(hwnd: HWND, target: &IDropTarget) {
    unsafe {
        let _ = RegisterDragDrop(hwnd, target);
    }
}
