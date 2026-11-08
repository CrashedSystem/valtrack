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
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    FrameRect, GetTextExtentPoint32W, InvalidateRect, SelectObject, SetBkMode, SetTextColor,
    CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE,
    FW_BOLD, FW_NORMAL, HDC, HFONT, OUT_TT_PRECIS, PAINTSTRUCT, TRANSPARENT,
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
    LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, HMENU, LWA_COLORKEY, MF_SEPARATOR, MF_STRING, MSG,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNOACTIVATE, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DESTROY, WM_ERASEBKGND,
    WM_HOTKEY, WM_LBUTTONDBLCLK, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WM_REFRESH: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const WM_UPDATE: u32 = WM_APP + 3;

const TRAY_ID: u32 = 1;
const MENU_TOGGLE: usize = 1;
const MENU_QUIT: usize = 2;
const MENU_UPDATE: usize = 3;

const WIDTH: i32 = 540;
const HEIGHT: i32 = 636;
const ROW_H: i32 = 46;
/// Left edge of the content column (panel border + gutter).
const PAD: i32 = 26;

fn rgb(r: u32, g: u32, b: u32) -> COLORREF {
    r | (g << 8) | (b << 16)
}

const KEY: COLORREF = 0x00FF_00FF; // magenta -> punched out
const BG: COLORREF = 0x0014_1113;
const BORDER: COLORREF = 0x002D_2B33;
const DIVIDER: COLORREF = 0x0026_242B;
const WHITE: COLORREF = 0x00F4_F3F6;
const DIM: COLORREF = 0x009C_9AA3;
const MUTED: COLORREF = 0x006D_6B73;
const ACCENT: COLORREF = 0x0055_46FF;
/// Background band behind the local player's row. Subtle on purpose: it should
/// read as "this one is you", not as a selection box.
const ME_ROW: COLORREF = 0x002C_1E1A;
const ALLY: COLORREF = 0x0055_46FF;
const ENEMY: COLORREF = 0x00FF_B8_7A;

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
    74 + 24 + rows(v.allies.len()) + 6 + 16 + 24 + rows(v.enemies.len()) + 40
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
               g: i64| Row {
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
            row("빨리자라#4141", "Jett", JETT, 18, 59, 87, Some(3), 10, 23),
            row("주인님구해요#2222", "Astra", ASTRA, 15, 49, 96, Some(3), 14, 20),
            row("공주등장#바보멍청이", "Chamber", CHAMBER, 9, 5, 12, None, 3, 9),
            row("그럼에도 불구하고#사랑해야지", "Omen", OMEN, 21, 56, 41, Some(2), 31, 55),
            row("hidden", "Sage", SAGE, 14, 63, 177, Some(2), 9, 18),
        ],
        enemies: (1..=5)
            .map(|i| row("hidden", "Raze", RAZE, 16, 31, i * 20, Some(5), i * 3, 20))
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
        InvalidateRect(hwnd, std::ptr::null(), 0);
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
        let face = wide("Malgun Gothic");
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
        (
            mk(17, FW_BOLD as i32),
            mk(15, FW_BOLD as i32),
            mk(13, FW_BOLD as i32),
            mk(13, FW_BOLD as i32),
            mk(12, FW_NORMAL as i32),
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
    let brush = CreateSolidBrush(DIVIDER);
    FillRect(hdc, &rc, brush);
    DeleteObject(brush);
}

unsafe fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    FillRect(hdc, rc, brush);
    DeleteObject(brush);
}

