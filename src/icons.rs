//! Agent / rank icons.
//!
//! The PNGs are embedded into the binary (`assets.rs`), so rendering never
//! touches the network and there is no first-paint stall.
//!
//! GDI drawing: decode PNG -> RGBA -> 32bpp top-down DIB section -> AlphaBlend.

use crate::assets;
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
    fn from_png(bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
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
                dst[i * 4] = rgba[i * 4 + 2];
                dst[i * 4 + 1] = rgba[i * 4 + 1];
                dst[i * 4 + 2] = rgba[i * 4];
                dst[i * 4 + 3] = rgba[i * 4 + 3];
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

fn cached(key: &str, bytes: &[u8]) -> Option<Arc<Icon>> {
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    if let Some(hit) = guard.get(key) {
        return hit.clone();
    }
    let icon = Icon::from_png(bytes).map(Arc::new);
    guard.insert(key.to_string(), icon.clone());
    icon
}

pub fn agent_icon(agent_id: &str) -> Option<Arc<Icon>> {
    if agent_id.is_empty() {
        return None;
    }
    let id = agent_id.to_lowercase();
    let bytes = assets::AGENTS
        .iter()
        .find(|(uuid, _)| *uuid == id)
        .map(|(_, b)| *b)?;
    cached(&format!("agent:{id}"), bytes)
}

pub fn rank_icon(tier: i64) -> Option<Arc<Icon>> {
    if tier <= 0 {
        return None;
    }
    let bytes = assets::RANKS
        .iter()
        .find(|(t, _)| *t == tier)
        .map(|(_, b)| *b)?;
    cached(&format!("rank:{tier}"), bytes)
}

pub unsafe fn draw(hdc: HDC, icon: &Icon, x: i32, y: i32, size: i32) {
    let blend = BlendFunction {
        blend_op: 0,
        blend_flags: 0,
        source_constant_alpha: 255,
        alpha_format: 1,
    };
    AlphaBlend(hdc, x, y, size, size, icon.dc, 0, 0, icon.w, icon.h, blend);
}
