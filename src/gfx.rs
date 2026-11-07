//! Direct2D / DirectWrite / DirectComposition plumbing and an immediate-mode canvas.

use crate::ui::Action;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;
use windows::core::{Interface, Result, HSTRING};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

// ---------------------------------------------------------------- geometry & colour

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn r(&self) -> f32 {
        self.x + self.w
    }
    pub fn b(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.r() && p.1 >= self.y && p.1 < self.b()
    }
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    pub fn split_left(&self, w: f32) -> (Rect, Rect) {
        (Rect::new(self.x, self.y, w, self.h), Rect::new(self.x + w, self.y, (self.w - w).max(0.0), self.h))
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.r(), bottom: self.b() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const fn hex(v: u32) -> Color {
        Color(
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
            1.0,
        )
    }
    pub const fn a(self, a: f32) -> Color {
        Color(self.0, self.1, self.2, self.3 * a)
    }
    pub fn mix(self, o: Color, t: f32) -> Color {
        Color(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
            self.3 + (o.3 - self.3) * t,
        )
    }
}

pub mod pal {
    use super::Color;
    pub const BLACK: Color = Color::hex(0x000000);
    pub const WHITE: Color = Color::hex(0xffffff);
    pub const TEXT: Color = Color::hex(0xf5f5f7);
    pub const DIM: Color = Color::hex(0x9a9aa2);
    pub const FAINT: Color = Color::hex(0x5c5c63);
    pub const CARD: Color = Color::hex(0x1c1c1f);
    pub const CARD_HI: Color = Color::hex(0x2a2a2e);
    pub const LINE: Color = Color::hex(0x333338);
    pub const ACCENT: Color = Color::hex(0x7c8cff);
    pub const GREEN: Color = Color::hex(0x4cd97b);
    pub const ORANGE: Color = Color::hex(0xff9f43);
    pub const RED: Color = Color::hex(0xff5a5f);
    pub const WATER: Color = Color::hex(0x4fb3ff);
    pub const PINK: Color = Color::hex(0xff7ab6);
    pub const YELLOW: Color = Color::hex(0xffd166);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum W {
    Regular = 400,
    Medium = 500,
    Semi = 600,
    Bold = 700,
}

/// Icon glyphs shared by Segoe Fluent Icons (Win 11) and Segoe MDL2 Assets (Win 10).
#[allow(dead_code)]
pub mod ico {
    pub const PLAY: char = '\u{E768}';
    pub const PAUSE: char = '\u{E769}';
    pub const NEXT: char = '\u{E893}';
    pub const PREV: char = '\u{E892}';
    pub const MUSIC: char = '\u{E8D6}';
    pub const MIC: char = '\u{E720}';
    pub const TIMER: char = '\u{E916}';
    pub const CLOCK: char = '\u{E823}';
    pub const CHECKLIST: char = '\u{E9D5}';
    pub const CALENDAR: char = '\u{E787}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const CLIPBOARD: char = '\u{E77F}';
    pub const BELL: char = '\u{EA8F}';
    pub const STATS: char = '\u{E9D9}';
    pub const VOLUME: char = '\u{E767}';
    pub const MUTE: char = '\u{E74F}';
    pub const BRIGHT: char = '\u{E706}';
    pub const KEYBOARD: char = '\u{E765}';
    pub const WIFI: char = '\u{E701}';
    pub const WARN: char = '\u{E7BA}';
    pub const BLUETOOTH: char = '\u{E702}';
    pub const HEADPHONE: char = '\u{E7F6}';
    pub const SETTINGS: char = '\u{E713}';
    pub const CLOSE: char = '\u{E711}';
    pub const ADD: char = '\u{E710}';
    pub const DELETE: char = '\u{E74D}';
    pub const COPY: char = '\u{E8C8}';
    pub const EDIT: char = '\u{E70F}';
    pub const PIN: char = '\u{E718}';
    pub const CHECK: char = '\u{E73E}';
    pub const LEFT: char = '\u{E76B}';
    pub const RIGHT: char = '\u{E76C}';
    pub const UP: char = '\u{E70E}';
    pub const DOWN: char = '\u{E70D}';
    pub const REFRESH: char = '\u{E72C}';
    pub const SKIP: char = '\u{E893}';
    pub const LINK: char = '\u{E71B}';
    pub const TEXT: char = '\u{E8D2}';
    pub const OPEN: char = '\u{E8A7}';
    pub const SNOOZE: char = '\u{E708}';
    pub const HOME: char = '\u{E80F}';
    pub const MAIL: char = '\u{E715}';
    pub const CAPS: char = '\u{E84A}';
    pub const BATTERY_CHARGING: char = '\u{EA93}';
    pub const PLUG: char = '\u{E83E}';
    pub const IMAGE: char = '\u{EB9F}';
    pub const DROP: char = '\u{EB42}';
    pub const INFO: char = '\u{E946}';
    pub const DISPLAY: char = '\u{E7F4}';
    pub const ART: char = '\u{E790}';
    pub const CPU: char = '\u{E950}';
    pub const GLOBE: char = '\u{E774}';
    pub const LOCK: char = '\u{E72E}';
    pub const MOON: char = '\u{E708}';
    pub const MIC_OFF: char = '\u{EC54}';
    pub const EYEDROPPER: char = '\u{EF3C}';
    pub const SEARCH: char = '\u{E721}';
    pub const CALCULATOR: char = '\u{E8EF}';
    pub const DOWNLOAD: char = '\u{E896}';
    pub const ZIP: char = '\u{F012}';
    pub const CONVERT: char = '\u{E8AB}';
    pub const SPEAKER: char = '\u{E7F5}';
}

pub fn battery_glyph(pct: u32, charging: bool) -> char {
    let lvl = (pct.min(100) / 10) as u32;
    char::from_u32(if charging { 0xEBAB } else { 0xEBA0 } + lvl).unwrap_or('?')
}

// ---------------------------------------------------------------- factories

#[derive(Clone)]
pub struct Gfx {
    pub d2d: ID2D1Factory1,
    pub dw: IDWriteFactory,
    pub wic: IWICImagingFactory,
    formats: Rc<RefCell<HashMap<(u32, u32, bool, Align, bool), IDWriteTextFormat>>>,
    font: HSTRING,
    icon_font: HSTRING,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let has = |name: &str| -> bool {
                let mut coll = None;
                if dw.GetSystemFontCollection(&mut coll, false).is_err() {
                    return false;
                }
                let Some(coll) = coll else { return false };
                let (mut idx, mut exists) = (0u32, BOOL(0));
                coll.FindFamilyName(&HSTRING::from(name), &mut idx, &mut exists).is_ok() && exists.as_bool()
            };
            let font = if has("Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
            let icon_font = if has("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
            Ok(Gfx {
                d2d,
                dw,
                wic,
                formats: Rc::new(RefCell::new(HashMap::new())),
                font: HSTRING::from(font),
                icon_font: HSTRING::from(icon_font),
            })
        }
    }

