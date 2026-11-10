//! Install location, login-autostart registration and process supervision.
//!
//! Shared by the CLI (`valtrack.exe`) and the GUI installer
//! (`valtrack-setup.exe`) through `#[path]`, so both agree on exactly one
//! install directory, one registry value and one set of rules. Nothing here
//! prints to stdout: callers decide how to report.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, CREATE_NO_WINDOW,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

pub const GAME_EXE: &str = "VALORANT.exe";
const APP_DIR: &str = "valtrack";
const AUTOSTART_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_NAME: &str = "valtrack";

/// Is a process with this image name in the process table?
pub unsafe fn process_running(name: &str) -> bool {
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

pub unsafe fn process_alive(pid: u32) -> bool {
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

/// `%LOCALAPPDATA%\valtrack` - the app lives in the user's own profile so
/// installing never needs an administrator prompt.
pub fn install_dir() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join(APP_DIR))
        .context("LOCALAPPDATA is not set")
}

pub fn installed_exe() -> Result<PathBuf> {
    Ok(install_dir()?.join("valtrack.exe"))
}

/// True when this process already runs from the installed location.
pub fn running_installed() -> bool {
    let (Ok(me), Ok(installed)) = (std::env::current_exe(), installed_exe()) else {
        return false;
    };
    match (me.canonicalize(), installed.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => me == installed,
    }
}

pub fn pid_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join(APP_DIR).join("agent.pid"))
}

pub fn agent_pid() -> Option<u32> {
    pid_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.trim().parse::<u32>().ok())
}

pub fn agent_alive() -> bool {
    agent_pid()
        .map(|pid| unsafe { process_alive(pid) })
        .unwrap_or(false)
}

/// Starts the supervisor from the installed copy. Detached: no stdio, no
/// console, so the GUI installer can exit while the agent keeps running.
pub fn start_agent() -> Result<u32> {
    let exe = installed_exe()?;
    if !exe.exists() {
        bail!("{} does not exist", exe.display());
    }
    let child = Command::new(&exe)
        .arg("agent")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("start {}", exe.display()))?;
    Ok(child.id())
}

pub fn stop_agent() -> Result<u32> {
    let Some(path) = pid_path() else {
        bail!("LOCALAPPDATA is not set");
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        bail!("no agent pid file");
    };
    let pid: u32 = text.trim().parse().context("bad pid file")?;

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            let _ = std::fs::remove_file(&path);
            bail!("agent pid {pid} is not running");
        }
        let ok = TerminateProcess(handle, 0) != 0;
        CloseHandle(handle);
        let _ = std::fs::remove_file(&path);
        if !ok {
            bail!("TerminateProcess failed for pid {pid}");
        }
    }
    Ok(pid)
}

/// Copies `payload` into the install directory, points the login entry at it
/// and starts the supervisor if it is not already up.
///
/// The copy is written under a temporary name and then renamed into place: a
/// running `valtrack.exe` cannot be overwritten, but it can be replaced by
/// rename, which is what makes reinstalling while the agent runs work.
pub fn install_from(payload: &Path) -> Result<PathBuf> {
    if !payload.exists() {
        bail!("{} does not exist", payload.display());
    }
    let dir = install_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let target = installed_exe()?;

    let same = payload
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .map(|(a, b)| a == b)
        .unwrap_or(false);
    if !same {
        let staging = dir.join("valtrack.new.exe");
        std::fs::copy(payload, &staging)
            .with_context(|| format!("copy {}", payload.display()))?;
        std::fs::rename(&staging, &target)
            .with_context(|| format!("move into {}", target.display()))?;
    }

    register_autostart(&target)?;
    if !agent_alive() {
        start_agent()?;
    }
    Ok(target)
}

pub fn uninstall() -> Result<()> {
    let _ = stop_agent();
    remove_autostart()?;

    let target = installed_exe()?;
    if running_installed() {
        bail!("{} is running; quit it first", target.display());
    }
    match std::fs::remove_file(&target) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("remove {}", target.display())),
    }
    // The agent may have recreated its pid file after we read it.
    if let Some(path) = pid_path() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

pub struct Status {
    pub installed: bool,
    pub autostart: Option<String>,
    pub agent_running: bool,
    pub game_running: bool,
}

pub fn status() -> Result<Status> {
    Ok(Status {
        installed: installed_exe()?.exists(),
        autostart: autostart_value()?,
        agent_running: agent_alive(),
        game_running: unsafe { process_running(GAME_EXE) },
    })
}

pub fn autostart_value() -> Result<Option<String>> {
    let out = Command::new("reg")
        .args(["query", AUTOSTART_KEY, "/v", AUTOSTART_NAME])
        .output()
        .context("run reg query")?;
    if !out.status.success() {
        return Ok(None);
    }
    // The value is the field after "REG_SZ" on the named line.
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .find(|line| line.trim_start().starts_with(AUTOSTART_NAME))
        .and_then(|line| line.split("REG_SZ").nth(1))
        .map(|v| v.trim().to_string()))
}

pub fn register_autostart(exe: &Path) -> Result<String> {
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
    Ok(value)
}

pub fn remove_autostart() -> Result<bool> {
    let status = Command::new("reg")
        .args(["delete", AUTOSTART_KEY, "/v", AUTOSTART_NAME, "/f"])
        .status()
        .context("run reg delete")?;
    Ok(status.success())
}
