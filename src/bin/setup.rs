//! GUI installer for valtrack.
//!
//! A separate binary on purpose: the app itself stays a single small exe with
//! no UI toolkit, and the installer is only needed once. It shares `install.rs`
//! with the CLI through `#[path]`, so both paths do exactly the same thing to
//! the machine - one of them just has buttons.
//!
//! No frameworks: a plain window with native child controls. The alternative
//! was dragging in a UI crate for three buttons and a status line.

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, LineTo, MoveToEx,
    PS_SOLID, SelectObject, SetBkMode, SetTextColor, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    FF_DONTCARE, FW_BOLD, FW_NORMAL, HBRUSH, HDC, HFONT, OUT_TT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetWindowTextW, ShowWindow, TranslateMessage,
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, MSG, SW_SHOW, WM_COMMAND, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_PAINT, WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD,
    WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

#[path = "../install.rs"]
#[allow(dead_code)]
mod install;

const ID_INSTALL: usize = 1001;
const ID_UNINSTALL: usize = 1002;
const ID_CLOSE: usize = 1003;

const WM_APP_STATUS: u32 = 0x8000 + 2;

const W: i32 = 460;
const H: i32 = 348;
const PAD: i32 = 28;

const INK: u32 = 0x0020_1B1D;
const ACCENT: u32 = 0x00E7_5C6C;

/// Busy flag: work runs on a worker thread so the window keeps painting.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Status line handle, published once the window is built.
static STATUS_HWND: Mutex<Option<isize>> = Mutex::new(None);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn set_text(hwnd: isize, s: &str) {
    let mut buf = wide(s);
    SetWindowTextW(hwnd as HWND, buf.as_mut_ptr());
}

unsafe fn build_ui(hwnd: HWND) {
    let instance = GetModuleHandleW(std::ptr::null());

    // Pretendard keeps the installer visually consistent with the app.
    let _ = pretendard();
    let face = wide("Pretendard");
    let mk = |px: i32, weight: i32| {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_TT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            5,
            FF_DONTCARE as u32,
            face.as_ptr(),
        )
    };
    let f_title = mk(24, FW_BOLD as i32);
    let f_body = mk(12, FW_NORMAL as i32);
    let f_small = mk(11, FW_NORMAL as i32);

    let label = |txt: &str, font: HFONT, top: i32, height: i32| -> HWND {
        let h = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide(txt).as_ptr(),
            WS_CHILD | WS_VISIBLE,
            PAD,
            top,
            W - PAD * 2,
            height,
            hwnd,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        SendMessageW(h, WM_SETFONT, font as WPARAM, 0);
        h
    };

    label("valtrack", f_title, 20, 32);
    label(
        "VALORANT 가 실행되면 랭크와 요원을 보여주는 오버레이를 자동으로 띄웁니다.",
        f_body,
        62,
        20,
    );
    label("설치 위치", f_small, 100, 16);
    let dir = install::install_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "LOCALAPPDATA 없음".to_string());
    label(&dir, f_small, 118, 16);

    // The one setup requirement that is not obvious and cannot be detected from
    // here: an exclusive-fullscreen game owns the display, so no overlay can be
    // composited over it.
    label(
        "VALORANT 디스플레이 모드는 '창 전체화면'이어야 오버레이가 보입니다.",
        f_small,
        146,
        18,
    );

    let status = label("확인 중…", f_small, 178, 56);
    *STATUS_HWND.lock().unwrap() = Some(status as isize);

    let by = H - 64;
    let bw = 118;
    let bh = 32;
    let mk_btn = |txt: &str, id: usize, x: i32, style: i32| {
        let b = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(txt).as_ptr(),
            (WS_CHILD | WS_VISIBLE | WS_TABSTOP) as u32 | style as u32,
            x,
            by,
            bw,
            bh,
            hwnd,
            id as *mut core::ffi::c_void,
            instance,
            std::ptr::null(),
        );
        SendMessageW(b, WM_SETFONT, f_body as WPARAM, 0);
    };
    mk_btn("설치", ID_INSTALL, PAD, BS_DEFPUSHBUTTON);
    mk_btn("제거", ID_UNINSTALL, PAD + bw + 8, BS_PUSHBUTTON);
    mk_btn("닫기", ID_CLOSE, W - PAD - bw, BS_PUSHBUTTON);
}