    pub fn format(&self, size: f32, weight: W, icon: bool, align: Align, wrap: bool) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32, weight as u32, icon, align, wrap);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let f = self
                .dw
                .CreateTextFormat(
                    if icon { &self.icon_font } else { &self.font },
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    &HSTRING::from("en-us"),
                )
                .ok()?;
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if wrap {
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            } else {
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                    let t = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                    let _ = f.SetTrimming(&t, &sign);
                }
            }
            self.formats.borrow_mut().insert(key, f.clone());
            Some(f)
        }
    }

    /// Decode an encoded image (PNG/JPEG/…) to a premultiplied WIC source, scaled to fit `max_px`.
    pub fn decode(&self, bytes: &[u8], max_px: u32) -> Option<IWICBitmapSource> {
        unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(bytes).ok()?;
            let dec = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad).ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h).ok()?;
            let mut src: IWICBitmapSource = frame.cast().ok()?;
            if w > max_px || h > max_px {
                let s = max_px as f32 / w.max(h) as f32;
                let scaler = self.wic.CreateBitmapScaler().ok()?;
                scaler
                    .Initialize(&src, ((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1), WICBitmapInterpolationModeHighQualityCubic)
                    .ok()?;
                src = scaler.cast().ok()?;
            }
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
                .ok()?;
            // Materialise so the decoder/stream (and `bytes`) are no longer needed.
            let bmp = self.wic.CreateBitmapFromSource(&conv, WICBitmapCacheOnLoad).ok()?;
            bmp.cast().ok()
        }
    }

    pub fn decode_bgra_pixels(&self, bytes: &[u8], max_px: u32) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(bytes).ok()?;
            let dec = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad).ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h).ok()?;
            let mut src: IWICBitmapSource = frame.cast().ok()?;
            if w > max_px || h > max_px {
                let s = max_px as f32 / w.max(h) as f32;
                let target_w = ((w as f32 * s) as u32).max(1);
                let target_h = ((h as f32 * s) as u32).max(1);
                let scaler = self.wic.CreateBitmapScaler().ok()?;
                scaler.Initialize(&src, target_w, target_h, WICBitmapInterpolationModeHighQualityCubic).ok()?;
                src = scaler.cast().ok()?;
                w = target_w;
                h = target_h;
            }
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(&src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
            let mut buf = vec![0u8; (w * h * 4) as usize];
            conv.CopyPixels(std::ptr::null(), w * 4, &mut buf).ok()?;
            Some((w, h, buf))
        }
    }
}

