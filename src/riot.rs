//! Thin wrapper over the same Riot endpoints the Overwolf app calls.

use crate::auth::TokenResponse;
use crate::model::*;
use anyhow::{Context, Result};
use base64::Engine;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const PLATFORM: &str = r#"{"platformType":"PC","platformOS":"Windows","platformOSVersion":"10.0.19044.1.256.64bit","platformChipset":"Unknown"}"#;

pub struct Riot {
    http: reqwest::blocking::Client,
    local: reqwest::blocking::Client,
    pub region: String,
    pub shard: String,
    pub client_version: String,
    pub puuid: String,
    access_token: String,
    entitlement: String,
    local_port: u16,
    local_auth: String,
}

impl Riot {
    pub fn new(
        tokens: &TokenResponse,
        region: String,
        shard: String,
        client_version: String,
        local_port: u16,
        local_password: &str,
    ) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("failed to build HTTPS client")?;
        let local = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(8))
            .build()
            .context("failed to build local client")?;
        Ok(Self {
            http,
            local,
            region,
            shard,
            client_version,
            puuid: tokens.subject.clone(),
            access_token: tokens.access_token.clone(),
            entitlement: tokens.entitlement.clone(),
            local_port,
            local_auth: base64::engine::general_purpose::STANDARD
                .encode(format!("riot:{}", local_password)),
        })
    }

    fn glz(&self) -> String {
        format!("https://glz-{}-1.{}.a.pvp.net", self.shard, self.region)
    }

    fn pd(&self) -> String {
        format!("https://pd.{}.a.pvp.net", self.region)
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::blocking::RequestBuilder {
        let platform = base64::engine::general_purpose::STANDARD.encode(PLATFORM);
        self.http
            .request(method, url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            .header("X-Riot-Entitlements-JWT", &self.entitlement)
            .header("X-Riot-ClientPlatform", platform)
            .header("X-Riot-ClientVersion", &self.client_version)
    }

    /// Current match id, if any. Checks the in-game and the agent-select state.
    pub fn active_match_id(&self) -> Result<Option<String>> {
        let url = format!("{}/core-game/v1/players/{}", self.glz(), self.puuid);
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .context("core-game player lookup failed")?;
        let status = resp.status();
        if status.is_success() {
            if let Ok(r) = resp.json::<PlayerMatchRef>() {
                if let Some(id) = r.match_id {
                    return Ok(Some(id));
                }
            }
        } else if status != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("core-game lookup returned {}: {}", status, body.trim());
        }

        let url = format!("{}/pregame/v1/players/{}", self.glz(), self.puuid);
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .context("pregame player lookup failed")?;
        let status = resp.status();
        if status.is_success() {
            if let Ok(r) = resp.json::<PlayerMatchRef>() {
                if let Some(id) = r.match_id {
                    return Ok(Some(id));
                }
            }
        } else if status != reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("pregame lookup returned {}: {}", status, body.trim());
        }
        Ok(None)
    }

    pub fn core_match(&self, match_id: &str) -> Result<CoreGameMatch> {
        let url = format!("{}/core-game/v1/matches/{}", self.glz(), match_id);
        let resp = self.request(reqwest::Method::GET, &url).send()?;
        ensure_ok(resp, "core-game match")?.json().context("bad core-game JSON")
    }

    pub fn pregame_match(&self, match_id: &str) -> Result<PregameMatch> {
        let url = format!("{}/pregame/v1/matches/{}", self.glz(), match_id);
        let resp = self.request(reqwest::Method::GET, &url).send()?;
        ensure_ok(resp, "pregame match")?
            .json()
            .context("bad pregame JSON")
    }

    /// Resolves Riot IDs for up to a full lobby.
    pub fn names(&self, puuids: &[String]) -> Result<HashMap<String, NameEntry>> {
        let url = format!("{}/name-service/v2/players", self.pd());
        let resp = self
            .request(reqwest::Method::PUT, &url)
            .json(puuids)
            .send()?;
        let list: Vec<NameEntry> = ensure_ok(resp, "name-service")?
            .json()
            .context("bad name-service JSON")?;
        Ok(list
            .into_iter()
            .map(|n| (n.subject.clone(), n))
            .collect())
    }

    pub fn mmr(&self, puuid: &str) -> Result<Mmr> {
        let url = format!("{}/mmr/v1/players/{}", self.pd(), puuid);
        let resp = self.request(reqwest::Method::GET, &url).send()?;
        ensure_ok(resp, "mmr")?.json().context("bad mmr JSON")
    }

    /// Active act ID. Rank and win rate are per-act, and the client keys
    /// `SeasonalInfoBySeasonID` by this ID.
    ///
    /// Three stages, because a wrong act silently shows last season's ranks:
    ///   1. in-memory cache, refreshed every [`ACT_TTL_SECS`]
    ///   2. live content-service query (the client's own source of truth)
    ///   3. the last ID that worked, from disk, so a transient 429/500 or a
    ///      changed endpoint does not blank every rank
    pub fn current_act_id(&self) -> Option<String> {
        static CACHE: OnceLock<Mutex<Option<(String, u64)>>> = OnceLock::new();
        let cell = CACHE.get_or_init(|| Mutex::new(None));

        {
            let guard = cell.lock().unwrap();
            if let Some((id, at)) = guard.as_ref() {
                if now_secs().saturating_sub(*at) < ACT_TTL_SECS {
                    return Some(id.clone());
                }
            }
        }

        match self.fetch_act_id() {
            Some(id) => {
                *cell.lock().unwrap() = Some((id.clone(), now_secs()));
                write_act_cache(&id);
                Some(id)
            }
            None => match read_act_cache() {
                Some(id) => {
                    crate::log(&format!("actId: live lookup failed, using cached {id}"));
                    Some(id)
                }
                None => {
                    crate::log("actId: live lookup failed and no cache; unsupported client");
                    None
                }
            },
        }
    }

    /// One uncached content-service query. `Type` must be checked first: the
    /// payload also marks the running episode as active.
    fn fetch_act_id(&self) -> Option<String> {
        let url = format!(
            "https://shared.{}.a.pvp.net/content-service/v3/content",
            self.region
        );
        let resp = self.request(reqwest::Method::GET, &url).send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let v: serde_json::Value = resp.json().ok()?;
        v.get("Seasons")?
            .as_array()?
            .iter()
            .find(|s| {
                s.get("Type").and_then(|t| t.as_str()) == Some("act")
                    && s.get("IsActive").and_then(|a| a.as_bool()) == Some(true)
            })?
            .get("ID")?
            .as_str()
            .map(str::to_string)
    }

    /// Raw JSON for a known endpoint. Used by `valtrack diag` to check which
    /// fields the live payloads actually carry before we model them.
    pub fn debug_json(&self, url: &str) -> Option<serde_json::Value> {
        self.get_json(url)
    }

    pub fn debug_match_details(&self, match_id: &str) -> Option<serde_json::Value> {
        let url = format!("{}/match-details/v1/matches/{}", self.pd(), match_id);
        self.get_json(&url)
    }

    pub fn debug_urls(&self) -> Vec<(&'static str, String)> {
        vec![
            ("core-game", format!("{}/core-game/v1/players/{}", self.glz(), self.puuid)),
            ("mmr", format!("{}/mmr/v1/players/{}", self.pd(), self.puuid)),
        ]
    }

    /// GET with one retry on 429. Used by the enrichment paths where a transient
    /// rate limit used to silently produce "Unrated" / missing party badges.
    fn get_json(&self, url: &str) -> Option<serde_json::Value> {
        for attempt in 0..2 {
            let resp = self.request(reqwest::Method::GET, url).send().ok()?;
            match resp.status().as_u16() {
                429 => {
                    std::thread::sleep(Duration::from_millis(1200));
                    let _ = attempt;
                    continue;
                }
                s if (200..300).contains(&s) => return resp.json().ok(),
                _ => return None,
            }
        }
        None
    }

    /// Party IDs per player. Works for the running match too, but can lag or
    /// 404 right after agent select, so callers must treat it as optional.
    pub fn match_details(&self, match_id: &str) -> Result<MatchDetails> {
        let url = format!("{}/match-details/v1/matches/{}", self.pd(), match_id);
        let Some(v) = self.get_json(&url) else {
            anyhow::bail!("match-details unavailable for {}", match_id);
        };
        serde_json::from_value(v).context("bad match-details JSON")
    }

    /// Members of *your* party. Riot blocks looking up anyone else's party
    /// (403 NONSELF_OPERATION), so this is the only exact live source.
    pub fn my_party(&self) -> Result<Vec<String>> {
        let url = format!("{}/parties/v1/players/{}", self.glz(), self.puuid);
        let resp = self.request(reqwest::Method::GET, &url).send()?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let v: serde_json::Value = resp.json().unwrap_or_default();
        let id = v
            .get("CurrentPartyID")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/parties/v1/parties/{}", self.glz(), id);
        let resp = self.request(reqwest::Method::GET, &url).send()?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let d: serde_json::Value = resp.json().unwrap_or_default();
        Ok(d.get("Members")
            .and_then(|m| m.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("Subject").and_then(|s| s.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Party size for lobby players that happen to be on your friends list.
    /// The local chat service only exposes friends, never the whole lobby.
    pub fn friend_party_sizes(&self) -> Result<HashMap<String, u8>> {
        let url = format!("https://127.0.0.1:{}/chat/v4/presences", self.local_port);
        let resp = self
            .local
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Basic {}", self.local_auth),
            )
            .send()?;
        if !resp.status().is_success() {
            return Ok(HashMap::new());
        }
        let v: serde_json::Value = resp.json().unwrap_or_default();
        let mut out = HashMap::new();
        if let Some(list) = v.get("presences").and_then(|p| p.as_array()) {
            for p in list {
                let Some(puuid) = p.get("puuid").and_then(|x| x.as_str()) else {
                    continue;
                };
                let Some(packed) = p.get("packedData").and_then(|x| x.as_str()) else {
                    continue;
                };
                let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(packed) else {
                    continue;
                };
                let Ok(inner) = serde_json::from_slice::<serde_json::Value>(&raw) else {
                    continue;
                };
                if let Some(size) = inner
                    .get("party")
                    .and_then(|party| party.get("currentSize"))
                    .and_then(|s| s.as_u64())
                {
                    out.insert(puuid.to_string(), size as u8);
                }
            }
        }
        Ok(out)
    }

    /// Recent match IDs for one player (newest first).
    pub fn match_history(&self, puuid: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/match-history/v1/history/{}?startIndex=0&endIndex=20",
            self.pd(),
            puuid
        );
        let Some(v) = self.get_json(&url) else {
            return Ok(Vec::new());
        };
        Ok(v.get("History")
            .and_then(|h| h.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        e.get("MatchID")
                            .and_then(|m| m.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Recent results for one player, newest first: `true` = win.
    ///
    /// Walks the match history newest-first and stops at the first result that
    /// disagrees with the previous one, so the caller gets the length of the
    /// current streak without fetching a fixed number of matches.
    pub fn recent_results(&self, puuid: &str, max: usize, want: usize) -> Vec<bool> {
        let ids = self.match_history(puuid).unwrap_or_default();
        let mut out = Vec::new();
        for id in ids.iter().take(max) {
            let Ok(details) = self.match_details(id) else {
                break;
            };
            let Some(win) = self.own_result(&details, puuid) else {
                break;
            };
            out.push(win);
            // A streak only needs the run up to the first reversal.
            if out.len() >= want || out.first() != Some(&win) {
                break;
            }
        }
        out
    }

    /// Whether *this player* won the given finished match, if it can be told.
    fn own_result(&self, details: &MatchDetails, puuid: &str) -> Option<bool> {
        let player = details.players.iter().find(|p| p.subject == puuid)?;
        let team_id = player.team_id.as_str();
        if team_id.is_empty() {
            return None;
        }
        // Explicit `won` wins when present; otherwise compare rounds.
        let team = details.teams.iter().find(|t| t.team_id == team_id)?;
        if team.won {
            return Some(true);
        }
        let best = details.teams.iter().map(|t| t.rounds_won).max().unwrap_or(0);
        if best > 0 && team.rounds_won == best {
            // Equal top scores mean a draw, which is neither a win nor a loss.
            let tied = details
                .teams
                .iter()
                .filter(|t| t.rounds_won == best)
                .count();
            if tied == 1 {
                return Some(true);
            }
        }
        Some(false)
    }

    /// How the Overwolf app infers premade groups without a live party API:
    /// find matches that 2+ lobby players share, then read the `partyId`
    /// those players had *in that match*. Single-member groups are dropped
    /// and overlapping groups are merged.
    pub fn shared_match_parties(&self, puuids: &[String]) -> Vec<Vec<String>> {
        let mut histories: Vec<(String, Vec<String>)> = Vec::new();
        std::thread::scope(|scope| {
            let handles: Vec<_> = puuids
                .iter()
                .map(|p| {
                    let me = &*self;
                    scope.spawn(move || (p.clone(), me.match_history(p).unwrap_or_default()))
                })
                .collect();
            for h in handles {
                if let Ok(v) = h.join() {
                    histories.push(v);
                }
            }
        });

        let mut by_match: HashMap<String, Vec<String>> = HashMap::new();
        for (puuid, ids) in &histories {
            for id in ids {
                by_match.entry(id.clone()).or_default().push(puuid.clone());
            }
        }
        let mut shared: Vec<(String, Vec<String>)> = by_match
            .into_iter()
            .filter(|(_, players)| players.len() >= 2)
            .collect();
        shared.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        shared.truncate(20);

        let mut groups: Vec<Vec<String>> = Vec::new();
        for (match_id, participants) in shared {
            let Ok(details) = self.match_details(&match_id) else {
                continue;
            };
            let mut by_party: HashMap<String, Vec<String>> = HashMap::new();
            for p in &details.players {
                if !participants.contains(&p.subject) || p.party_id.is_empty() {
                    continue;
                }
                by_party
                    .entry(p.party_id.clone())
                    .or_default()
                    .push(p.subject.clone());
            }
            for (_, members) in by_party {
                if members.len() >= 2 {
                    merge_group(&mut groups, members);
                }
            }
        }
        groups
    }

    /// Rank for every lobby member. Sequential with a small gap plus one 429
    /// retry: bursting 10 requests in parallel was tripping rate limits and
    /// silently degrading ranks to "Unrated".
    pub fn mmr_all(&self, puuids: &[String]) -> HashMap<String, Mmr> {
        let mut out = HashMap::new();
        for (i, p) in puuids.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(60));
            }
            let url = format!("{}/mmr/v1/players/{}", self.pd(), p);
            if let Some(v) = self.get_json(&url) {
                if let Ok(m) = serde_json::from_value::<Mmr>(v) {
                    out.insert(p.clone(), m);
                }
            }
        }
        out
    }
}

/// Premade groups exactly the way the tracker.gg app computes them: group the
/// match's own players by `partyId`, drop every one-player group, and keep the
/// rest. `partyId` only exists in match-details, so this is exact but needs the
/// match record (it 404s while the match is still live).
pub fn party_groups(details: &MatchDetails) -> Vec<Vec<String>> {
    let mut by_party: HashMap<String, Vec<String>> = HashMap::new();
    for p in &details.players {
        if p.party_id.is_empty() {
            continue;
        }
        by_party
            .entry(p.party_id.clone())
            .or_default()
            .push(p.subject.clone());
    }
    let mut groups: Vec<Vec<String>> = by_party
        .into_values()
        .filter(|g| g.len() >= 2)
        .collect();
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    groups
}

/// How long a resolved act ID is trusted before asking content-service again.
/// Acts last about two months, so this only matters for a client left running
/// across an act boundary; it also keeps us from re-querying on every poll.
const ACT_TTL_SECS: u64 = 6 * 60 * 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn act_cache_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(
        std::path::Path::new(&base)
            .join("valtrack")
            .join("act.txt"),
    )
}

/// Last act ID that resolved successfully, kept across restarts so a transient
/// content-service failure does not silently blank every rank.
fn read_act_cache() -> Option<String> {
    let path = act_cache_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn write_act_cache(id: &str) {
    let Some(path) = act_cache_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, id) {
        crate::log(&format!("actId: cache write failed: {e}"));
    }
}

fn merge_group(groups: &mut Vec<Vec<String>>, incoming: Vec<String>) {
    if let Some(existing) = groups
        .iter_mut()
        .find(|g| g.iter().any(|m| incoming.contains(m)))
    {
        for m in incoming {
            if !existing.contains(&m) {
                existing.push(m);
            }
        }
    } else {
        groups.push(incoming);
    }
}

fn ensure_ok(
    resp: reqwest::blocking::Response,
    what: &str,
) -> Result<reqwest::blocking::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().unwrap_or_default();
    let trimmed = body.trim();
    let short: String = trimmed.chars().take(200).collect();
    anyhow::bail!("{} returned {}: {}", what, status, short)
}
