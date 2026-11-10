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
//!
//! Install/autostart plumbing lives in `install.rs`, shared with the GUI
//! installer; this module only owns the loop and the CLI wrappers.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use std::os::windows::process::CommandExt;

use anyhow::{Context, Result};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::install;

const POLL: Duration = Duration::from_secs(2);

/// Supervisor loop. Blocks forever; stop it with `valtrack.exe agent --stop`.
pub fn run() -> Result<()> {
    // One agent per session: a second autostart entry (or a manual launch) must
    // not produce two overlays fighting over the same window.
    if install::agent_alive() {
        crate::log("agent: already running, exiting");
        return Ok(());
    }

    if let Some(path) = install::pid_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, std::process::id().to_string());
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let mut child: Option<Child> = None;
    // Test hook: lets the supervisor be exercised without launching VALORANT.
    let game_exe =
        std::env::var("VALTRACK_GAME_EXE").unwrap_or_else(|_| install::GAME_EXE.to_string());
    crate::log(&format!("agent: watching for {game_exe}"));

    loop {
        let running = unsafe { install::process_running(&game_exe) };

        // Reap a child that died on its own (user pressed Alt+Q, or it crashed).
        if let Some(c) = child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                crate::log(&format!("agent: overlay exited ({status})"));
                child = None;
            }
        }

        if running && child.is_none() {
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

/// `valtrack.exe agent --stop`
pub fn stop() -> Result<()> {
    let pid = install::stop_agent()?;
    println!("agent stopped (pid {pid})");
    Ok(())
}

/// `valtrack.exe install` - the same work the GUI installer performs.
pub fn install() -> Result<()> {
    let me = std::env::current_exe().context("current_exe")?;
    let target = install::install_from(&me)?;
    println!("installed  {}", target.display());
    println!(
        "autostart  {}",
        install::autostart_value()?.unwrap_or_default()
    );
    match install::agent_pid() {
        Some(pid) => println!("agent      running (pid {pid})"),
        None => println!("agent      starting"),
    }
    println!();
    println!("VALORANT 가 실행되면 오버레이가 자동으로 뜹니다.");
    Ok(())
}

/// `valtrack.exe uninstall`
pub fn uninstall() -> Result<()> {
    let target = PathBuf::from(install::installed_exe()?);
    install::uninstall()?;
    println!("removed    {}", target.display());
    Ok(())
}

/// `valtrack.exe status`
pub fn status() -> Result<()> {
    let st = install::status()?;
    println!(
        "install path   {}",
        install::installed_exe()?.display()
    );
    println!("installed      {}", yes_no(st.installed));
    println!("login entry    {}", yes_no(st.autostart.is_some()));
    if let Some(value) = st.autostart.as_deref() {
        println!("login target   {value}");
    }
    println!("agent          {}", if st.agent_running { "running" } else { "stopped" });
    println!(
        "VALORANT       {}",
        if st.game_running { "running" } else { "not running" }
    );
    Ok(())
}

/// `valtrack.exe autostart` / `--remove`
pub fn autostart(remove: bool) -> Result<()> {
    if remove {
        if install::remove_autostart()? {
            println!("autostart removed");
        } else {
            println!("autostart was not registered");
        }
        return Ok(());
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let value = install::register_autostart(&exe)?;
    println!("autostart installed: {value}");
    Ok(())
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}
