//! Always-on-top, click-through overlay rendered with GDI.
//!
//! A magenta colour key makes the background fully transparent, WS_EX_TRANSPARENT
//! makes the window click-through so it never steals input from the game.

use crate::rank::{
    mode_label as mode_label_of, party_label, state_label as state_label_of,
};
use crate::{connect, Lobby, Row};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    COLORREF, ERROR_ALREADY_EXISTS, GetLastError, SetLastError, HWND, LPARAM, POINT, LRESULT, RECT,
    SIZE, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW,
    FillRect, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD, FW_NORMAL, HDC, HFONT,
    OUT_TT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::Console::FreeConsole;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, VK_Q, VK_T};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, FindWindowW, GetClientRect, GetCursorPos, GetMessageW, GetSystemMetrics,
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer,
    SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage, UpdateLayeredWindow, CS_HREDRAW,
    CS_VREDRAW, HMENU, MF_SEPARATOR, MF_STRING, MSG, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, TPM_RETURNCMD, TPM_RIGHTBUTTON, ULW_ALPHA,
    WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_PAINT, WM_RBUTTONUP, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WM_REFRESH: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const WM_UPDATE: u32 = WM_APP + 3;

const TRAY_ID: u32 = 1;
const MENU_TOGGLE: usize = 1;
const MENU_QUIT: usize = 2;
const MENU_UPDATE: usize = 3;

const WIDTH: i32 = 392;
const HEIGHT: i32 = 520;
const ROW_H: i32 = 46;
/// Corner radius of the panel and of the round popup controls.
const PANEL_RADIUS: i32 = 10;
/// Window height while no match is running.
const WAITING_H: i32 = 104;
/// Agent portrait and rank crest, in real device pixels. Icons are resampled to
/// these sizes once at load, so they are drawn 1:1 instead of being scaled by
/// AlphaBlend (which drops pixels and looks stair-stepped).
const ICON_AGENT: i32 = 34;
const ICON_RANK: i32 = 22;
/// Left edge of the content column (panel border + gutter).
const PAD: i32 = 16;

fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    r | (g << 8) | (b << 16)
}

const BG: COLORREF = 0x0010_0C0D;
const HAIR: COLORREF = 0x002E_2226;
const WHITE: COLORREF = 0x00F5_F1F2;
const DIM: COLORREF = 0x0098_878B;
const MUTED: COLORREF = 0x0058_4C69;
const ACCENT: COLORREF = 0x00E7_5C6C;
const ALLY: COLORREF = 0x00E7_5C6C;
const ENEMY: COLORREF = 0x005C_A0E8;
/// Local player's row background: one step up from the panel.
const ME_ROW: COLORREF = 0x0020_1719;

/// Party size -> rail colour. Premade groups are the single most useful thing
/// on this panel, so each size gets its own hue and a bar down the row.
fn party_color(size: u8) -> Option<COLORREF> {
    match size {
        2 => Some(rgb(0x2E, 0xC4, 0xB6)),
        3 => Some(rgb(0xA7, 0x8B, 0xFA)),
        4 => Some(rgb(0xF5, 0xA6, 0x23)),
        5 => Some(rgb(0x4A, 0xDE, 0x80)),
        _ => None,
    }
}

/// Mean competitive tier, used as the "how hard is this lobby" summary.
fn avg_rank(rows: &[Row]) -> String {
    let mut sum = 0i64;
    let mut n = 0i64;
    for r in rows {
        if r.tier > 2 {
            sum += r.tier;
            n += 1;
        }
    }
    if n == 0 {
        return String::new();
    }
    let avg = ((sum as f64) / (n as f64)).round() as i64;
    crate::rank::tier_name_ko(avg.clamp(3, 27)).to_string()
}

/// Height the panel needs for this roster. Modes like Gauntlet and Deathmatch
/// put more than five players on a side, which used to overflow the fixed
/// window and clip the last rows.
fn view_height(v: &View) -> i32 {
    let rows = |n: usize| (n.max(1) as i32) * ROW_H;
    // header (58) + ally label (20) + ally rows + gap (10) + enemy label (20)
    // + enemy rows + footer (40, so the last row clears the hint line)
    58 + 20 + rows(v.allies.len()) + 10 + 20 + rows(v.enemies.len()) + 46
}

#[derive(Clone, Default)]
struct View {
    map: String,
    mode: String,
    state: String,
    allies: Vec<Row>,
    enemies: Vec<Row>,
}

static STATE: OnceLock<Mutex<Option<View>>> = OnceLock::new();
static HWND_SLOT: OnceLock<Mutex<isize>> = OnceLock::new();
static FONTS: OnceLock<(usize, usize, usize, usize, usize)> = OnceLock::new();
static VISIBLE: AtomicBool = AtomicBool::new(false);
static HAD_MATCH: AtomicBool = AtomicBool::new(false);
/// User explicitly asked to see the overlay while no match is running, so show
/// the small "waiting" panel instead of an invisible window.
static WAITING: AtomicBool = AtomicBool::new(false);
static TRAY_ADDED: AtomicBool = AtomicBool::new(false);
/// Newer release detected by the background check, if any.
static UPDATE_AVAILABLE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

struct Cache {
    match_id: String,
    roster: String,
    allies: Vec<Row>,
    enemies: Vec<Row>,
}

