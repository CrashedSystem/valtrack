//! Update check against the GitHub releases API.
//!
//! Deliberately *not* a self-updater. Replacing a running .exe on Windows needs
//! a helper process and a verified download; doing that badly is worse than
//! doing nothing. Instead this checks once per day and surfaces "vX.Y.Z is out",
//! and the tray menu opens the release page. `VALTRACK_NO_UPDATE=1` disables the
//! request entirely for anyone who does not want the outbound call.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

const REPO: &str = "CrashedSystem/valtrack";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| {
        PathBuf::from(base)
            .join("valtrack")
            .join("update.json")
    })
}

/// `0.1.2` / `v0.1.2` / `0.1.2-beta` -> (0, 1, 2)
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let trimmed = text.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(candidate: &str) -> bool {
    match (
        parse_version(candidate),
        parse_version(env!("CARGO_PKG_VERSION")),
    ) {
        (Some(remote), Some(local)) => remote > local,
        _ => false,
    }
}

/// Reads the last result when it is younger than a day, so double-clicking the
/// overlay five times does not produce five API calls.
fn cached() -> Option<String> {
    let path = cache_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let checked = value.get("checked").and_then(|v| v.as_u64())?;
    if now_secs().saturating_sub(checked) > CACHE_TTL_SECS {
        return None;
    }
    value
        .get("latest")
        .and_then(|v| v.as_str())
        .filter(|v| is_newer(v))
        .map(str::to_string)
}

fn store(latest: &str) {
    let Some(path) = cache_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let payload = serde_json::json!({
        "checked": now_secs(),
        "latest": latest,
        "current": env!("CARGO_PKG_VERSION"),
    });
    let _ = std::fs::write(path, payload.to_string());
}

/// One API call (cached for a day). Returns the newer version, without the `v`.
pub fn check() -> Option<String> {
    if std::env::var_os("VALTRACK_NO_UPDATE").is_some() {
        return None;
    }
    if let Some(hit) = cached() {
        return Some(hit);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(format!("valtrack/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body: serde_json::Value = client
        .get(url)
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    let tag = body.get("tag_name").and_then(|v| v.as_str())?;
    let version = tag.trim_start_matches(['v', 'V']).to_string();

    store(&version);
    is_newer(&version).then_some(version)
}

/// Background wrapper: off the UI thread, result delivered on a channel.
pub fn spawn(tx: Sender<String>) {
    let _ = std::thread::Builder::new()
        .name("valtrack-update".into())
        .spawn(move || {
            if let Some(version) = check() {
                let _ = tx.send(version);
            }
        });
}

/// Opens the releases page in the default browser.
pub fn open_releases() -> Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let url = format!("https://github.com/{REPO}/releases/latest");
    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = format!("{url}\0").encode_utf16().collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;

    // ShellExecuteW returns a value <= 32 on failure.
    if result <= 32 {
        anyhow::bail!("ShellExecuteW failed ({result})");
    }
    Ok(())
}

#[allow(dead_code)]
fn describe(version: &str) -> String {
    format!("새 버전 v{version} 사용 가능")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version("1.10.0"), Some((1, 10, 0)));
        assert_eq!(parse_version("0.2"), Some((0, 2, 0)));
        assert_eq!(parse_version("nope"), None);
    }

    #[test]
    fn newer_than_current() {
        assert!(is_newer("99.0.0"));
        assert!(!is_newer("0.0.1"));
    }
}
