//! File shelf: drop files, links or text onto the notch; drag them back out anywhere.

use crate::store;
use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::{implement, w, Result, HRESULT, HSTRING};
use windows_core::Interface;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{
    BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, GENERIC_READ,
    GENERIC_WRITE, HGLOBAL, HWND, POINTL, SIZE, S_OK,
};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDIBits, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_ContainerFormatPng,
    GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGRA, IWICBitmapEncoder,
    IWICBitmapFrameEncode, IWICBitmapSource, IWICFormatConverter,
    IWICImagingFactory, IWICStream, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeHighQualityCubic, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
};
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
pub enum DropAction {
    Hold,     // 📥 Standard shelf storage
    Convert,  // 🖼️ PNG <-> JPG WIC conversion
    Compress, // 🗜️ Pack into .zip archive
    Ocr,      // 🔍 Extract text via Windows.Media.Ocr
}

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

    pub fn replace_or_add(&mut self, old_path: PathBuf, new_path: PathBuf) {
        if let Some(it) = self.items.iter_mut().find(|it| it.path == old_path) {
            it.path = new_path.clone();
            it.label = file_label(&new_path);
        } else {
            self.items.push(Item {
                kind: Kind::File,
                label: file_label(&new_path),
                path: new_path,
                content: String::new(),
            });
        }
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
        if let Ok(rd) = std::fs::read_dir(snippet_dir()) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        self.save();
    }

    pub fn convert_image(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.items.len() { return None; }
        let p = self.items[i].path.clone();
        convert_image_async(p.clone());
        Some(p)
    }

    pub fn zip_item(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.items.len() { return None; }
        let p = self.items[i].path.clone();
        compress_files_async(vec![p.clone()]);
        Some(p)
    }

    pub fn open(&self, i: usize) {
        self.open_item(i);
    }

    pub fn open_item(&self, i: usize) {
        let Some(it) = self.items.get(i) else { return };
        match it.kind {
            Kind::Link => {
                let u: Vec<u16> = it.content.encode_utf16().chain(Some(0)).collect();
                unsafe {
                    let _ = ShellExecuteW(None, w!("open"), windows::core::PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
                }
            }
            Kind::Text => {
                crate::app::copy_text(crate::events::target(), &it.content);
            }
            Kind::File => {
                let u: Vec<u16> = it.path.as_os_str().encode_wide().chain(Some(0)).collect();
                unsafe {
                    let _ = ShellExecuteW(None, w!("open"), windows::core::PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
                }
            }
        }
    }
}

fn label_for(kind: Kind, p: &Path, content: &str) -> String {
    match kind {
        Kind::File => file_label(p),
        Kind::Link => {
            let host = content.split("://").nth(1).unwrap_or(content).split('/').next().unwrap_or(content);
            host.trim_start_matches("www.").to_string()
        }
        Kind::Text => {
            let first = content.lines().next().unwrap_or("Snippet").trim();
            if first.is_empty() {
                "Snippet".into()
            } else {
                let mut chars = first.chars();
                let head: String = chars.by_ref().take(28).collect();
                if chars.next().is_some() {
                    format!("{head}…")
                } else {
                    head
                }
            }
        }
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

// ---------------------------------------------------------------- drop handling

#[derive(Clone, Debug)]
pub enum Dropped {
    Files(Vec<PathBuf>),
    Text(String),
}

fn url_format() -> u32 {
    static FMT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let f = FMT.load(std::sync::atomic::Ordering::Relaxed);
    if f != 0 {
        return f;
    }
    let u = unsafe { RegisterClipboardFormatW(w!("UniformResourceLocatorW")) };
    FMT.store(u, std::sync::atomic::Ordering::Relaxed);
    u
}

fn fmt(cf: u32) -> FORMATETC {
    FORMATETC {
        cfFormat: cf as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

unsafe fn global_text(h: HGLOBAL) -> String {
    if h.0.is_null() {
        return String::new();
    }
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

fn read_data(d: &IDataObject) -> Option<Dropped> {
    unsafe {
        let f = fmt(CF_HDROP.0.into());
        if d.QueryGetData(&f) == S_OK {
            if let Ok(mut m) = d.GetData(&f) {
                let hdrop = HDROP(m.u.hGlobal.0 as *mut _);
                let n = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
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
        for cf in [url_format(), CF_UNICODETEXT.0 as u32] {
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
    unsafe { [CF_HDROP.0 as u32, url_format(), CF_UNICODETEXT.0 as u32].iter().any(|&cf| d.QueryGetData(&fmt(cf)) == S_OK) }
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
    fn DragEnter(&self, d: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let ok = d.map(can_accept).unwrap_or(false);
        self.ok.set(ok);
        unsafe { *eff = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if ok {
            let has_images = d.and_then(read_data).map(|dropped| match dropped {
                Dropped::Files(files) => files.iter().any(|p| {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tiff")
                }),
                _ => false,
            }).unwrap_or(false);
            crate::app::with(|a| {
                a.drag_has_images = has_images;
                a.drag_enter();
                a.drag_over_pt(pt.x, pt.y);
            });
        }
        Ok(())
    }
    fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        unsafe { *eff = if self.ok.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if self.ok.get() {
            crate::app::with(|a| a.drag_over_pt(pt.x, pt.y));
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        crate::app::with(|a| a.drag_leave());
        Ok(())
    }
    fn Drop(&self, d: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let got = d.and_then(read_data);
        unsafe { *eff = if got.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        crate::app::with(|a| {
            a.drag_over_pt(pt.x, pt.y);
            a.dropped(got);
        });
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

pub fn icon_pixels(path: &Path, px: i32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let hbmp = f.GetImage(SIZE { cx: px, cy: px }, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK).ok()?;
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        let dc = GetDC(None);
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
        let len = w.checked_mul(h).and_then(|n| n.checked_mul(4))? as usize;
        if len == 0 || len > 16 * 1024 * 1024 {
            ReleaseDC(None, dc);
            let _ = DeleteObject(hbmp);
            return None;
        }
        let mut buf = vec![0u8; len];
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

// ---------------------------------------------------------------- WIC image conversion

struct ComGuard(bool);
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

enum ShelfJob {
    Convert(PathBuf),
    Ocr(PathBuf),
    Compress(Vec<PathBuf>),
}

static SHELF_TX: std::sync::OnceLock<std::sync::mpsc::Sender<ShelfJob>> = std::sync::OnceLock::new();

fn shelf_sender() -> &'static std::sync::mpsc::Sender<ShelfJob> {
    SHELF_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<ShelfJob>();
        std::thread::Builder::new()
            .name("shelf_worker".into())
            .spawn(move || {
                let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                let _guard = ComGuard(hr.is_ok());

                while let Ok(job) = rx.recv() {
                    match job {
                        ShelfJob::Convert(src_path) => {
                            match convert_image_wic(&src_path) {
                                Ok(new_path) => {
                                    crate::events::push(crate::events::Event::ShelfImageConverted {
                                        old_path: src_path,
                                        new_path,
                                    });
                                }
                                Err(e) => {
                                    crate::events::push(crate::events::Event::ShelfActionFailed(
                                        format!("Conversion failed: {}", friendly_io_error(&e)),
                                    ));
                                }
                            }
                        }
                        ShelfJob::Ocr(file_path) => {
                            match run_winrt_ocr(&file_path) {
                                Ok(text) => {
                                    let trimmed = text.trim().to_string();
                                    if trimmed.is_empty() {
                                        crate::events::push(crate::events::Event::ShelfActionFailed(
                                            "No text detected in image".into(),
                                        ));
                                    } else {
                                        crate::clipboard::set_text(&trimmed);
                                        crate::events::push(crate::events::Event::ShelfOcrSuccess {
                                            path: file_path,
                                            text: trimmed,
                                        });
                                    }
                                }
                                Err(e) => {
                                    crate::events::push(crate::events::Event::ShelfActionFailed(
                                        format!("OCR failed: {}", friendly_ocr_error(&e)),
                                    ));
                                }
                            }
                        }
                        ShelfJob::Compress(paths) => {
                            match compress_files_to_zip(&paths) {
                                Ok(zip_path) => {
                                    crate::events::push(crate::events::Event::ShelfZipCreated { path: zip_path });
                                }
                                Err(e) => {
                                    crate::events::push(crate::events::Event::ShelfActionFailed(
                                        format!("ZIP failed: {e}"),
                                    ));
                                }
                            }
                        }
                    }
                }
            })
            .ok();
        tx
    })
}

pub fn convert_image_async(src_path: PathBuf) {
    let _ = shelf_sender().send(ShelfJob::Convert(src_path));
}

fn friendly_io_error(e: &windows_core::Error) -> String {
    let code = e.code().0 as u32;
    match code {
        0x80070002 => "File not found".into(),
        0x80070005 => "Access denied".into(),
        0x80070020 => "File is locked by another process".into(),
        _ => "Unsupported image format or corrupt file".into(),
    }
}

fn friendly_ocr_error(e: &windows_core::Error) -> String {
    let code = e.code().0 as u32;
    match code {
        0x80070002 => "Image file not found".into(),
        0x80070005 => "Access denied to image file".into(),
        0x80070020 => "Image file is in use by another application".into(),
        0x80040154 => "Windows OCR components not installed".into(),
        _ => "No OCR language pack installed or unreadable format".into(),
    }
}

fn convert_image_wic(src: &Path) -> Result<PathBuf> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let to_jpeg = ext == "png";
    let target_ext = if to_jpeg { "jpg" } else { "png" };
    let container_format = if to_jpeg {
        GUID_ContainerFormatJpeg
    } else {
        GUID_ContainerFormatPng
    };
    let target_pixel_format = if to_jpeg {
        GUID_WICPixelFormat24bppBGR
    } else {
        GUID_WICPixelFormat32bppBGRA
    };

    let dst = unique_output_path(src, target_ext);
    let tmp_dst = dst.with_extension(format!("{target_ext}.tmp"));

    let encode_res = (|| unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

        let src_hstring = HSTRING::from(src.as_os_str());
        let decoder = factory.CreateDecoderFromFilename(
            &src_hstring,
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;

        let frame_decode = decoder.GetFrame(0)?;

        let bitmap_source: IWICBitmapSource = if to_jpeg {
            let conv_pre: IWICFormatConverter = factory.CreateFormatConverter()?;
            conv_pre.Initialize(
                &frame_decode,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let (mut w, mut h) = (0u32, 0u32);
            conv_pre.GetSize(&mut w, &mut h)?;
            let stride = w.checked_mul(4).ok_or_else(|| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "image too large"))?;
            let len = (stride as usize).checked_mul(h as usize).filter(|n| *n > 0 && *n <= 256 * 1024 * 1024)
                .ok_or_else(|| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "image too large"))?;
            let mut bgra = vec![0u8; len];
            conv_pre.CopyPixels(std::ptr::null(), stride, bgra.as_mut_slice())?;
            // Flatten onto white background so transparent PNGs do not encode as black in JPEG
            for chunk in bgra.chunks_exact_mut(4) {
                let a = chunk[3] as u32;
                if a < 255 {
                    let inv_a = 255 - a;
                    chunk[0] = ((chunk[0] as u32 * a + 255 * inv_a) / 255) as u8; // B
                    chunk[1] = ((chunk[1] as u32 * a + 255 * inv_a) / 255) as u8; // G
                    chunk[2] = ((chunk[2] as u32 * a + 255 * inv_a) / 255) as u8; // R
                    chunk[3] = 255;
                }
            }
            let white_bmp = factory.CreateBitmapFromMemory(
                w,
                h,
                &GUID_WICPixelFormat32bppBGRA,
                w * 4,
                &bgra,
            )?;
            white_bmp.cast()?
        } else {
            frame_decode.cast()?
        };

        let converter: IWICFormatConverter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &bitmap_source,
            &target_pixel_format,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let stream: IWICStream = factory.CreateStream()?;
        let tmp_hstring = HSTRING::from(tmp_dst.as_os_str());
        stream.InitializeFromFilename(&tmp_hstring, GENERIC_WRITE.0)?;

        let encoder: IWICBitmapEncoder =
            factory.CreateEncoder(&container_format, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;

        let mut frame_encode: Option<IWICBitmapFrameEncode> = None;
        encoder.CreateNewFrame(&mut frame_encode, std::ptr::null_mut())?;
        let frame_encode = frame_encode.unwrap();

        frame_encode.Initialize(None)?;

        let mut width = 0u32;
        let mut height = 0u32;
        converter.GetSize(&mut width, &mut height)?;
        frame_encode.SetSize(width, height)?;

        let mut actual_format = target_pixel_format;
        frame_encode.SetPixelFormat(&mut actual_format)?;

        frame_encode.WriteSource(&converter, std::ptr::null())?;
        frame_encode.Commit()?;
        encoder.Commit()?;
        Ok::<(), windows::core::Error>(())
    })();

    if let Err(e) = encode_res {
        let _ = std::fs::remove_file(&tmp_dst);
        return Err(e);
    }

    if std::fs::rename(&tmp_dst, &dst).is_err() {
        let _ = std::fs::remove_file(&tmp_dst);
        return Err(windows::core::Error::from_win32());
    }

    Ok(dst)
}

fn unique_output_path(src: &Path, new_ext: &str) -> PathBuf {
    let parent = src.parent().unwrap_or_else(|| Path::new("."));
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("converted");
    unique_path(parent, stem, new_ext)
}

fn unique_path(parent: &Path, stem: &str, ext: &str) -> PathBuf {
    let candidate = parent.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return candidate;
    }
    for i in 1..10_000 {
        let numbered = parent.join(format!("{stem} ({i}).{ext}"));
        if !numbered.exists() {
            return numbered;
        }
    }
    // Extremely unlikely fallback; still never returns an existing path.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!("{stem} ({n}).{ext}"))
}

// ---------------------------------------------------------------- WinRT OCR

pub fn extract_text_from_image_async(file_path: PathBuf) {
    let _ = shelf_sender().send(ShelfJob::Ocr(file_path));
}

fn run_winrt_ocr(path: &Path) -> Result<String> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _guard = ComGuard(hr.is_ok());

        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

        let path_hstr = HSTRING::from(path.as_os_str());
        let decoder = factory.CreateDecoderFromFilename(
            &path_hstr,
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;

        let converter: IWICFormatConverter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;

        let mut w = 0u32;
        let mut h = 0u32;
        converter.GetSize(&mut w, &mut h)?;

        let source: IWICBitmapSource = if w > 4096 || h > 4096 {
            let s = 4096.0 / (w.max(h) as f32);
            let target_w = ((w as f32 * s) as u32).max(1);
            let target_h = ((h as f32 * s) as u32).max(1);
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&converter, target_w, target_h, WICBitmapInterpolationModeHighQualityCubic)?;
            w = target_w;
            h = target_h;
            scaler.cast()?
        } else {
            converter.cast()?
        };

        let mut bgra_bytes = {
            let stride = w.checked_mul(4).ok_or_else(|| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "image too large"))?;
            let len = (stride as usize).checked_mul(h as usize).filter(|n| *n > 0 && *n <= 256 * 1024 * 1024)
                .ok_or_else(|| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "image too large"))?;
            vec![0u8; len]
        };
        source.CopyPixels(
            std::ptr::null(),
            w * 4,
            bgra_bytes.as_mut_slice(),
        )?;

        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra_bytes)?;
        let buffer = writer.DetachBuffer()?;

        let software_bm = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            w as i32,
            h as i32,
        )?;

        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
        let result = engine.RecognizeAsync(&software_bm)?.get()?;

        let lines = result.Lines()?;
        let mut full_text = String::new();
        for i in 0..lines.Size()? {
            let line = lines.GetAt(i)?;
            full_text.push_str(&line.Text()?.to_string());
            full_text.push('\n');
        }

        Ok(full_text)
    }
}

// ---------------------------------------------------------------- Native ZIP

const MAX_ZIP_DEPTH: usize = 16;

pub fn compress_files_to_zip(paths: &[PathBuf]) -> std::result::Result<PathBuf, String> {
    if paths.is_empty() {
        return Err("No files selected for compression".into());
    }

    let parent = paths[0]
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = if paths.len() == 1 {
        paths[0].file_stem().and_then(|s| s.to_str()).unwrap_or("Archive")
    } else {
        "Archive"
    };
    let zip_path = unique_path(parent, stem, "zip");

    // `create_new` guarantees a concurrent run cannot clobber a fresh archive.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&zip_path)
        .map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut seen: HashSet<String> = HashSet::new();
    let result = (|| -> std::result::Result<(), String> {
        for path in paths {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            if path.is_dir() {
                add_zip_dir(&mut zip, path, &name, &mut seen, 0, options)?;
            } else if path.is_file() {
                add_zip_file(&mut zip, path, &name, &mut seen, options)?;
            }
        }
        Ok(())
    })();

    if let Err(e) = result {
        // Never leave a half-written archive behind.
        drop(zip);
        let _ = std::fs::remove_file(&zip_path);
        return Err(e);
    }
    zip.finish().map_err(|e| {
        let _ = std::fs::remove_file(&zip_path);
        e.to_string()
    })?;
    Ok(zip_path)
}

fn add_zip_dir(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    prefix: &str,
    seen: &mut HashSet<String>,
    depth: usize,
    options: zip::write::SimpleFileOptions,
) -> std::result::Result<(), String> {
    if depth > MAX_ZIP_DEPTH {
        return Err("Folder nesting is too deep to archive".into());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        // Skip symlinks and junctions so a folder cannot pull in an entire drive.
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let arc = format!("{prefix}/{name}");
        if ft.is_dir() {
            add_zip_dir(zip, &path, &arc, seen, depth + 1, options)?;
        } else if ft.is_file() {
            add_zip_file(zip, &path, &arc, seen, options)?;
        }
    }
    Ok(())
}

fn add_zip_file(
    zip: &mut zip::ZipWriter<std::fs::File>,
    path: &Path,
    name: &str,
    seen: &mut HashSet<String>,
    options: zip::write::SimpleFileOptions,
) -> std::result::Result<(), String> {
    let name = unique_archive_name(seen, name);
    zip.start_file(&name, options).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::copy(&mut f, zip).map_err(|e| e.to_string())?;
    Ok(())
}

/// Archive entry names are case-insensitive on Windows; never add the same name twice.
fn unique_archive_name(seen: &mut HashSet<String>, name: &str) -> String {
    if seen.insert(name.to_ascii_lowercase()) {
        return name.to_string();
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    let parent = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    for i in 1..10_000 {
        let cand = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        let full = if parent.is_empty() { cand } else { format!("{parent}/{cand}") };
        if seen.insert(full.to_ascii_lowercase()) {
            return full;
        }
    }
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let fallback = if ext.is_empty() {
        format!("{stem} ({n})")
    } else {
        format!("{stem} ({n}).{ext}")
    };
    if parent.is_empty() {
        fallback
    } else {
        format!("{parent}/{fallback}")
    }
}

pub fn compress_files_async(paths: Vec<PathBuf>) {
    let _ = shelf_sender().send(ShelfJob::Compress(paths));
}

pub fn register(hwnd: HWND, target: &IDropTarget) {
    unsafe {
        let _ = RegisterDragDrop(hwnd, target);
    }
}