pub fn bitmap_from_wic(rt: &ID2D1RenderTarget, src: &IWICBitmapSource) -> Option<ID2D1Bitmap> {
    unsafe { rt.CreateBitmapFromWicBitmap(src, None).ok() }
}

/// Create a D2D bitmap from premultiplied BGRA pixels.
pub fn bitmap_from_bgra(rt: &ID2D1RenderTarget, w: u32, h: u32, px: &[u8]) -> Option<ID2D1Bitmap> {
    unsafe {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        rt.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(px.as_ptr() as *const _), w * 4, &props).ok()
    }
}

// ---------------------------------------------------------------- composition surface

/// A premultiplied-alpha swap chain composed onto a window with DirectComposition.
pub struct CompSurface {
    pub dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _d3d: ID3D11Device,
    dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    pub w: u32,
    pub h: u32,
}

impl CompSurface {
    pub fn new(gfx: &Gfx, hwnd: HWND, w: u32, h: u32) -> Result<CompSurface> {
        unsafe {
            let mut d3d = None;
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            if D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, None, flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None)
                .is_err()
            {
                D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, None, flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None)?;
            }
            let d3d = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w.max(1),
                Height: h.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForComposition(&d3d, &desc, None)?;
            let dev = gfx.d2d.CreateDevice(&dxgi)?;
            let dc = dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            let s = CompSurface { dc, swap, _d3d: d3d, dcomp, _target: target, _visual: visual, w: w.max(1), h: h.max(1) };
            s.bind()?;
            Ok(s)
        }
    }

    fn bind(&self) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
        }
        Ok(())
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        if w == self.w && h == self.h {
            return Ok(());
        }
        unsafe {
            self.dc.SetTarget(None);
            self.swap.ResizeBuffers(2, w.max(1), h.max(1), DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.w = w.max(1);
        self.h = h.max(1);
        self.bind()
    }

    pub fn present(&self) -> Result<()> {
        unsafe {
            self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            self.dcomp.Commit()
        }
    }
}

// ---------------------------------------------------------------- canvas

pub struct Canvas {
    pub rt: ID2D1RenderTarget,
    pub gfx: Gfx,
    brush: ID2D1SolidColorBrush,
    pub hits: Vec<(Rect, Action)>,
    pub mouse: Option<(f32, f32)>,
    pub alpha: f32,
}