fn main() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("ValtrackSetup");

        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.hbrBackground = CreateSolidBrush(0x00FA_F8F9) as HBRUSH;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);

        let x = 640;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("valtrack 설치").as_ptr(),
            // A plain overlapped window: WS_POPUP with a caption confuses
            // EnumWindows on this build, which also hides it from screenshot
            // tooling.
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            x,
            240,
            W,
            H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        ShowWindow(hwnd, SW_SHOW);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Registers the app's embedded Pretendard for this process, so setup and the
/// overlay use the same face.
unsafe fn pretendard() -> bool {
    let files: [&[u8]; 2] = [
        include_bytes!("../../assets/fonts/Pretendard-Regular.otf"),
        include_bytes!("../../assets/fonts/Pretendard-SemiBold.otf"),
    ];
    let mut any = false;
    for bytes in files {
        let mut count: u32 = 0;
        let h = AddFontMemResourceEx(
            bytes.as_ptr() as *const core::ffi::c_void,
            bytes.len() as u32,
            std::ptr::null(),
            &mut count,
        );
        if !h.is_null() {
            any = true;
        }
    }
    any
}

fn set_status(msg: &str) {
    let h = *STATUS_HWND.lock().unwrap();
    if let Some(h) = h {
        unsafe { set_text(h, msg) };
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            build_ui(hwnd);
            // Reflect the machine's current state before the user clicks.
            let target = hwnd as isize;
            std::thread::spawn(move || {
                let summary = match install::status() {
                    Ok(st) => {
                        let mut parts = vec![if st.installed { "설치됨" } else { "설치 안 됨" }];
                        if st.autostart.is_some() {
                            parts.push("자동 시작 켜짐");
                        }
                        if st.agent_running {
                            parts.push("감시 중");
                        }
                        if st.game_running {
                            parts.push("VALORANT 실행 중");
                        }
                        parts.join("   ·   ")
                    }
                    Err(err) => format!("상태 확인 실패: {err:#}"),
                };
                unsafe { post_status(target, &summary) };
            });
            0
        }
        WM_COMMAND => {
            let id = wp & 0xFFFF;
            if BUSY.load(Ordering::SeqCst) {
                return 0;
            }
            match id {
                ID_INSTALL => begin(hwnd, "설치하는 중…", Action::Install),
                ID_UNINSTALL => begin(hwnd, "제거하는 중…", Action::Uninstall),
                ID_CLOSE => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_APP_STATUS => {
            // The string travels in a leaked Box; reading it here and dropping
            // it keeps the worker thread free of window handles.
            let ptr = lp as *mut String;
            if !ptr.is_null() {
                let s = *Box::from_raw(ptr);
                set_status(&s);
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as HDC;
            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, INK);
            // Return the window background so labels sit on the surface colour.
            CreateSolidBrush(0x00FA_F8F9) as LRESULT
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

enum Action {
    Install,
    Uninstall,
}

unsafe fn post_status(target: isize, msg: &str) {
    let boxed = Box::into_raw(Box::new(msg.to_string()));
    PostMessageW(target as HWND, WM_APP_STATUS, 0, boxed as LPARAM);
}

fn begin(hwnd: HWND, busy_text: &str, action: Action) {
    BUSY.store(true, Ordering::SeqCst);
    set_status(busy_text);
    let target = hwnd as isize;

    std::thread::spawn(move || {
        let msg = match action {
            Action::Install => {
                let payload = payload_path();
                match install::install_from(&payload) {
                    Ok(target) => format!("설치 완료\n{}", target.display()),
                    Err(err) => format!("설치 실패: {err:#}"),
                }
            }
            Action::Uninstall => match install::uninstall() {
                Ok(()) => "제거 완료".to_string(),
                Err(err) => format!("제거 실패: {err:#}"),
            },
        };
        BUSY.store(false, Ordering::SeqCst);
        unsafe { post_status(target, &msg) };
    });
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT = std::mem::zeroed();
    let hdc = windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);

    // Accent rule under the title: the only decoration, and the same colour the
    // overlay uses for the local player.
    let pen = CreatePen(PS_SOLID, 3, ACCENT);
    let old_pen = SelectObject(hdc, pen as _);
    MoveToEx(hdc, PAD, 56, std::ptr::null_mut());
    LineTo(hdc, PAD + 152, 56);
    SelectObject(hdc, old_pen);
    DeleteObject(pen);

    windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
}

/// The installer ships next to `valtrack.exe` and installs that copy. Falling
/// back to our own image keeps a standalone `valtrack-setup.exe` useful.
fn payload_path() -> PathBuf {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let sibling = dir.join("valtrack.exe");
            if sibling.exists() {
                return sibling;
            }
        }
        return me;
    }
    PathBuf::from("valtrack.exe")
}