/// Small always-on-top strip shown when the user asks for the overlay but no
/// match is running, so a double-click visibly does something.
unsafe fn paint_waiting(hwnd: HWND, hdc: HDC) {
    let _ = hwnd;
    let (f_title, _f_name, _f_team, _f_rank, f_small) = fonts();

    let panel = RECT {
        left: 8,
        top: 8,
        right: WIDTH - 8,
        bottom: 96,
    };
    fill(hdc, &panel, BG);
    let border = CreateSolidBrush(BORDER);
    FrameRect(hdc, &panel, border);
    DeleteObject(border);

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
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut client: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);

    // Clear with the colour key so the desktop shows through.
    let key_brush = CreateSolidBrush(KEY);
    FillRect(hdc, &client, key_brush);
    DeleteObject(key_brush);

    let view = state().lock().unwrap().clone();
    let Some(v) = view else {
        if WAITING.load(Ordering::Relaxed) {
            paint_waiting(hwnd, hdc);
        }
        EndPaint(hwnd, &ps);
        return;
    };

    let (f_title, f_name, f_team, f_rank, f_small) = fonts();
    let h = view_height(&v);

    // Panel
    let panel = RECT {
        left: 8,
        top: 8,
        right: WIDTH - 8,
        bottom: h - 8,
    };
    fill(hdc, &panel, BG);
    let border = CreateSolidBrush(BORDER);
    FrameRect(hdc, &panel, border);
    DeleteObject(border);

    // Header: brand, map, then queue/state on one quiet line.
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
        bottom: 38,
    };
    let map_ko = crate::l10n::map_ko(&v.map).unwrap_or(v.map.as_str());
    draw_text(hdc, f_name, map_ko, &mut rc2, WHITE, DT_RIGHT);

    let mut rc3 = RECT {
        left: PAD,
        top: 38,
        right: WIDTH - PAD,
        bottom: 58,
    };
    let sub = format!("{}  ·  {}", mode_label_of(&v.mode), state_label_of(&v.state));
    draw_text(hdc, f_small, &sub, &mut rc3, MUTED, DT_LEFT);

    // No act ID means every rank below would be last season's value. Say it.
    if crate::ACT_UNKNOWN.load(Ordering::Relaxed) {
        let mut warn = RECT {
            left: WIDTH / 2,
            top: 38,
            right: WIDTH - PAD,
            bottom: 58,
        };
        draw_text(
            hdc,
            f_small,
            "랭크 미지원 · 업데이트 대기",
            &mut warn,
            rgb(0xF5, 0xA6, 0x23),
            DT_RIGHT,
        );
    }

    divider(hdc, 64);

    let y = team(
        hdc, "내 팀", &v.allies, 74, f_team, f_name, f_rank, f_small, ALLY,
    );
    divider(hdc, y + 6);
    team(
        hdc,
        "상대 팀",
        &v.enemies,
        y + 16,
        f_team,
        f_name,
        f_rank,
        f_small,
        ENEMY,
    );

    let mut hint = RECT {
        left: PAD,
        top: h - 30,
        right: WIDTH - PAD,
        bottom: h - 12,
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
        top: h - 30,
        right: WIDTH - PAD,
        bottom: h - 12,
    };
    draw_text(hdc, f_small, "valtrack", &mut brand, MUTED, DT_RIGHT);

    EndPaint(hwnd, &ps);
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
    // Team marker
    let sq = RECT {
        left: PAD,
        top: y + 5,
        right: PAD + 8,
        bottom: y + 13,
    };
    fill(hdc, &sq, color);

    let mut rc = RECT {
        left: PAD + 14,
        top: y,
        right: WIDTH / 2,
        bottom: y + 20,
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
        top: y + 3,
        right: WIDTH - PAD,
        bottom: y + 20,
    };
    draw_text(hdc, f_small, &summary, &mut sr, MUTED, DT_RIGHT);

    let mut ry = y + 24;

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

    for row in rows {
        draw_row(hdc, row, ry, f_name, f_rank, f_small);
        ry += ROW_H;
    }
    ry
}