fn roster_key(lobby: &Lobby) -> String {
    let mut ids: Vec<&str> = lobby
        .allies
        .iter()
        .chain(lobby.enemies.iter())
        .map(|r| r.puuid.as_str())
        .collect();
    ids.sort_unstable();
    ids.join(",")
}

fn state() -> &'static Mutex<Option<View>> {
    STATE.get_or_init(|| Mutex::new(None))
}

fn demo_view() -> View {
    #[allow(clippy::too_many_arguments)]
    let row = |n: &str,
               agent: &str,
               agent_id: &str,
               tier: i64,
               rr: i64,
               lv: i64,
               party: Option<u8>,
               w: i64,
               g: i64,
               earned: i64| Row {
        puuid: String::new(),
        is_me: false,
        name: n.to_string(),
        level: lv,
        incognito: n == "hidden",
        hide_level: n == "hidden",
        rank: String::new(),
        agent: agent.to_string(),
        agent_id: agent_id.to_string(),
        tier,
        rr,
        rr_earned: earned,
        streak: if earned > 0 { 3 } else if earned < 0 { -2 } else { 0 },
        peak: if earned > 0 { Some("다이아몬드 2".to_string()) } else { None },
        derank_games: match earned {
            -12 => 0,
            -20 => 2,
            _ => 0,
        },
        at_derank_tier: earned < 0,
        penalized: false,
        party,
        wins: w,
        games: g,
    };
    const JETT: &str = "add6443a-41bd-e414-f6ad-e58d267f4e95";
    const ASTRA: &str = "41fb69c1-4189-7b37-f117-bcaf1e96f1bf";
    const CHAMBER: &str = "22697a3d-45bf-8dd7-4fec-84a9e28c69d7";
    const OMEN: &str = "8e253930-4c05-31dd-1b6c-968525494517";
    const RAZE: &str = "f94c3b30-42be-e959-889c-5aa313dba261";
    const SAGE: &str = "569fdd95-4d10-43ab-ca70-79becc718b46";
    View {
        map: "Sunset".into(),
        mode: "competitive".into(),
        state: "IN_PROGRESS".into(),
        allies: vec![
            row("빨리자라#4141", "Jett", JETT, 18, 59, 87, Some(3), 10, 23, 21),
            row("주인님구해요#2222", "Astra", ASTRA, 15, 49, 96, Some(3), 14, 20, 18),
            row("공주등장#바보멍청이", "Chamber", CHAMBER, 9, 0, 12, None, 3, 9, -12),
            row("그럼에도 불구하고#사랑해야지", "Omen", OMEN, 21, 56, 41, Some(2), 31, 55, 24),
            row("hidden", "Sage", SAGE, 14, 63, 177, Some(2), 9, 18, 19),
        ],
        enemies: (1..=5)
            .map(|i| row("hidden", "Raze", RAZE, 16, if i <= 2 { 0 } else { 31 }, i * 20, Some(5), i * 3, 20, -20))
            .collect(),
    }
}

fn hwnd_slot() -> &'static Mutex<isize> {
    HWND_SLOT.get_or_init(|| Mutex::new(0))
}

fn update_slot() -> &'static Mutex<Option<String>> {
    UPDATE_AVAILABLE.get_or_init(|| Mutex::new(None))
}

/// Tray balloon. Separate from `add_tray` because a NIM_MODIFY with NIF_INFO
/// reuses the registered icon instead of creating a second one.
unsafe fn balloon(hwnd: HWND, title: &str, text: &str) {
    if !TRAY_ADDED.load(Ordering::Relaxed) {
        return;
    }
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    for (i, c) in wide(title).iter().take(63).enumerate() {
        nid.szInfoTitle[i] = *c;
    }
    for (i, c) in wide(text).iter().take(255).enumerate() {
        nid.szInfo[i] = *c;
    }
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

/// Runs the release check off the UI thread; the window wakes on `WM_UPDATE`.
fn spawn_update_check(hwnd: HWND) {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    crate::update::spawn(tx);
    let slot = hwnd as isize;
    std::thread::spawn(move || {
        // Bounded wait: a hung check must not leave a thread around forever.
        if let Ok(version) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
            *update_slot().lock().unwrap() = Some(version);
            unsafe {
                PostMessageW(slot as HWND, WM_UPDATE, 0, 0);
            }
        }
    });
}

/// Toggle the overlay. With no match running there is nothing to draw, so flip
/// into "waiting" mode: a small panel that says the overlay is armed.
unsafe fn toggle(hwnd: HWND) {
    let has = state().lock().unwrap().is_some();
    let shown = VISIBLE.load(Ordering::Relaxed) && (has || WAITING.load(Ordering::Relaxed));
    let next = !shown;

    VISIBLE.store(next, Ordering::Relaxed);
    WAITING.store(next && !has, Ordering::Relaxed);

    if next {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Same reason as WM_REFRESH: the layered surface only exists after a
        // present(), so repaint here rather than waiting for WM_PAINT.
        paint(hwnd);
    } else {
        ShowWindow(hwnd, SW_HIDE);
    }
}

unsafe fn app_icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    // Resource id 1 comes from res/app.rc (linked by build.rs).
    let h = GetModuleHandleW(std::ptr::null());
    let icon = LoadIconW(h, 1 as *const u16);
    if icon.is_null() {
        LoadIconW(std::ptr::null_mut(), 32512 as *const u16) // IDI_APPLICATION
    } else {
        icon
    }
}

