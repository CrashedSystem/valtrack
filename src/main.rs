mod agent;
mod assets;
mod auth;
mod fonts;
mod icons;
mod install;
mod l10n;
mod model;
mod overlay;
mod paths;
mod rank;
mod riot;
mod update;
mod ws;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// Set when the active act could not be resolved from content-service and no
/// cached ID exists. Ranks would otherwise silently fall back to last season.
pub static ACT_UNKNOWN: AtomicBool = AtomicBool::new(false);

#[derive(Parser)]
#[command(
    name = "valtrack",
    version,
    about = "Minimal VALORANT live-match tracker (no Overwolf)"
)]
struct Cli {
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Clone)]
enum Cmd {
    /// Show the current lobby with ranks and agents (default)
    Live,
    /// Re-print whenever the lobby changes
    Watch {
        /// Seconds between polls
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },
    /// Always-on-top, click-through overlay
    Overlay {
        /// Render a sample lobby (no game needed)
        #[arg(long)]
        demo: bool,
    },
    /// Show account, region, client version and token state
    Whoami,
    /// Run the overlay only while VALORANT is up (invisible supervisor)
    Agent {
        /// Stop a running agent instead of starting one
        #[arg(long)]
        stop: bool,
    },
    /// Start the agent with Windows (login autostart)
    Autostart {
        /// Remove the login entry instead of adding it
        #[arg(long)]
        remove: bool,
    },
    /// Copy this build to %LOCALAPPDATA%, start it at login and launch the agent
    Install,
    /// Remove the login entry, stop the agent and delete the installed copy
    Uninstall,
    /// Show what is installed, whether it runs at login, and whether it is live
    Status,
    /// Dump the live API payloads so we can see which fields exist
    Diag,
}

#[derive(Clone)]
pub struct Row {
    pub puuid: String,
    /// True for the local player. The overlay tints that row so you can find
    /// yourself in a ten-player lobby without reading every name.
    pub is_me: bool,
    pub name: String,
    pub level: i64,
    /// Streamer/incognito mode: the client hides this player's identity from
    /// other players, so the name column carries a `[S]` marker.
    pub incognito: bool,
    /// The player asked the client to hide their account level.
    pub hide_level: bool,
    pub rank: String,
    pub agent: String,
    /// Agent UUID, used for the icon.
    pub agent_id: String,
    /// Competitive tier number, used for the rank icon.
    pub tier: i64,
    /// Ranked rating for the current act (only meaningful with `tier`).
    pub rr: i64,
    /// RR gained or lost in the most recent ranked match. A player trending up
    /// is a different opponent from one bleeding points, even at the same tier.
    pub rr_earned: i64,
    /// Current streak: positive = wins in a row, negative = losses. Zero when
    /// unknown or the last result was undetermined.
    pub streak: i32,
    /// Highest tier this account has ever reached, as a Korean tier name. The
    /// current act resets every few months; peak is the better skill signal.
    pub peak: Option<String>,
    /// Games of derank protection left. Meaningful only when `at_derank_tier`:
    /// zero there means the next loss demotes.
    pub derank_games: i64,
    /// The rank sits at the bottom division of its tier, where demotion applies.
    pub at_derank_tier: bool,
    /// Riot has penalised this account for leaving or going AFK.
    pub penalized: bool,
    /// Size of this player's party, when it can be inferred.
    pub party: Option<u8>,
    /// Wins / games in the current act (0 when unknown).
    pub wins: i64,
    pub games: i64,
}

#[derive(Clone)]
pub struct Lobby {
    pub origin: &'static str,
    pub match_id: String,
    pub map: String,
    pub mode: String,
    pub state: String,
    pub ranked: bool,
    pub allies: Vec<Row>,
    pub enemies: Vec<Row>,
}

/// Lightweight file log at `%LOCALAPPDATA%\valtrack\valtrack.log`.
///
/// The overlay is a windowless background process once its console is hidden,
/// so this is the only way to tell "did it even start" from "did it exit".
pub fn log(msg: &str) {
    let Ok(base) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let dir = std::path::Path::new(&base).join("valtrack");
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("valtrack.log"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{secs} pid={} {msg}", std::process::id());
    }
}