/// One player: party rail, agent icon, name, stats, rank block.
unsafe fn draw_row(hdc: HDC, row: &Row, y: i32, f_name: HFONT, f_rank: HFONT, f_small: HFONT) {
    let party = row.party.filter(|n| *n >= 2);
    let pcolor = party.and_then(party_color);

    // Local player: a full-width band plus an accent name. Beat any badge for
    // "which one is me" at a glance in a ten-player lobby.
    if row.is_me {
        let band = RECT {
            left: 10,
            top: y,
            right: WIDTH - 10,
            bottom: y + ROW_H,
        };
        fill(hdc, &band, ME_ROW);
    }

    // Party rail: the fastest way to spot a premade group.
    if let Some(c) = pcolor {
        let rail = RECT {
            left: 13,
            top: y + 6,
            right: 16,
            bottom: y + ROW_H - 6,
        };
        fill(hdc, &rail, c);
    }

    if let Some(icon) = crate::icons::agent_icon(&row.agent_id) {
        crate::icons::draw(hdc, &icon, PAD, y + 8, 30);
    }

    // Name column: stops well before the rank block so long KR names ellipsize.
    let mut name_rc = RECT {
        left: PAD + 38,
        top: y + 4,
        right: WIDTH - 176,
        bottom: y + 26,
    };
    let name_color = if row.is_me { ACCENT } else { WHITE };
    // `[S]` = streamer/incognito mode: the client hides this identity from
    // everyone else, so we mark it instead of pretending the name is reliable.
    let name_text = if row.incognito {
        format!("{}  [S]", row.name)
    } else {
        row.name.clone()
    };
    draw_text(hdc, f_name, &name_text, &mut name_rc, name_color, DT_LEFT);

    let mut parts = Vec::new();
    if let Some(ko) = crate::l10n::agent_ko(&row.agent_id) {
        parts.push(ko.to_string());
    } else if !row.agent.is_empty() {
        parts.push(row.agent.clone());
    }
    if row.hide_level {
        parts.push("레벨 -".to_string());
    } else {
        parts.push(format!("레벨 {}", row.level));
    }
    if row.games > 0 {
        parts.push(format!(
            "승률 {}%",
            (row.wins * 100 / row.games).clamp(0, 100)
        ));
    }
    let sub = parts.join("  ·  ");
    let mut sub_rc = RECT {
        left: PAD + 38,
        top: y + 25,
        right: WIDTH - 176,
        bottom: y + 43,
    };
    draw_text(hdc, f_small, &sub, &mut sub_rc, DIM, DT_LEFT);

    // Party size, inline after the stat line so it never touches the rank block.
    if let Some(n) = party {
        let label = party_label(Some(n));
        let sw = text_width(hdc, f_small, &sub);
        let lw = text_width(hdc, f_small, &label);
        let right_limit = WIDTH - 176 - lw;
        let px = (PAD + 38 + sw + 10).min(right_limit).max(PAD + 38);
        let mut pr = RECT {
            left: px,
            top: y + 25,
            right: px + lw + 2,
            bottom: y + 43,
        };
        draw_text(hdc, f_small, &label, &mut pr, pcolor.unwrap_or(DIM), DT_LEFT);
    }

    if let Some(icon) = crate::icons::rank_icon(row.tier) {
        crate::icons::draw(hdc, &icon, WIDTH - 168, y + 12, 22);
    }

    let tier_text = crate::rank::tier_name_ko(row.tier).to_string();
    let mut tr = RECT {
        left: WIDTH - 138,
        top: y + 6,
        right: WIDTH - PAD,
        bottom: y + 26,
    };
    draw_text(hdc, f_rank, &tier_text, &mut tr, WHITE, DT_RIGHT);

    if row.tier > 2 {
        let rr_text = format!("{} RR", row.rr);
        let mut rr_rc = RECT {
            left: WIDTH - 138,
            top: y + 24,
            right: WIDTH - PAD,
            bottom: y + 42,
        };
        draw_text(hdc, f_small, &rr_text, &mut rr_rc, DIM, DT_RIGHT);
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

            if has && VISIBLE.load(Ordering::Relaxed) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            } else if WAITING.load(Ordering::Relaxed) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                InvalidateRect(hwnd, std::ptr::null(), 0);
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
            InvalidateRect(hwnd, std::ptr::null(), 0);
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

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let x = (screen_w - WIDTH - 24).max(0);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("valtrack overlay").as_ptr(),
            WS_POPUP,
            x,
            24,
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

        SetLayeredWindowAttributes(hwnd, KEY, 0, LWA_COLORKEY);
        *hwnd_slot().lock().unwrap() = hwnd as isize;

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
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