fn cf(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.0, g: c.1, b: c.2, a: c.3 }
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

impl Canvas {
    pub fn new(rt: ID2D1RenderTarget, gfx: Gfx, mouse: Option<(f32, f32)>) -> Result<Canvas> {
        let brush = unsafe { rt.CreateSolidColorBrush(&cf(pal::WHITE), None)? };
        Ok(Canvas { rt, gfx, brush, hits: Vec::new(), mouse, alpha: 1.0 })
    }

    fn b(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&cf(c.a(self.alpha))) };
        &self.brush
    }

    pub fn clear(&self, c: Color) {
        unsafe { self.rt.Clear(Some(&cf(c))) };
    }

    pub fn rect(&self, r: Rect, c: Color) {
        unsafe { self.rt.FillRectangle(&r.d2d(), self.b(c)) };
    }

    pub fn rrect(&self, r: Rect, rad: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.b(c)) };
    }

    pub fn rrect_stroke(&self, r: Rect, rad: f32, c: Color, w: f32) {
        let rr = D2D1_ROUNDED_RECT { rect: r.inset(w / 2.0, w / 2.0).d2d(), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.b(c), w, None) };
    }

    pub fn circle(&self, cx: f32, cy: f32, rad: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rad, radiusY: rad };
        unsafe { self.rt.FillEllipse(&e, self.b(c)) };
    }

    #[allow(dead_code)]
    pub fn ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rx, radiusY: ry };
        unsafe { self.rt.FillEllipse(&e, self.b(c)) };
    }

    pub fn ring(&self, cx: f32, cy: f32, rad: f32, w: f32, c: Color) {
        let e = D2D1_ELLIPSE { point: pt(cx, cy), radiusX: rad, radiusY: rad };
        unsafe { self.rt.DrawEllipse(&e, self.b(c), w, None) };
    }

    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color, w: f32) {
        unsafe { self.rt.DrawLine(pt(x1, y1), pt(x2, y2), self.b(c), w, &self.round_stroke()) };
    }

    fn round_stroke(&self) -> ID2D1StrokeStyle {
        thread_local! { static S: RefCell<Option<ID2D1StrokeStyle>> = const { RefCell::new(None) }; }
        S.with(|s| {
            if s.borrow().is_none() {
                let props = D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 1.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                };
                let f: ID2D1Factory = unsafe { self.rt.GetFactory() }.unwrap();
                *s.borrow_mut() = unsafe { f.CreateStrokeStyle(&props, None) }.ok();
            }
            s.borrow().clone().unwrap()
        })
    }

    /// Arc stroke from angle 0 = 12 o'clock, clockwise, covering `frac` of the circle.
    pub fn arc(&self, cx: f32, cy: f32, rad: f32, w: f32, frac: f32, c: Color) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.0005 {
            return;
        }
        if frac >= 0.9995 {
            self.ring(cx, cy, rad, w, c);
            return;
        }
        unsafe {
            let Ok(geo) = self.gfx.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            let a = frac * std::f32::consts::TAU;
            sink.BeginFigure(pt(cx, cy - rad), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: pt(cx + rad * a.sin(), cy - rad * a.cos()),
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.rt.DrawGeometry(&geo, self.b(c), w, &self.round_stroke());
        }
    }

    pub fn fill_path(&self, pts: &[(f32, f32)], c: Color) {
        if pts.len() < 3 {
            return;
        }
        unsafe {
            let Ok(geo) = self.gfx.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(pt(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts[1..] {
                sink.AddLine(pt(p.0, p.1));
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.rt.FillGeometry(&geo, self.b(c), None);
        }
    }

    pub fn polyline(&self, pts: &[(f32, f32)], c: Color, w: f32) {
        if pts.len() < 2 {
            return;
        }
        unsafe {
            let Ok(geo) = self.gfx.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(pt(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
            for p in &pts[1..] {
                sink.AddLine(pt(p.0, p.1));
            }
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.rt.DrawGeometry(&geo, self.b(c), w, &self.round_stroke());
        }
    }

    /// The notch silhouette: flat top, rounded bottom corners, concave "flares" into the screen edge.
    pub fn notch_shape(&self, x0: f32, x1: f32, h: f32, r: f32, flare: f32, c: Color) {
        let r = r.min(h / 2.0).min((x1 - x0) / 2.0).max(0.0);
        let f = flare.min(h / 2.0);
        unsafe {
            let Ok(geo) = self.gfx.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            let arc = |p: D2D_POINT_2F, rad: f32, dir| D2D1_ARC_SEGMENT {
                point: p,
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: dir,
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            sink.BeginFigure(pt(x0 - f, 0.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddArc(&arc(pt(x0, f), f, D2D1_SWEEP_DIRECTION_CLOCKWISE));
            sink.AddLine(pt(x0, h - r));
            sink.AddArc(&arc(pt(x0 + r, h), r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
            sink.AddLine(pt(x1 - r, h));
            sink.AddArc(&arc(pt(x1, h - r), r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
            sink.AddLine(pt(x1, f));
            sink.AddArc(&arc(pt(x1 + f, 0.0), f, D2D1_SWEEP_DIRECTION_CLOCKWISE));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.rt.FillGeometry(&geo, self.b(c), None);
        }
    }

    pub fn text(&self, s: &str, r: Rect, size: f32, w: W, c: Color, align: Align) {
        self.text_inner(s, r, size, w, c, align, false, false);
    }

    pub fn text_wrap(&self, s: &str, r: Rect, size: f32, w: W, c: Color, align: Align) {
        self.text_inner(s, r, size, w, c, align, true, false);
    }

    pub fn icon(&self, g: char, r: Rect, size: f32, c: Color) {
        let mut buf = [0u16; 2];
        self.text_inner(g.encode_utf16(&mut buf), r, size, W::Regular, c, Align::Center, false, true);
    }

    fn text_inner<S: AsText + ?Sized>(&self, s: &S, r: Rect, size: f32, w: W, c: Color, align: Align, wrap: bool, icon: bool) {
        let Some(f) = self.gfx.format(size, w, icon, align, wrap) else { return };
        let u = s.utf16();
        if u.is_empty() {
            return;
        }
        unsafe {
            self.rt.DrawText(&u, &f, &r.d2d(), self.b(c), D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    pub fn measure(&self, s: &str, size: f32, w: W) -> f32 {
        let Some(f) = self.gfx.format(size, w, false, Align::Left, true) else { return 0.0 };
        let u: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(l) = self.gfx.dw.CreateTextLayout(&u, &f, 10000.0, 1000.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    /// Height of wrapped text at a given width.
    pub fn measure_h(&self, s: &str, width: f32, size: f32, w: W) -> f32 {
        let Some(f) = self.gfx.format(size, w, false, Align::Left, true) else { return 0.0 };
        let u: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(l) = self.gfx.dw.CreateTextLayout(&u, &f, width.max(1.0), 10000.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.height
        }
    }

    pub fn image(&self, bmp: &ID2D1Bitmap, r: Rect, rad: f32) {
        unsafe {
            if rad > 0.0 {
                if let Ok(geo) = self.gfx.d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: rad, radiusY: rad }) {
                    let params = D2D1_LAYER_PARAMETERS {
                        contentBounds: r.d2d(),
                        geometricMask: ManuallyDrop::new(Some(geo.cast().unwrap())),
                        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                        maskTransform: Matrix3x2::identity(),
                        opacity: 1.0,
                        opacityBrush: ManuallyDrop::new(None),
                        layerOptions: D2D1_LAYER_OPTIONS_NONE,
                    };
                    self.rt.PushLayer(&params, None);
                    self.rt.DrawBitmap(bmp, Some(&r.d2d()), self.alpha, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
                    self.rt.PopLayer();
                    ManuallyDrop::into_inner(params.geometricMask);
                    return;
                }
            }
            self.rt.DrawBitmap(bmp, Some(&r.d2d()), self.alpha, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
    }

    pub fn clip(&self, r: Rect) {
        unsafe { self.rt.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    pub fn unclip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }

    // ------------------------------------------------ interaction helpers

    pub fn hover(&self, r: Rect) -> bool {
        self.mouse.map(|m| r.contains(m)).unwrap_or(false)
    }

    pub fn hit(&mut self, r: Rect, a: Action) -> bool {
        self.hits.push((r, a));
        self.hover(r)
    }

    pub fn icon_button(&mut self, r: Rect, g: char, size: f32, a: Action, c: Color) {
        let h = self.hit(r, a);
        if h {
            self.rrect(r, r.h.min(r.w) / 2.0, pal::WHITE.a(0.12));
        }
        self.icon(g, r, size, c);
    }

    pub fn button(&mut self, r: Rect, label: &str, a: Action, fill: Color, fg: Color) {
        let h = self.hit(r, a);
        let fill = if h { fill.mix(pal::WHITE, 0.15) } else { fill };
        self.rrect(r, r.h / 2.0, fill);
        self.text(label, r, 12.5, W::Semi, fg, Align::Center);
    }

    pub fn toggle(&mut self, r: Rect, on: bool, a: Action) {
        // `r` is the whole row (for a big hit target); the switch sits at its right edge.
        self.hit(r, a);
        let sw = Rect::new(r.r() - 40.0, r.cy() - 11.0, 40.0, 22.0);
        self.rrect(sw, 11.0, if on { pal::ACCENT } else { pal::CARD_HI });
        let kx = if on { sw.r() - 11.0 } else { sw.x + 11.0 };
        self.circle(kx, sw.cy(), 8.0, pal::WHITE);
    }

    pub fn segmented(&mut self, r: Rect, opts: &[&str], sel: usize, mk: &dyn Fn(usize) -> Action) {
        self.rrect(r, 8.0, pal::CARD_HI);
        let w = r.w / opts.len() as f32;
        for (i, o) in opts.iter().enumerate() {
            let cell = Rect::new(r.x + w * i as f32, r.y, w, r.h).inset(2.0, 2.0);
            let h = self.hit(cell, mk(i));
            if i == sel {
                self.rrect(cell, 6.0, pal::ACCENT);
            } else if h {
                self.rrect(cell, 6.0, pal::WHITE.a(0.08));
            }
            self.text(o, cell, 12.0, if i == sel { W::Semi } else { W::Regular }, pal::TEXT, Align::Center);
        }
    }

    pub fn stepper(&mut self, r: Rect, value: &str, dec: Action, inc: Action) {
        self.rrect(r, 8.0, pal::CARD_HI);
        let (l, rest) = r.split_left(28.0);
        let (mid, rr) = rest.split_left(rest.w - 28.0);
        self.icon_button(l.inset(2.0, 2.0), '\u{E738}', 10.0, dec, pal::TEXT);
        self.text(value, mid, 12.5, W::Semi, pal::TEXT, Align::Center);
        self.icon_button(rr.inset(2.0, 2.0), ico::ADD, 10.0, inc, pal::TEXT);
    }

    pub fn bar(&self, r: Rect, frac: f32, bg: Color, fg: Color) {
        self.rrect(r, r.h / 2.0, bg);
        let f = frac.clamp(0.0, 1.0);
        if f > 0.0 {
            self.rrect(Rect::new(r.x, r.y, (r.w * f).max(r.h), r.h), r.h / 2.0, fg);
        }
    }
}

pub trait AsText {
    fn utf16(&self) -> Vec<u16>;
}
impl AsText for str {
    fn utf16(&self) -> Vec<u16> {
        self.encode_utf16().collect()
    }
}
impl AsText for [u16] {
    fn utf16(&self) -> Vec<u16> {
        self.to_vec()
    }
}