fn main() {
    log(&format!("start args={:?}", std::env::args().skip(1).collect::<Vec<_>>()));

    // Hide the console before anything else when started from the desktop icon
    // (no arguments = overlay) so there is no visible flash.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(String::as_str);
    let overlay_like = match first {
        None => true,
        Some("overlay") => true,
        // The supervisor is invisible by design; `--stop` stays visible so the
        // user can actually read the result.
        Some("agent") => !args.iter().any(|a| a == "--stop"),
        _ => false,
    };
    if overlay_like {
        overlay::hide_console();
    }
    log("console handled");

    if let Err(err) = run() {
        log(&format!("fatal: {err:#}"));
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    // No subcommand means "double-clicked": start the overlay. A .lnk shortcut
    // is not an option here, so the desktop item is the exe itself.
    let cmd = cli.cmd.unwrap_or(if cli.json {
        Cmd::Live
    } else {
        Cmd::Overlay { demo: false }
    });
    match cmd {
        Cmd::Live => once(cli.json),
        Cmd::Watch { interval } => watch(cli.json, interval.max(2)),
        Cmd::Overlay { demo } => overlay::run(demo),
        Cmd::Whoami => whoami(cli.json),
        Cmd::Agent { stop } => {
            if stop {
                agent::stop()
            } else {
                agent::run()
            }
        }
        Cmd::Autostart { remove } => agent::autostart(remove),
        Cmd::Install => agent::install(),
        Cmd::Uninstall => agent::uninstall(),
        Cmd::Status => agent::status(),
        Cmd::Diag => diag(),
    }
}

/// Dumps the live payloads we read so we can see what else they carry.
fn diag() -> Result<()> {
    let riot = connect()?;
    for (name, url) in riot.debug_urls() {
        match riot.debug_json(&url) {
            Some(v) => {
                println!("=== {name} ===");
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            None => println!("=== {name} === (unavailable)"),
        }
    }
    if let Some(id) = riot.active_match_id().ok().flatten() {
        println!("=== match-details ({id}) ===");
        if let Some(v) = riot.debug_match_details(&id) {
            if let Some(players) = v.get("players").and_then(|p| p.as_array()) {
                if let Some(first) = players.first() {
                    println!("player: {first}");
                }
            }
            if let Some(teams) = v.get("teams") {
                println!("teams: {teams}");
            }
        } else {
            println!("(unavailable)");
        }
    }
    diag_lobby(&riot);
    Ok(())
}

/// One line per lobby player with the fields the overlay acts on, so the
/// penalty and demotion flags can be checked against a real lobby.
fn diag_lobby(riot: &riot::Riot) {
    let Ok(Some(lobby)) = fetch_lobby(riot) else {
        return;
    };
    let mut ids: Vec<String> = lobby.allies.iter().map(|r| r.puuid.clone()).collect();
    ids.extend(lobby.enemies.iter().map(|r| r.puuid.clone()));
    let names = riot.names(&ids).unwrap_or_default();
    let mmrs = riot.mmr_all(&ids);

    println!();
    println!("=== lobby penalty / demotion ===");
    println!(
        "{:<24} {:>5} {:>4} {:>5} {:>7} {:>6}",
        "player", "tier", "rr", "derank", "protect", "afk"
    );
    for id in &ids {
        let label = names
            .get(id)
            .map(|n| {
                if !n.game_name.is_empty() {
                    format!("{}#{}", n.game_name, n.tag_line)
                } else {
                    n.display_name.clone()
                }
            })
            .unwrap_or_else(|| id.chars().take(8).collect());
        match mmrs.get(id) {
            Some(m) => {
                // Same rule the overlay uses: a demotion match needs 0 RR at the
                // bottom division. Printing the raw flag made the table claim
                // "derank" for a player still sitting on 74 RR.
                let demotion = m.at_derank_tier && m.latest.rr == 0;
                println!(
                    "{:<24} {:>5} {:>4} {:>5} {:>7} {:>6}",
                    label,
                    m.latest.tier,
                    m.latest.rr,
                    if demotion { "yes" } else { "-" },
                    m.derank_games_left,
                    m.latest.afk_penalty
                )
            }
            None => println!("{:<24} (mmr unavailable)", label),
        }
    }
}

pub fn connect() -> Result<riot::Riot> {
    let lockfile = paths::read_lockfile()?;
    let log = paths::read_game_log()?;
    let tokens = auth::fetch_tokens(&lockfile)?;
    riot::Riot::new(
        &tokens,
        log.region,
        log.shard,
        log.client_version,
        lockfile.port,
        &lockfile.password,
    )
}

fn once(json: bool) -> Result<()> {
    let riot = connect()?;
    match build_lobby(&riot)? {
        Some(lobby) => {
            if json {
                println!("{}", to_json(&lobby));
            } else {
                print_lobby(&lobby);
            }
        }
        None => println!("No live match. Start a game (or agent select) and run again."),
    }
    Ok(())
}

fn watch(json: bool, interval: u64) -> Result<()> {
    let mut last = String::new();
    loop {
        match connect().and_then(|riot| build_lobby(&riot)) {
            Ok(Some(lobby)) => {
                let fingerprint = fingerprint(&lobby);
                if fingerprint != last {
                    last = fingerprint;
                    if json {
                        println!("{}", to_json(&lobby));
                    } else {
                        print_lobby(&lobby);
                    }
                }
            }
            Ok(None) => {
                if last != "none" {
                    last = "none".into();
                    println!("[{}] no live match", timestamp());
                }
            }
            Err(err) => {
                if last != "error" {
                    last = "error".into();
                    eprintln!("[{}] {err:#}", timestamp());
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}

fn whoami(json: bool) -> Result<()> {
    let lockfile = paths::read_lockfile()?;
    let log = paths::read_game_log()?;
    let tokens = auth::fetch_tokens(&lockfile)?;
    let riot = riot::Riot::new(
        &tokens,
        log.region.clone(),
        log.shard.clone(),
        log.client_version.clone(),
        lockfile.port,
        &lockfile.password,
    )?;
    let mmr = riot.mmr(&tokens.subject).ok();
    let act = riot.current_act_id();
    let seasonal = mmr
        .as_ref()
        .and_then(|m| act.as_ref().and_then(|a| m.queue_skills.competitive.seasonal.get(a)));
    let (tier, rr, wins, games) = match seasonal {
        Some(s) => (s.tier, s.rr, s.wins, s.games),
        None => (
            mmr.as_ref().map(|m| m.latest.tier).unwrap_or(0),
            mmr.as_ref().map(|m| m.latest.rr).unwrap_or(0),
            0,
            0,
        ),
    };
    let wr = if games > 0 {
        format!("{}% ({}/{})", wins * 100 / games, wins, games)
    } else {
        "-".to_string()
    };

    if json {
        let out = serde_json::json!({
            "puuid": tokens.subject,
            "region": riot.region,
            "shard": riot.shard,
            "client_version": riot.client_version,
            "rank": rank::rank_label(tier, rr),
            "act_id": act,
            "wins": wins,
            "games": games,
            "lockfile_port": lockfile.port,
        });
        println!("{out}");
        return Ok(());
    }

    println!("puuid          {}", tokens.subject);
    println!("region / shard {} / {}", riot.region, riot.shard);
    println!("client version {}", riot.client_version);
    println!("issuer         {}", tokens.issuer);
    println!("local api      127.0.0.1:{} (lockfile)", lockfile.port);
    println!("rank           {}", rank::rank_label(tier, rr));
    println!("win rate       {wr}");
    Ok(())
}

/// Cheap path: active match + roster only (1-2 requests).
pub fn fetch_lobby(riot: &riot::Riot) -> Result<Option<Lobby>> {
    let Some(match_id) = riot.active_match_id()? else {
        return Ok(None);
    };

    let mut lobby = match riot.core_match(&match_id) {
        Ok(m) => lobby_from_core(&m, &riot.puuid),
        Err(_) => {
            let m = riot
                .pregame_match(&match_id)
                .context("match id found but neither core-game nor pregame returned it")?;
            lobby_from_pregame(&m, &riot.puuid)
        }
    };
    lobby.match_id = match_id;
    Ok(Some(lobby))
}

/// Expensive path: names, ranks, agents, party grouping.
pub fn enrich_lobby(riot: &riot::Riot, lobby: &mut Lobby) {
    let mut puuids: Vec<String> = lobby.allies.iter().map(|r| r.puuid.clone()).collect();
    puuids.extend(lobby.enemies.iter().map(|r| r.puuid.clone()));

    let names = riot.names(&puuids).unwrap_or_default();
    let mmrs = riot.mmr_all(&puuids);
    let agents = agent_map();
    let act_id = riot.current_act_id();
    // Rank and win rate key off the active act. If it cannot be resolved we say
    // so in the overlay instead of silently showing last season's snapshot.
    ACT_UNKNOWN.store(act_id.is_none(), Ordering::Relaxed);

    for row in lobby.allies.iter_mut().chain(lobby.enemies.iter_mut()) {
        if let Some(n) = names.get(&row.puuid) {
            let best = if !n.display_name.is_empty() {
                n.display_name.clone()
            } else if !n.game_name.is_empty() {
                format!("{}#{}", n.game_name, n.tag_line)
            } else {
                row.name.clone()
            };
            row.name = best;
        }

        if let Some(m) = mmrs.get(&row.puuid) {
            // The client renders the *current act* entry from QueueSkills; the
            // LatestCompetitiveUpdate field goes stale for idle players, which
            // is what produced wrong "Unrated"/old-tier rows.
            let seasonal = act_id
                .as_ref()
                .and_then(|act| m.queue_skills.competitive.seasonal.get(act));

            match seasonal {
                Some(s) if s.tier > 0 => {
                    row.rank = rank::rank_label(s.tier, s.rr);
                    row.tier = s.tier;
                    row.rr = s.rr;
                    row.wins = s.wins;
                    row.games = s.games;
                }
                _ if m.latest.tier > 0 => {
                    row.rank = rank::rank_label(m.latest.tier, m.latest.rr);
                    row.tier = m.latest.tier;
                    row.rr = m.latest.rr;
                }
                _ => {}
            }
            // Trend, independent of which branch above supplied the rank.
            row.rr_earned = m.latest.rr_earned;
            // Peak history: the highest division-free tier they ever held.
            if let Some((_, name)) = rank::peak_tier_ko(
                &m.lifetime_prestige
                    .iter()
                    .map(|(k, v)| (k.clone(), v.count))
                    .collect(),
            ) {
                row.peak = Some(name.to_string());
            }
            // Demotion risk. Only meaningful at the bottom division of a tier;
            // elsewhere the flag is false and the overlay says nothing.
            row.derank_games = m.derank_games_left;
            row.at_derank_tier = m.at_derank_tier;
            row.penalized = m.latest.afk_penalty != 0 || m.latest.rr_penalty != 0;

            // Placement games show tier 0 but still have a match count.
            if row.games == 0 {
                if let Some(s) = seasonal {
                    row.wins = s.wins;
                    row.games = s.games;
                }
            }
        }

        row.agent = agents
            .get(&row.agent)
            .cloned()
            .unwrap_or_else(|| shorten(&row.agent));
    }

    // Party sizes, in order of reliability (tracker.gg uses step 1 only):
    //   1. match-details `partyId` -> exact grouping, but 404 while live
    //   2. your own party -> exact, our team only (Riot blocks other players)
    //   3. friends' presence -> friends only
    //   4. shared recent matches -> heuristic, fills whatever 1-3 could not
    let mut sizes: HashMap<String, u8> = HashMap::new();
    // Players whose party status is already exact. Solo players land here too,
    // so the history heuristic can never promote them into a premade group.
    let mut known: HashSet<String> = HashSet::new();

    if let Ok(details) = riot.match_details(&lobby.match_id) {
        for p in &details.players {
            if !p.party_id.is_empty() {
                known.insert(p.subject.clone());
            }
        }
        for group in crate::riot::party_groups(&details) {
            let n = group.len() as u8;
            for m in group {
                sizes.insert(m, n);
            }
        }
    }

    if let Ok(members) = riot.my_party() {
        if members.len() >= 2 {
            let n = members.len() as u8;
            for m in members {
                known.insert(m.clone());
                sizes.entry(m).or_insert(n);
            }
        }
    }

    if let Ok(friends) = riot.friend_party_sizes() {
        for (puuid, size) in friends {
            known.insert(puuid.clone());
            sizes.entry(puuid).or_insert(size);
        }
    }

    // Always infer premades from matches the lobby players already shared, but
    // skip any group that would contradict an exact source.
    let ally_ids: Vec<String> = lobby.allies.iter().map(|r| r.puuid.clone()).collect();
    let enemy_ids: Vec<String> = lobby.enemies.iter().map(|r| r.puuid.clone()).collect();
    for team in [&ally_ids, &enemy_ids] {
        if team.len() > 1 {
            for group in riot.shared_match_parties(team) {
                if group.iter().any(|m| known.contains(m)) {
                    continue;
                }
                let n = group.len() as u8;
                for m in group {
                    sizes.entry(m).or_insert(n);
                }
            }
        }
    }

    for row in lobby.allies.iter_mut().chain(lobby.enemies.iter_mut()) {
        row.party = sizes.get(&row.puuid).copied();
    }

    // Streaks. Each player costs up to STREAK_MATCHES match-details calls, so
    // this runs in parallel across the lobby and stops at the first reversal
    // inside recent_results(). Players whose history is unavailable stay at 0
    // and the overlay simply omits the line.
    let all_ids: Vec<String> = lobby
        .allies
        .iter()
        .chain(lobby.enemies.iter())
        .map(|r| r.puuid.clone())
        .collect();
    let mut streaks: HashMap<String, i32> = HashMap::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = all_ids
            .iter()
            .map(|p| {
                let me = &*riot;
                scope.spawn(move || (p.clone(), me.recent_results(p, STREAK_MATCHES, STREAK_CAP)))
            })
            .collect();
        for h in handles {
            if let Ok((puuid, results)) = h.join() {
                let Some(&first) = results.first() else {
                    continue;
                };
                let run = results.iter().take_while(|w| **w == first).count() as i32;
                // A single result is a real 1-game streak, not noise: label it.
                streaks.insert(puuid, if first { run } else { -run });
            }
        }
    });
    for row in lobby.allies.iter_mut().chain(lobby.enemies.iter_mut()) {
        row.streak = streaks.get(&row.puuid).copied().unwrap_or(0);
    }
}

/// How far back to look for a streak, and when to stop counting.
const STREAK_MATCHES: usize = 8;
const STREAK_CAP: usize = 5;

/// Agent UUID -> display name. Embedded (`assets.rs`), so no network at runtime.
fn agent_map() -> HashMap<String, String> {
    assets::AGENT_NAMES
        .iter()
        .map(|(uuid, name)| (uuid.to_string(), name.to_string()))
        .collect()
}

pub fn build_lobby(riot: &riot::Riot) -> Result<Option<Lobby>> {
    let Some(mut lobby) = fetch_lobby(riot)? else {
        return Ok(None);
    };
    enrich_lobby(riot, &mut lobby);
    Ok(Some(lobby))
}

fn lobby_from_core(m: &model::CoreGameMatch, my_puuid: &str) -> Lobby {
    let my_team = m
        .players
        .iter()
        .find(|p| p.puuid() == my_puuid)
        .and_then(|p| p.team_id.clone());

    let mut allies = Vec::new();
    let mut enemies = Vec::new();
    for p in &m.players {
        let row = row_of(p);
        if Some(&p.team_id) == Some(&my_team) || p.team_id.as_deref() == my_team.as_deref() {
            allies.push(row);
        } else {
            enemies.push(row);
        }
    }

    mark_me(&mut allies, &mut enemies, my_puuid);

    Lobby {
        origin: "core-game",
        match_id: m.match_id.clone(),
        map: rank::map_name(&m.map_id),
        mode: rank::mode_name(&m.mode_id),
        state: if m.state.is_empty() {
            "-".into()
        } else {
            m.state.clone()
        },
        ranked: m.matchmaking.as_ref().map(|q| q.is_ranked).unwrap_or(false),
        allies,
        enemies,
    }
}

fn lobby_from_pregame(m: &model::PregameMatch, my_puuid: &str) -> Lobby {
    // During agent select only your own team is exposed; the enemy roster is
    // empty until the match starts.
    let mut allies: Vec<Row> = Vec::new();
    let mut enemies: Vec<Row> = Vec::new();

    let my_team = m
        .teams
        .iter()
        .find(|t| t.players.iter().any(|p| p.puuid() == my_puuid))
        .and_then(|t| t.team_id.clone())
        .or_else(|| m.ally_team.as_ref().and_then(|t| t.team_id.clone()));

    for team in &m.teams {
        let is_mine = team.team_id == my_team;
        for p in &team.players {
            if is_mine {
                allies.push(row_of(p));
            } else {
                enemies.push(row_of(p));
            }
        }
    }

    if allies.is_empty() {
        allies = m
            .ally_team
            .as_ref()
            .map(|t| t.players.iter().map(row_of).collect())
            .unwrap_or_default();
    }
    if enemies.is_empty() {
        enemies = m
            .enemy_team
            .as_ref()
            .map(|t| t.players.iter().map(row_of).collect())
            .unwrap_or_default();
    }

    mark_me(&mut allies, &mut enemies, my_puuid);

    Lobby {
        origin: "pregame",
        match_id: m.match_id.clone(),
        map: rank::map_name(&m.map_id),
        mode: rank::mode_name(if m.queue_id.is_empty() {
            &m.mode
        } else {
            m.queue_id.as_str()
        }),
        state: if m.pregame_state.is_empty() {
            "AGENT_SELECT".into()
        } else {
            m.pregame_state.to_uppercase()
        },
        ranked: m.is_ranked,
        allies,
        enemies,
    }
}

fn row_of(p: &model::GamePlayer) -> Row {
    let incognito = p.identity.as_ref().map(|i| i.incognito).unwrap_or(false);
    let hide_level = p
        .identity
        .as_ref()
        .map(|i| i.hide_account_level)
        .unwrap_or(false);
    Row {
        puuid: p.puuid().to_string(),
        is_me: false,
        name: "hidden".into(),
        level: p.identity.as_ref().map(|i| i.account_level).unwrap_or(0),
        incognito,
        hide_level,
        rank: rank::rank_label(p.competitive_tier, 0),
        agent: p.character_id.clone().unwrap_or_default(),
        agent_id: p.character_id.clone().unwrap_or_default(),
        tier: p.competitive_tier,
        rr: 0,
        rr_earned: 0,
        streak: 0,
        peak: None,
        derank_games: 0,
        at_derank_tier: false,
        penalized: false,
        party: None,
        wins: 0,
        games: 0,
    }
}

/// Flags the local player's row in both lists so the overlay can tint it.
fn mark_me(allies: &mut [Row], enemies: &mut [Row], my_puuid: &str) {
    for r in allies.iter_mut().chain(enemies.iter_mut()) {
        r.is_me = r.puuid == my_puuid;
    }
}

fn print_lobby(l: &Lobby) {
    println!();
    println!(
        "VALORANT  {}  |  {}  |  {}  |  ranked: {}",
        l.map,
        l.mode,
        l.state,
        if l.ranked { "yes" } else { "no" }
    );
    println!("match {} ({})", l.match_id, l.origin);
    println!();
    print_team("YOUR TEAM", &l.allies);
    println!();
    if l.enemies.is_empty() {
        println!("ENEMY TEAM");
        println!("  hidden until the match starts");
    } else {
        print_team("ENEMY TEAM", &l.enemies);
    }
    println!();
}

fn print_team(title: &str, rows: &[Row]) {
    println!("{title}");
    println!(
        "  {:<3} {:<22} {:>4}  {:<19} {:<12} {:>5} {}",
        "#", "Riot ID", "Lvl", "Rank", "Agent", "WR", "Party"
    );
    for (i, r) in rows.iter().enumerate() {
        let wr = if r.games > 0 {
            format!("{}%", (r.wins * 100 / r.games).clamp(0, 100))
        } else {
            "-".to_string()
        };
        // Streamer mode hides the name from other clients; level can be hidden
        // independently of that. Both are shown as markers, not as real data.
        let name = if r.incognito {
            format!("{} [S]", truncate(&r.name, 18))
        } else {
            truncate(&r.name, 22)
        };
        let level = if r.hide_level {
            "-".to_string()
        } else {
            r.level.to_string()
        };
        println!(
            "  {:<3} {:<22} {:>4}  {:<19} {:<12} {:>5} {}",
            i + 1,
            name,
            level,
            truncate(&r.rank, 19),
            truncate(&r.agent, 12),
            wr,
            rank::party_label(r.party)
        );
    }
}

fn to_json(l: &Lobby) -> String {
    let row = |r: &Row| {
        serde_json::json!({
            "puuid": r.puuid,
            "name": r.name,
            "level": if r.hide_level { 0 } else { r.level },
            "level_hidden": r.hide_level,
            "incognito": r.incognito,
            "rank": r.rank,
            "agent": r.agent,
            "party": r.party,
            "wins": r.wins,
            "games": r.games,
        })
    };
    let out = serde_json::json!({
        "match_id": l.match_id,
        "source": l.origin,
        "map": l.map,
        "mode": l.mode,
        "state": l.state,
        "ranked": l.ranked,
        "allies": l.allies.iter().map(row).collect::<Vec<_>>(),
        "enemies": l.enemies.iter().map(row).collect::<Vec<_>>(),
    });
    out.to_string()
}

fn fingerprint(l: &Lobby) -> String {
    let mut s = format!("{}|{}|", l.match_id, l.state);
    for r in l.allies.iter().chain(l.enemies.iter()) {
        s.push_str(&r.puuid);
        s.push(',');
    }
    s
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("+{}s", now.as_secs())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('~');
        out
    }
}

fn shorten(uuid: &str) -> String {
    if uuid.is_empty() {
        "-".into()
    } else {
        uuid.chars().take(8).collect()
    }
}
