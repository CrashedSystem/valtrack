//! Overlay typeface: Pretendard, embedded and registered per-process.
//!
//! The fonts are compiled into the binary and handed to GDI with
//! `AddFontMemResourceEx`, which registers them for this process only. Nothing
//! is written to disk and the user's system font list is never touched: an
//! overlay should not leave footprints behind.

use std::sync::OnceLock;
use windows_sys::Win32::Graphics::Gdi::AddFontMemResourceEx;

/// Regular / Medium / SemiBold / Bold, in that order.
static FILES: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/Pretendard-Regular.otf"),
    include_bytes!("../assets/fonts/Pretendard-Medium.otf"),
    include_bytes!("../assets/fonts/Pretendard-SemiBold.otf"),
    include_bytes!("../assets/fonts/Pretendard-Bold.otf"),
];

static LOADED: OnceLock<bool> = OnceLock::new();

/// Family name the overlay asks GDI for.
pub const FAMILY: &str = "Pretendard";

/// Registers the embedded weights for this process. Idempotent.
///
/// Returns whether at least one weight loaded. Callers fall back to a system
/// face when it returns false, so a stripped build still renders text.
pub fn load() -> bool {
    loaded();
    *LOADED.get().unwrap_or(&false)
}

/// Whether the embedded faces registered, without retrying.
pub fn loaded() -> bool {
    *LOADED.get_or_init(|| unsafe { register() })
}

unsafe fn register() -> bool {
    let mut any = false;
    for bytes in FILES.iter() {
        let mut count: u32 = 0;
        let handle = AddFontMemResourceEx(
            bytes.as_ptr() as *const core::ffi::c_void,
            bytes.len() as u32,
            std::ptr::null(),
            &mut count,
        );
        // The handle is deliberately not released: it lives as long as the
        // process, which is exactly what the overlay needs.
        if !handle.is_null() && count > 0 {
            any = true;
        }
    }
    if !any {
        crate::log("pretendard: AddFontMemResourceEx failed; using fallback face");
    }
    any
}