unsafe fn add_tray(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = app_icon();
    let tip = wide("valtrack - VALORANT 오버레이 (우클릭: 메뉴)");
    for (i, c) in tip.iter().take(127).enumerate() {
        nid.szTip[i] = *c;
    }
    // One-shot balloon so a double-click visibly does something even when the
    // game is idle. Dismisses itself; failures are harmless.
    nid.uFlags |= NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    let title = wide("valtrack");
    for (i, c) in title.iter().take(63).enumerate() {
        nid.szInfoTitle[i] = *c;
    }
    let info = wide("매치가 시작되면 자동으로 표시됩니다. 트레이 아이콘 우클릭 → 종료.");
    for (i, c) in info.iter().take(255).enumerate() {
        nid.szInfo[i] = *c;
    }
    TRAY_ADDED.store(Shell_NotifyIconW(NIM_ADD, &nid) != 0, Ordering::Relaxed);
}

unsafe fn remove_tray(hwnd: HWND) {
    if !TRAY_ADDED.swap(false, Ordering::Relaxed) {
        return;
    }
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn tray_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    let shown =
        VISIBLE.load(Ordering::Relaxed) && (state().lock().unwrap().is_some() || WAITING.load(Ordering::Relaxed));
    let label = if shown { "숨기기  (Alt+T)" } else { "표시  (Alt+T)" };
    AppendMenuW(menu, MF_STRING, MENU_TOGGLE, wide(label).as_ptr());
    // Only present when the background check actually found a newer build.
    let pending = update_slot().lock().unwrap().clone();
    if let Some(version) = &pending {
        AppendMenuW(
            menu,
            MF_STRING,
            MENU_UPDATE,
            wide(&format!("새 버전 v{version} 열기")).as_ptr(),
        );
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, MENU_QUIT, wide("종료  (Alt+Q)").as_ptr());

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    // Without this the menu does not dismiss when clicking elsewhere.
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        std::ptr::null(),
    );
    DestroyMenu(menu);

    match cmd as usize {
        MENU_TOGGLE => toggle(hwnd),
        MENU_UPDATE => {
            if let Err(err) = crate::update::open_releases() {
                crate::log(&format!("open releases failed: {err:#}"));
            }
        }
        MENU_QUIT => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn draw_text(hdc: HDC, font: HFONT, text: &str, rc: &mut RECT, color: COLORREF, align: u32) {
    SelectObject(hdc, font);
    SetTextColor(hdc, color);
    SetBkMode(hdc, TRANSPARENT as i32);
    let mut buf = wide(text);
    DrawTextW(
        hdc,
        buf.as_mut_ptr(),
        -1,
        rc,
        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | align,
    );
}

/// Width of `text` in the given font, so labels can be placed right after it
/// instead of at a guess. `wide` appends a NUL, hence `len - 1`.
unsafe fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    SelectObject(hdc, font);
    let buf = wide(text);
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(hdc, buf.as_ptr(), (buf.len() - 1) as i32, &mut size);
    size.cx
}

unsafe fn fonts() -> (HFONT, HFONT, HFONT, HFONT, HFONT) {
    let (title, name, team, rank, small) = *FONTS.get_or_init(|| {
        crate::fonts::load();
        let face = wide(if crate::fonts::loaded() {
            crate::fonts::FAMILY
        } else {
            "Malgun Gothic"
        });
        let mk = |h: i32, w: i32| {
            CreateFontW(
                -h,
                0,
                0,
                0,
                w,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_TT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                5,
                FF_DONTCARE as u32,
                face.as_ptr(),
            ) as usize
        };
        // CleanerText / ClearType-ish smoothing is off by default for layered
        // windows; these sizes match the v0 mockup's type ramp.
        (
            mk(15, FW_BOLD as i32),
            mk(13, FW_BOLD as i32),
            mk(11, FW_BOLD as i32),
            mk(11, 600),
            mk(10, FW_NORMAL as i32),
        )
    });
    (
        title as HFONT,
        name as HFONT,
        team as HFONT,
        rank as HFONT,
        small as HFONT,
    )
}

/// 1px horizontal rule inside the panel.
unsafe fn divider(hdc: HDC, y: i32) {
    let rc = RECT {
        left: PAD,
        top: y,
        right: WIDTH - PAD,
        bottom: y + 1,
    };
    let brush = CreateSolidBrush(HAIR);
    FillRect(hdc, &rc, brush);
    DeleteObject(brush);
}

unsafe fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc, brush);
    DeleteObject(brush);
}

/// Filled rounded rectangle. `radius` is clamped so a tiny box cannot invert.
///
/// The region is applied inside a saved DC state: selecting it as the DC's
/// default region and then deleting it leaves the DC pointing at freed memory,
/// which silently kills every later draw call.
unsafe fn fill_round(hdc: HDC, rc: &RECT, radius: i32, color: COLORREF) {
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRoundRectRgn, FillRgn, RestoreDC, SaveDC, SelectClipRgn,
    };
    let r = radius.max(0);
    let rgn = CreateRoundRectRgn(
        rc.left,
        rc.top,
        rc.right + 1,
        rc.bottom + 1,
        r * 2,
        r * 2,
    );
    if rgn.is_null() {
        fill(hdc, rc, color);
        return;
    }
    let saved = SaveDC(hdc);
    SelectClipRgn(hdc, rgn);
    let brush = CreateSolidBrush(color);
    FillRgn(hdc, rgn, brush);
    DeleteObject(brush);
    if saved != 0 {
        RestoreDC(hdc, saved);
    } else {
        SelectClipRgn(hdc, std::ptr::null_mut());
    }
    DeleteObject(rgn);
}

