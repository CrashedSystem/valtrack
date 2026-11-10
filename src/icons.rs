//! Agent / rank icons.
//!
//! The PNGs are embedded into the binary (`assets.rs`), so rendering never
//! touches the network and there is no first-paint stall.
//!
//! The icons ship at 64x64 (agents) and 32x32 (ranks) but are drawn at 24 and
//! 20 px. Handing that reduction to `AlphaBlend` makes GDI drop pixels instead
//! of blending them, which is why the artwork looked stair-stepped. So each
//! icon is resampled once, with Lanczos, to exactly the size it is drawn at,
//! and premultiplied so `AC_SRC_ALPHA` blends without a bright fringe. Drawing
//! is then a 1:1 blit: no scaling, no resampling, no artefacts.

use crate::assets;
use image::imageops::FilterType;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};

#[repr(C)]
struct BlendFunction {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[link(name = "msimg32")]
extern "system" {
    fn AlphaBlend(
        hdc_dest: HDC,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        hdc_src: HDC,
        x1: i32,
        y1: i32,
        w1: i32,
        h1: i32,
        blend: BlendFunction,
    ) -> i32;
}

pub struct Icon {
    pub w: i32,
    pub h: i32,
    dc: HDC,
    bmp: HBITMAP,
}

impl Icon {
    /// Decodes `bytes`, resamples to `size` x `size` and premultiplies.
    fn from_png(bytes: &[u8], size: i32) -> Option<Self> {
        let src = image::load_from_memory(bytes).ok()?.to_rgba8();
        let size = size.max(1) as u32;
        let img = if src.width() != size || src.height() != size {
            image::imageops::resize(&src, size, size, FilterType::Lanczos3)
        } else {
            src
        };
        let (w, h) = img.dimensions();
        let rgba = img.as_raw();

        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return None;
            }

            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(
                screen,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(std::ptr::null_mut(), screen);
            if bmp.is_null() || dc.is_null() || bits.is_null() {
                return None;
            }
            SelectObject(dc, bmp as HGDIOBJ);

            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for i in 0..(w * h) as usize {
                let a = rgba[i * 4 + 3] as u32;
                // Premultiply so AC_SRC_ALPHA composites correctly. Without it
                // the anti-aliased edge pixels stay at full intensity and read
                // as a light halo against the dark panel.
                let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                dst[i * 4] = pm(rgba[i * 4 + 2]);
                dst[i * 4 + 1] = pm(rgba[i * 4 + 1]);
                dst[i * 4 + 2] = pm(rgba[i * 4]);
                dst[i * 4 + 3] = a as u8;
            }

            Some(Self {
                w: w as i32,
                h: h as i32,
                dc,
                bmp,
            })
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.bmp as HGDIOBJ);
            DeleteObject(self.dc as HGDIOBJ);
        }
    }
}

// GDI handles are used only on the UI thread; the cache needs to be a static.
unsafe impl Send for Icon {}
unsafe impl Sync for Icon {}

static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<Icon>>>>> = OnceLock::new();

fn cached(key: String, bytes: &[u8], size: i32) -> Option<Arc<Icon>> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    if let Some(hit) = guard.get(&key) {
        return hit.clone();
    }
    let icon = Icon::from_png(bytes, size).map(Arc::new);
    guard.insert(key, icon.clone());
    icon
}

pub fn agent_icon(agent_id: &str, size: i32) -> Option<Arc<Icon>> {
    if agent_id.is_empty() {
        return None;
    }
    let id = agent_id.to_lowercase();
    let bytes = assets::AGENTS
        .iter()
        .find(|(uuid, _)| *uuid == id)
        .map(|(_, b)| *b)?;
    cached(format!("agent:{id}:{size}"), bytes, size)
}

pub fn rank_icon(tier: i64, size: i32) -> Option<Arc<Icon>> {
    if tier <= 0 {
        return None;
    }
    let bytes = assets::RANKS
        .iter()
        .find(|(t, _)| *t == tier)
        .map(|(_, b)| *b)?;
    cached(format!("rank:{tier}:{size}"), bytes, size)
}

/// Blits at the icon's own pixel size. `size` is accepted so callers keep
/// reading the same way, but scaling happens at load time, not here.
pub unsafe fn draw(hdc: HDC, icon: &Icon, x: i32, y: i32, _size: i32) {
    let blend = BlendFunction {
        blend_op: 0,
        // AC_SRC_ALPHA: the DIB carries premultiplied alpha.
        blend_flags: 1,
        source_constant_alpha: 255,
        alpha_format: 1,
    };
    AlphaBlend(hdc, x, y, icon.w, icon.h, icon.dc, 0, 0, icon.w, icon.h, blend);
}
