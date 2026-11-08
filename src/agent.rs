//! Background supervisor: run the overlay only while VALORANT is up.
//!
//! A tray app that starts with Windows and sits in the menu would still be
//! "running" outside the game, which is exactly what this tool avoids. So the
//! agent is a tiny supervisor with no window, no tray icon and no HTTP: it
//! polls the process list every 2 s, spawns `valtrack.exe overlay` when
//! VALORANT.exe appears, kills that child when the game goes away, and keeps
//! waiting for the next session.
//!
//! It deliberately does *not* touch the Riot API. Only the child reads the
//! lockfile, so an idle agent costs one process-table snapshot per 2 s.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use std::os::windows::process::CommandExt;

use anyhow::{bail, Context, Result};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};

const GAME_EXE: &str = "VALORANT.exe";
const POLL: Duration = Duration::from_secs(2);const AUTOSTART_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_NAME: &str = "valtrack";

fn pid_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("valtrack").join("agent.pid"))
}

unsafe fn process_running(name: &str) -> bool {
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }

    let mut entry: PROCESSENTRY32W = std::mem::zeroed();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = false;

    if Process32FirstW(snapshot, &mut entry) != 0 {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if exe.eq_ignore_ascii_case(name) {
                found = true;
                break;
            }
            if Process32NextW(snapshot, &mut entry) == 0 {
                break;
            }
        }
    }

    CloseHandle(snapshot);
    found
}

/// Supervisor loop. Blocks forever; stop it with `valtrack.exe agent --stop`.
pub fn run() -> Result<()> {
    // One agent per session: a second autostart entry (or a manual launch) must
    // not produce two overlays fighting over the same window.
    let already = pid_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.trim().parse::<u32>().ok())
        .map(|pid| unsafe { process_alive(pid) })
        .unwrap_or(false);
    if already {
        crate::log("agent: already running, exiting");
        return Ok(());
    }

    if let Some(path) = pid_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, std::process::id().to_string());
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let mut child: Option<Child> = None;
    let mut seen_game = false;
    // Test hook: lets the supervisor be exercised without launching VALORANT.
    let game_exe = std::env::var("VALTRACK_GAME_EXE").unwrap_or_else(|_| GAME_EXE.to_string());
    crate::log(&format!("agent: watching for {game_exe}"));

    loop {
        let running = unsafe { process_running(&game_exe) };

        // Reap a child that died on its own (user pressed Alt+Q, or it crashed).
        if let Some(c) = child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                crate::log(&format!("agent: overlay exited ({status})"));
                child = None;
            }
        }

        if running && child.is_none() {
            if !seen_game {
                seen_game = true;
            }
            let spawned = Command::new(&exe)
                .arg("overlay")
                // hide_console() freed our console, so the inherited stdio
                // handles are invalid: passing them to a child fails with
                // ERROR_INVALID_HANDLE. Detach fully instead.
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .with_context(|| format!("spawn {}", exe.display()));
            match spawned {
                Ok(c) => {
                    crate::log(&format!("agent: game up -> overlay pid {}", c.id()));
                    child = Some(c);
                }
                Err(err) => crate::log(&format!("agent: spawn failed: {err:#}")),
            }
        } else if !running && child.is_some() {
            // The game is gone: the overlay has nothing left to draw.
            if let Some(mut c) = child.take() {
                let _ = c.kill();
                let _ = c.wait();
                crate::log("agent: game down -> overlay stopped");
            }
        }

        std::thread::sleep(POLL);
    }
}

unsafe fn process_alive(pid: u32) -> bool {
    // A handle alone proves nothing: Windows keeps terminated pids around and
    // the id can be recycled, so query the exit code instead.
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return false;
    }
    let mut code: u32 = 0;
    let ok = GetExitCodeProcess(handle, &mut code) != 0;
    CloseHandle(handle);
    // STILL_ACTIVE (259) is what a live process reports.
    ok && code == 259
}

/// `valtrack.exe agent --stop`
pub fn stop() -> Result<()> {
    let Some(path) = pid_path() else {
        bail!("LOCALAPPDATA is not set");
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        bail!("no agent pid file at {}", path.display());
    };
    let pid: u32 = text.trim().parse().context("bad pid file")?;

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            let _ = std::fs::remove_file(&path);
            bail!("agent pid {pid} is not running (stale pid file removed)");
        }
        let ok = TerminateProcess(handle, 0) != 0;
        CloseHandle(handle);
        let _ = std::fs::remove_file(&path);
        if !ok {
            bail!("TerminateProcess failed for pid {pid}");
        }
    }
    println!("agent stopped (pid {pid})");
    Ok(())
}

/// `valtrack.exe autostart` / `--remove`
pub fn autostart(remove: bool) -> Result<()> {
    if remove {
        let status = Command::new("reg")
            .args(["delete", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/f"])
            .status()
            .context("run reg delete")?;
        // A missing value is already the desired state.
        if status.success() {
            println!("autostart removed");
        } else {
            println!("autostart was not registered");
        }
        return Ok(());
    }

    let exe = std::env::current_exe().context("current_exe")?;
    let value = format!("\"{}\" agent", exe.display());
    let status = Command::new("reg")
        .args([
            "add",
            AUTOSTART_KEY,
            "/v",
            AUTOSTART_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &value,
            "/f",
        ])
        .status()
        .context("run reg add")?;
    if !status.success() {
        bail!("reg add failed ({status})");
    }
    println!("autostart installed: {value}");
    Ok(())
}