/// Card with a 1 px outline, drawn as two nested rounded rects.
unsafe fn card(hdc: HDC, rc: &RECT, radius: i32, body: COLORREF, edge: COLORREF) {
    fill_round(hdc, rc, radius, edge);
    let inner = RECT {
        left: rc.left + 1,
        top: rc.top + 1,
        right: rc.right - 1,
        bottom: rc.bottom - 1,
    };
    fill_round(hdc, &inner, (radius - 1).max(0), body);
}

/// Small always-on-top strip shown when the user asks for the overlay but no
/// match is running, so a double-click visibly does something.
unsafe fn paint_waiting(hwnd: HWND, hdc: HDC) -> RECT {
    let _ = hwnd;
    let (f_title, _f_name, _f_team, _f_rank, f_small) = fonts();

    let panel = RECT {
        left: 0,
        top: 0,
        right: WIDTH,
        bottom: WAITING_H,
    };
    card(hdc, &panel, PANEL_RADIUS, BG, HAIR);

    let mut rc = RECT {
        left: PAD,
        top: 14,
        right: WIDTH - PAD,
        bottom: 38,
    };
    draw_text(hdc, f_title, "VALORANT", &mut rc, ACCENT, DT_LEFT);

    let mut rc2 = RECT {
        left: PAD,
        top: 16,
        right: WIDTH - PAD,
        bottom: 36,
    };
    draw_text(hdc, f_small, "대기 중", &mut rc2, MUTED, DT_RIGHT);

    let mut rc3 = RECT {
        left: PAD,
        top: 44,
        right: WIDTH - PAD,
        bottom: 62,
    };
    draw_text(
        hdc,
        f_small,
        "게임이 시작되면 자동으로 표시됩니다",
        &mut rc3,
        WHITE,
        DT_LEFT,
    );

    let mut rc4 = RECT {
        left: PAD,
        top: 64,
        right: WIDTH - PAD,
        bottom: 82,
    };
    draw_text(
        hdc,
        f_small,
        "Alt+T 숨기기 · Alt+Q 종료 · 트레이 우클릭",
        &mut rc4,
        MUTED,
        DT_LEFT,
    );
    panel
}

/// Height of the waiting strip: header, one line, one hint line.

/// Offscreen 32-bpp surface. The overlay renders here and then hands the whole
/// bitmap to `UpdateLayeredWindow`, which is the only layered-window path where
/// GDI keeps its text anti-aliasing.
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    unsafe fn new(w: i32, h: i32) -> Option<Surface> {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
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
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(std::ptr::null_mut(), screen);
        if bmp.is_null() || dc.is_null() || bits.is_null() {
            return None;
        }
        SelectObject(dc, bmp as _);
        // GDI leaves the alpha byte untouched, so start from fully transparent.
        let all = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
        for p in all.iter_mut() {
            *p = 0;
        }
        Some(Surface {
            dc,
            bmp,
            bits: bits as *mut u32,
            w,
            h,
        })
    }

    /// Turns the rounded-rect outline into an alpha mask.
    ///
    /// GDI drawing writes no alpha, so the channel is rebuilt here: opaque
    /// inside the panel, transparent outside, one pixel of coverage on the
    /// corners. Interior RGB is premultiplied as `AC_SRC_ALPHA` requires.
    unsafe fn mask_round(&self, rc: RECT, radius: i32) {
        let r = radius as f32;
        let (l, t, rr_, b) = (
            rc.left as f32,
            rc.top as f32,
            rc.right as f32,
            rc.bottom as f32,
        );
        let buf = std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize);
        for y in 0..self.h {
            let py = y as f32 + 0.5;
            for x in 0..self.w {
                let px = x as f32 + 0.5;
                // Distance outside the rectangle, per axis.
                let ox = if px < l {
                    l - px
                } else if px > rr_ {
                    px - rr_
                } else {
                    0.0
                };
                let oy = if py < t {
                    t - py
                } else if py > b {
                    py - b
                } else {
                    0.0
                };
                let sd = if ox > 0.0 || oy > 0.0 {
                    (ox * ox + oy * oy).sqrt()
                } else {
                    // Inside: distance to the nearest corner arc.
                    let qx = if px < l + r {
                        l + r - px
                    } else if px > rr_ - r {
                        px - (rr_ - r)
                    } else {
                        0.0
                    };
                    let qy = if py < t + r {
                        t + r - py
                    } else if py > b - r {
                        py - (b - r)
                    } else {
                        0.0
                    };
                    (qx * qx + qy * qy).sqrt() - r
                };
                let cov = (0.5 - sd).clamp(0.0, 1.0);
                let a = (cov * 255.0 + 0.5) as u32;
                let i = (y * self.w + x) as usize;
                let p = buf[i];
                buf[i] = if a == 0 {
                    0
                } else if a >= 255 {
                    p | 0xFF00_0000
                } else {
                    let bl = (p & 0xFF) * a / 255;
                    let gr = ((p >> 8) & 0xFF) * a / 255;
                    let rd = ((p >> 16) & 0xFF) * a / 255;
                    bl | (gr << 8) | (rd << 16) | (a << 24)
                };
            }
        }
    }

    unsafe fn present(&self, hwnd: HWND) {
        let size = SIZE {
            cx: self.w,
            cy: self.h,
        };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            hwnd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
            self.dc,
            &src,
            0,
            &blend,
            ULW_ALPHA,
        );
    }

    unsafe fn destroy(&self) {
        DeleteObject(self.bmp as _);
        DeleteObject(self.dc as _);
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut client: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);
    let (w, h) = (client.right - client.left, client.bottom - client.top);
    if w <= 0 || h <= 0 {
        return;
    }
    let Some(surf) = Surface::new(w, h) else {
        return;
    };

    let view = state().lock().unwrap().clone();
    let panel = match view {
        Some(v) => draw_roster(surf.dc, &v, w, h),
        None => {
            if !WAITING.load(Ordering::Relaxed) {
                surf.destroy();
                return;
            }
            paint_waiting(hwnd, surf.dc)
        }
    };
    surf.mask_round(panel, PANEL_RADIUS);
    surf.present(hwnd);
    surf.destroy();
}

