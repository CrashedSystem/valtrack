//! Filesystem discovery + parsing of the two artifacts Tracker.gg relies on:
//! the Riot Client `lockfile` and VALORANT's `ShooterGame.log`.

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use std::path::PathBuf;

pub struct Lockfile {
    pub port: u16,
    pub password: String,
}

pub struct GameLog {
    pub shard: String,
    pub region: String,
    pub client_version: String,
}

fn local_app_data() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("LOCALAPPDATA is not set; this tool only runs on Windows"))
}

pub fn lockfile_path() -> Result<PathBuf> {
    Ok(local_app_data()?
        .join("Riot Games")
        .join("Riot Client")
        .join("Config")
        .join("lockfile"))
}

pub fn game_log_path() -> Result<PathBuf> {
    Ok(local_app_data()?
        .join("VALORANT")
        .join("Saved")
        .join("Logs")
        .join("ShooterGame.log"))
}

/// Format: `name:pid:port:password:protocol`
pub fn read_lockfile() -> Result<Lockfile> {
    let path = lockfile_path()?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let parts: Vec<&str> = raw.trim().split(':').collect();
    if parts.len() < 4 {
        bail!("unexpected lockfile contents ({} fields)", parts.len());
    }
    let port: u16 = parts[2]
        .parse()
        .with_context(|| format!("bad port in lockfile: {:?}", parts[2]))?;
    Ok(Lockfile {
        port,
        password: parts[3].to_string(),
    })
}

/// Same rules the Overwolf app uses:
///   version : `/CI server version: ([a-zA-Z0-9.-]+)/`
///   shard/region: `/glz-(\w+)-\d+\.(\w+)\.a\.pvp\.net/`
pub fn read_game_log() -> Result<GameLog> {
    let path = game_log_path()?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;

    let version_re = Regex::new(r"CI server version: ([a-zA-Z0-9.\-]+)").unwrap();
    let endpoint_re = Regex::new(r"glz-(\w+)-\d+\.(\w+)\.a\.pvp\.net").unwrap();

    let mut version = version_re
        .captures(&text)
        .map(|c| c[1].to_string())
        .ok_or_else(|| anyhow!("ShooterGame.log has no 'CI server version' line"))?;

    if version.contains("release") && !version.contains("shipping") {
        let fix = Regex::new(r"(release-\d+\.\d+)(\S*)").unwrap();
        if let Some(c) = fix.captures(&version) {
            version = format!("{}-shipping{}", &c[1], &c[2]);
        }
    }

    let caps = endpoint_re
        .captures(&text)
        .ok_or_else(|| anyhow!("ShooterGame.log has no glz-<shard>-1.<region>.a.pvp.net URL"))?;

    Ok(GameLog {
        shard: caps[1].to_string(),
        region: caps[2].to_string(),
        client_version: version,
    })
}