/// Draws the roster into `hdc` and returns the panel outline, which becomes the
/// window's alpha mask.
unsafe fn draw_roster(hdc: HDC, v: &View, w: i32, h: i32) -> RECT {
    let (f_title, f_name, f_team, f_rank, f_small) = fonts();

    // The panel now fills the window: with per-pixel alpha there is no colour
    // key to keep away from the edge, so the old 10 px inset is dead space.
    let panel = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    card(hdc, &panel, PANEL_RADIUS, BG, HAIR);

    // Header: brand, map, then queue/state on one quiet line.
    let mut rc = RECT {
        left: PAD,
        top: 10,
        right: w - PAD,
        bottom: 32,
    };
    draw_text(hdc, f_title, "VALORANT", &mut rc, ACCENT, DT_LEFT);

    let mut rc2 = RECT {
        left: PAD,
        top: 12,
        right: w - PAD,
        bottom: 32,
    };
    let map_ko = crate::l10n::map_ko(&v.map).unwrap_or(v.map.as_str());
    draw_text(hdc, f_name, map_ko, &mut rc2, WHITE, DT_RIGHT);

    let mut rc3 = RECT {
        left: PAD,
        top: 32,
        right: w - PAD,
        bottom: 50,
    };
    let sub = format!("{}  ·  {}", mode_label_of(&v.mode), state_label_of(&v.state));
    draw_text(hdc, f_small, &sub, &mut rc3, MUTED, DT_LEFT);

    // No act ID means every rank below would be last season's value. Say it.
    if crate::ACT_UNKNOWN.load(Ordering::Relaxed) {
        let mut warn = RECT {
            left: w / 2,
            top: 32,
            right: w - PAD,
            bottom: 50,
        };
        draw_text(
            hdc,
            f_small,
            "랭크 미지원",
            &mut warn,
            rgb(0xF5, 0xA6, 0x23),
            DT_RIGHT,
        );
    }

    divider(hdc, 58);

    let y = team(
        hdc, "내 팀", &v.allies, 62, f_team, f_name, f_rank, f_small, ALLY,
    );
    team(
        hdc,
        "상대 팀",
        &v.enemies,
        y + 10,
        f_team,
        f_name,
        f_rank,
        f_small,
        ENEMY,
    );

    divider(hdc, h - 30);

    let mut hint = RECT {
        left: PAD,
        top: h - 26,
        right: w - PAD,
        bottom: h - 8,
    };
    draw_text(
        hdc,
        f_small,
        "Alt+T 숨기기  ·  Alt+Q 종료",
        &mut hint,
        MUTED,
        DT_LEFT,
    );

    let mut brand = RECT {
        left: PAD,
        top: h - 26,
        right: w - PAD,
        bottom: h - 8,
    };
    draw_text(hdc, f_small, "valtrack", &mut brand, MUTED, DT_RIGHT);

    panel
}

/// Draws one team: coloured marker + name + lobby summary, then the rows.
/// Returns the y coordinate just below the last row.
#[allow(clippy::too_many_arguments)]
unsafe fn team(
    hdc: HDC,
    label: &str,
    rows: &[Row],
    y: i32,
    f_team: HFONT,
    f_name: HFONT,
    f_rank: HFONT,
    f_small: HFONT,
    color: COLORREF,
) -> i32 {
    // Team label from the v0 mockup: a small tinted square, the team name in
    // small caps, and the lobby summary pushed to the right edge. No card, no
    // background — the rows below are the content, the label is a caption.
    let dot = RECT {
        left: PAD + 2,
        top: y + 7,
        right: PAD + 7,
        bottom: y + 12,
    };
    fill(hdc, &dot, color);

    let mut rc = RECT {
        left: PAD + 12,
        top: y + 1,
        right: WIDTH / 2,
        bottom: y + 18,
    };
    draw_text(hdc, f_team, label, &mut rc, color, DT_LEFT);

    let mut summary = format!("{}명", rows.len());
    let avg = avg_rank(rows);
    if !avg.is_empty() {
        summary.push_str("  ·  평균 ");
        summary.push_str(&avg);
    }
    let mut sr = RECT {
        left: WIDTH / 2,
        top: y + 2,
        right: WIDTH - PAD,
        bottom: y + 18,
    };
    draw_text(hdc, f_small, &summary, &mut sr, MUTED, DT_RIGHT);

    let mut ry = y + 20;

    if rows.is_empty() {
        let mut er = RECT {
            left: PAD + 14,
            top: ry + 4,
            right: WIDTH - PAD,
            bottom: ry + 24,
        };
        draw_text(
            hdc,
            f_small,
            "요원 선택 중에는 표시되지 않습니다",
            &mut er,
            MUTED,
            DT_LEFT,
        );
        return ry + 32;
    }

    for (i, row) in rows.iter().enumerate() {
        draw_row(hdc, row, ry, f_name, f_rank, f_small);
        ry += ROW_H;
        // Hairline between players: makes the list read as a table instead of
        // one solid block of text.
        if i + 1 < rows.len() {
            let sep = RECT {
                left: PAD + 34,
                top: ry - 1,
                right: WIDTH - PAD,
                bottom: ry,
            };
            fill(hdc, &sep, HAIR);
        }    }
    ry
}

/// One player: agent icon, name, stats, party chip, rank icon + RR.
unsafe fn draw_row(hdc: HDC, row: &Row, y: i32, f_name: HFONT, f_rank: HFONT, f_small: HFONT) {
    let party = row.party.filter(|n| *n >= 2);
    let pcolor = party.and_then(party_color);

    // Local player: raised surface only. No stripe, no border: the mockup reads
    // better when the row is simply lifted out of the list.
    if row.is_me {
        let band = RECT {
            left: PAD - 6,
            top: y,
            right: WIDTH - PAD + 6,
            bottom: y + ROW_H,
        };
        fill_round(hdc, &band, 5, ME_ROW);
    }

    let icon_x = PAD;
    // ICON_AGENT: resampled to this exact size at load, then blitted 1:1.
    if let Some(icon) = crate::icons::agent_icon(&row.agent_id, ICON_AGENT) {
        crate::icons::draw(hdc, &icon, icon_x, y + (ROW_H - ICON_AGENT) / 2, ICON_AGENT);
    }

    // Right column is fixed-width so every RR lines up.
    let right = WIDTH - PAD;
    let rr_x = right;

    // Name: stops before the rank block so long KR names ellipsize cleanly.
    let name_left = icon_x + ICON_AGENT + 8;
    let mut name_rc = RECT {
        left: name_left,
        top: y + 2,
        right: right - 92,
        bottom: y + 19,
    };
    let name_color = if row.is_me { ACCENT } else { WHITE };
    // Streamer mode withholds the Riot ID. Nine rows of "hidden" identify
    // nobody, so the agent name stands in.
    let name_text = if row.incognito {
        match crate::l10n::agent_ko(&row.agent_id) {
            Some(agent) => format!("{agent}  [S]"),
            None if !row.agent.is_empty() => format!("{}  [S]", row.agent),
            None => "익명".to_string(),
        }
    } else {
        row.name.clone()
    };
    draw_text(hdc, f_name, &name_text, &mut name_rc, name_color, DT_LEFT);

    // Level, right after the name's slot but before the rank block: a small
    // number most players glance at to judge account age.
    if !row.hide_level && row.level > 0 {
        let level_text = format!("Lv {}", row.level);
        let level_w = text_width(hdc, f_small, &level_text);
        let mut lv_rc = RECT {
            left: right - 92 - level_w - 4,
            top: y + 3,
            right: right - 92 - 4,
            bottom: y + 18,
        };
        draw_text(hdc, f_small, &level_text, &mut lv_rc, MUTED, DT_RIGHT);
    }

    // Second line: agent · win record · party. Every fact we hold, packed.
    let mut parts = Vec::new();
    if let Some(ko) = crate::l10n::agent_ko(&row.agent_id) {
        parts.push(ko.to_string());
    } else if !row.agent.is_empty() {
        parts.push(row.agent.clone());
    }
    if row.games > 0 {
        // games is the total, so lead with it: "23전 10승 43%" reads as
        // total / wins / rate, and stays short enough for a 392 px row.
        parts.push(format!(
            "{}전 {}승 {}%",
            row.games,
            row.wins,
            (row.wins * 100 / row.games).clamp(0, 100)
        ));
    } else {
        parts.push("경쟁 기록 없음".to_string());
    }
    let sub = parts.join("  ·  ");
    let mut sub_rc = RECT {
        left: name_left,
        top: y + 18,
        right: right - 92,
        bottom: y + 34,
    };
    draw_text(hdc, f_small, &sub, &mut sub_rc, MUTED, DT_LEFT);

    // Party chip, coloured per size: the fastest way to spot a premade.
    if let Some(n) = party {
        let label = party_label(Some(n));
        let sw = text_width(hdc, f_small, &sub);
        let lw = text_width(hdc, f_small, &label);
        let limit = right - 92 - lw;
        let px = (name_left + sw + 7).min(limit).max(name_left);
        let mut pr = RECT {
            left: px,
            top: y + 18,
            right: px + lw + 2,
            bottom: y + 34,
        };
        draw_text(hdc, f_small, &label, &mut pr, pcolor.unwrap_or(MUTED), DT_LEFT);
    }

    // Third line: streak, trend, then the two flags worth acting on.
    let mut third = Vec::new();
    if row.streak != 0 {
        let n = row.streak.unsigned_abs();
        third.push(if row.streak > 0 {
            format!("{n}연승 중")
        } else {
            format!("{n}연패 중")
        });
    }
    if row.tier > 2 && row.rr_earned != 0 {
        third.push(format!(
            "최근 {}{} RR",
            if row.rr_earned > 0 { "+" } else { "" },
            row.rr_earned
        ));
    }
    // Demotion risk and penalties outrank the peak label: both change how you
    // read the player, and the peak is trivia once you know either.
    // A player is only really in a demotion match at the bottom division of a
    // tier with 0 RR; anywhere above 0 they still have points to lose first.
    let derank_match = row.at_derank_tier && row.rr == 0 && row.tier > 2;
    let risky = derank_match || row.penalized;
    if derank_match {
        third.push(if row.derank_games > 0 {
            format!("강등전 (보호 {}개 남음)", row.derank_games)
        } else {
            "강등전".to_string()
        });
    }
    if row.penalized {
        third.push("제재 이력".to_string());
    }
    if !risky {
        if let Some(peak) = &row.peak {
            let current = crate::rank::tier_name_ko(row.tier);
            if !peak.is_empty() && !current.is_empty() && peak != current {
                third.push(format!("최고 {peak}"));
            }
        }
    }
    if !third.is_empty() {
        let color = if risky {
            rgb(0xD9, 0x5B, 0x66)
        } else if row.streak > 0 || (row.streak == 0 && row.rr_earned > 0) {
            rgb(0x6E, 0xC2, 0x8A)
        } else if row.streak < 0 || row.rr_earned < 0 {
            rgb(0xD9, 0x5B, 0x66)
        } else {
            MUTED
        };
        let text = third.join("  ·  ");
        let mut tr = RECT {
            left: name_left,
            top: y + 32,
            right: right - 92,
            bottom: y + ROW_H - 1,
        };
        draw_text(hdc, f_small, &text, &mut tr, color, DT_LEFT);
    }

    // Rank: icon + RR only. The tier name is redundant once the icon is legible
    // and it was the element that overflowed at this width.
    if let Some(icon) = crate::icons::rank_icon(row.tier, ICON_RANK) {
        crate::icons::draw(
            hdc,
            &icon,
            right - ICON_RANK - 34,
            y + (ROW_H - ICON_RANK) / 2,
            ICON_RANK,
        );
    }
    if row.tier > 2 {
        let rr_text = format!("{}", row.rr);
        let mut rr_rc = RECT {
            left: rr_x - 32,
            top: y + (ROW_H - 16) / 2,
            right: rr_x,
            bottom: y + (ROW_H + 16) / 2,
        };
        draw_text(hdc, f_rank, &rr_text, &mut rr_rc, DIM, DT_RIGHT);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_REFRESH => {
            let has = state().lock().unwrap().is_some();
            let had = HAD_MATCH.swap(has, Ordering::Relaxed);

            if has && !had {
                // A new match started: show once, then the user owns visibility.
                VISIBLE.store(true, Ordering::Relaxed);
                WAITING.store(false, Ordering::Relaxed);
            }
            if !has {
                // Keep the "armed" panel up only if the user asked for it.
                VISIBLE.store(WAITING.load(Ordering::Relaxed), Ordering::Relaxed);
            }

            // Roster size varies by mode (Gauntlet, Deathmatch). Size the window
            // to the content so the last rows are never clipped.
            if has {
                let h = state()
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(view_height)
                    .unwrap_or(HEIGHT);
                let h = h.min(GetSystemMetrics(SM_CYSCREEN) - 48).max(120);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    WIDTH,
                    h,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }

            // With UpdateLayeredWindow the window has no painted surface until
            // the first present() call, so ShowWindow alone leaves an invisible
            // window: paint directly instead of waiting for WM_PAINT.
            if has && VISIBLE.load(Ordering::Relaxed) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                paint(hwnd);
            } else if WAITING.load(Ordering::Relaxed) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                paint(hwnd);
            } else {
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_HOTKEY => {
            // 1 = Alt+T (toggle), 2 = Alt+Q (quit).
            if wp as i32 == 2 {
                DestroyWindow(hwnd);
            } else {
                toggle(hwnd);
            }
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_RBUTTONUP => tray_menu(hwnd),
                WM_LBUTTONDBLCLK => toggle(hwnd),
                _ => {}
            }
            0
        }
        WM_UPDATE => {
            if let Some(version) = update_slot().lock().unwrap().clone() {
                crate::log(&format!("update available: v{version}"));
                balloon(
                    hwnd,
                    "valtrack 새 버전",
                    &format!("v{version} 사용 가능 · 트레이 우클릭 → 열기"),
                );
            }
            0
        }
        WM_TIMER => {
            // Safety-net repaint. A layered window keeps its last present()
            // image, but this also catches the case where the first present
            // happened before the window was on screen.
            if VISIBLE.load(Ordering::Relaxed) || WAITING.load(Ordering::Relaxed) {
                paint(hwnd);
            }
            0
        }
        WM_DESTROY => {
            remove_tray(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Detach from the console so nothing is left on screen when started from the
/// desktop icon. `ShowWindow(SW_HIDE)` is not enough when Windows Terminal is
/// the default terminal host, and can stall on a pseudo console — `FreeConsole`
/// is the supported way to drop it.
pub fn hide_console() {
    if std::env::var_os("VALTRACK_KEEP_CONSOLE").is_some() {
        return;
    }
    unsafe {
        FreeConsole();
    }
}

pub fn run(demo: bool) -> anyhow::Result<()> {
    crate::log(&format!("overlay start demo={demo}"));
    unsafe {
        // One overlay per session: a second double-click brings the running one
        // up instead of stacking another always-on-top window.
        let mutex_name = wide("Local\\valtrack-overlay");
        SetLastError(0);
        let _guard = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            crate::log("mutex exists -> toggling existing window and exiting");
            let class_name = wide("ValtrackOverlay");
            let existing = FindWindowW(class_name.as_ptr(), std::ptr::null());
            if !existing.is_null() {
                PostMessageW(existing, WM_HOTKEY, 1, 0);
            }
            return Ok(());
        }

        // Launched from the desktop shortcut: hide_console() already ran in
        // main(); this is a no-op second guard for direct calls.
        hide_console();

        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("ValtrackOverlay");

        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinstance;
        wc.hbrBackground = std::ptr::null_mut();
        wc.lpszClassName = class_name.as_ptr();
        RegisterClassW(&wc);

        let _screen_w = GetSystemMetrics(SM_CXSCREEN);
        // Left edge, under the minimap: the kill feed owns the top-right corner.
        let x = 20;
        let y = 190;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("valtrack overlay").as_ptr(),
            WS_POPUP,
            x,
            y,
            WIDTH,
            HEIGHT,
            std::ptr::null_mut(),
            std::ptr::null_mut() as HMENU,
            hinstance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            anyhow::bail!("CreateWindowExW failed");
        }

        // No colour key any more: the window is composited from a premultiplied
        // RGBA bitmap with UpdateLayeredWindow. LWA_COLORKEY would force GDI to
        // drop text anti-aliasing, which is what made the type look stepped.

        // WS_POPUP ignored the create-time x/y on this build, so place it once.
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            WIDTH,
            HEIGHT,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        *hwnd_slot().lock().unwrap() = hwnd as isize;
        crate::log(&format!("overlay placed at ({x},{y}) {WIDTH}x{HEIGHT}"));

        // Alt+T toggles the overlay; Alt+Q quits. The game keeps its own keys.
        RegisterHotKey(hwnd, 1, MOD_ALT, VK_T as u32);
        RegisterHotKey(hwnd, 2, MOD_ALT, VK_Q as u32);

        add_tray(hwnd);
        crate::log("window created, tray added");
        spawn_update_check(hwnd);

        // 30s is only a safety net; the worker posts a repaint on every poll.
        SetTimer(hwnd, 1, 30_000, None);

        // Polling worker: HTTP must never run on the UI thread.
        if demo {
            *state().lock().unwrap() = Some(demo_view());
        } else {
            // Presence events wake the worker immediately. The timer below is only
            // a fallback (and the only trigger if the WebSocket cannot connect).
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            crate::ws::spawn(tx);

            std::thread::spawn(move || {
                // Expensive lookups (names/ranks/parties) are cached per roster so a
                // poll does not hammer Riot with ~26 requests every tick.
                let mut cache: Option<Cache> = None;
                loop {
                    let view = match connect() {
                        Ok(riot) => match crate::fetch_lobby(&riot) {
                            Ok(Some(mut lobby)) => {
                                let key = roster_key(&lobby);
                                match &cache {
                                    Some(c) if c.match_id == lobby.match_id && c.roster == key => {
                                        lobby.allies = c.allies.clone();
                                        lobby.enemies = c.enemies.clone();
                                    }
                                    _ => {
                                        crate::enrich_lobby(&riot, &mut lobby);
                                        cache = Some(Cache {
                                            match_id: lobby.match_id.clone(),
                                            roster: key,
                                            allies: lobby.allies.clone(),
                                            enemies: lobby.enemies.clone(),
                                        });
                                    }
                                }
                                Some(View {
                                    map: lobby.map,
                                    mode: lobby.mode,
                                    state: lobby.state,
                                    allies: lobby.allies,
                                    enemies: lobby.enemies,
                                })
                            }
                            _ => None,
                        },
                        Err(_) => None,
                    };

                    let has_view = view.is_some();
                    *state().lock().unwrap() = view;

                    let slot = *hwnd_slot().lock().unwrap();
                    if slot != 0 {
                        PostMessageW(slot as HWND, WM_REFRESH, 0, 0);
                    }

                    // In a match 5s keeps agent picks and score changes fresh; idle
                    // 10s halves the token churn because presence events normally
                    // wake us the moment a queue pops.
                    let wait = if has_view {
                        Duration::from_secs(5)
                    } else {
                        Duration::from_secs(10)
                    };
                    // Wake on the next presence event, or after `wait` if the
                    // WebSocket is down / quiet. Drain duplicates queued meanwhile.
                    let _ = rx.recv_timeout(wait);
                    while rx.try_recv().is_ok() {}
                }
            });
        }

        if demo {
            VISIBLE.store(true, Ordering::Relaxed);
            HAD_MATCH.store(true, Ordering::Relaxed);
            // Demo skips the polling worker, so size the window here too.
            let h = state()
                .lock()
                .unwrap()
                .as_ref()
                .map(view_height)
                .unwrap_or(HEIGHT)
                .min(GetSystemMetrics(SM_CYSCREEN) - 48);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                WIDTH,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            paint(hwnd);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
