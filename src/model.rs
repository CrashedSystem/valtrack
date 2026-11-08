//! Riot PD/GLZ API models (only the fields we actually render).
//! Unused fields are kept so the shapes stay faithful to the live payloads.

#![allow(dead_code)]

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerIdentity {
    #[serde(rename = "Subject")]
    pub subject: String,
    #[serde(rename = "AccountLevel", default)]
    pub account_level: i64,
    #[serde(rename = "Incognito", default)]
    pub incognito: bool,
    /// Set when the player asked the client to hide their account level.
    #[serde(rename = "HideAccountLevel", default)]
    pub hide_account_level: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GamePlayer {
    #[serde(rename = "Subject")]
    pub subject: String,
    #[serde(rename = "TeamID", default)]
    pub team_id: Option<String>,
    #[serde(rename = "CharacterID", default)]
    pub character_id: Option<String>,
    #[serde(rename = "CompetitiveTier", default)]
    pub competitive_tier: i64,
    #[serde(rename = "PlayerIdentity")]
    pub identity: Option<PlayerIdentity>,
}

impl GamePlayer {
    pub fn puuid(&self) -> &str {
        self.identity
            .as_ref()
            .map(|i| i.subject.as_str())
            .unwrap_or(&self.subject)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Team {
    #[serde(rename = "TeamID")]
    pub team_id: Option<String>,
    #[serde(rename = "TeamNumber", default)]
    pub team_number: Option<i64>,
    #[serde(rename = "Players", default)]
    pub players: Vec<GamePlayer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TeamRoster {
    #[serde(rename = "TeamID", default)]
    pub team_id: Option<String>,
    #[serde(rename = "Players", default)]
    pub players: Vec<GamePlayer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreGameMatch {
    #[serde(rename = "MatchID")]
    pub match_id: String,
    #[serde(rename = "State", default)]
    pub state: String,
    #[serde(rename = "MapID", default)]
    pub map_id: String,
    #[serde(rename = "ModeID", default)]
    pub mode_id: String,
    #[serde(rename = "Players", default)]
    pub players: Vec<GamePlayer>,
    #[serde(rename = "Teams", default)]
    pub teams: Vec<Team>,
    #[serde(rename = "MatchmakingData", default)]
    pub matchmaking: Option<MatchmakingData>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatchmakingData {
    #[serde(rename = "QueueID", default)]
    pub queue_id: String,
    #[serde(rename = "IsRanked", default)]
    pub is_ranked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PregameMatch {
    #[serde(rename = "ID", alias = "MatchID")]
    pub match_id: String,
    #[serde(rename = "MapID", default)]
    pub map_id: String,
    #[serde(rename = "QueueID", default)]
    pub queue_id: String,
    #[serde(rename = "Mode", default)]
    pub mode: String,
    #[serde(rename = "IsRanked", default)]
    pub is_ranked: bool,
    #[serde(rename = "PregameState", default)]
    pub pregame_state: String,
    #[serde(rename = "Teams", default)]
    pub teams: Vec<Team>,
    #[serde(rename = "AllyTeam", default)]
    pub ally_team: Option<TeamRoster>,
    #[serde(rename = "EnemyTeam", default)]
    pub enemy_team: Option<TeamRoster>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerMatchRef {
    #[serde(rename = "MatchID", default)]
    pub match_id: Option<String>,
    #[serde(rename = "QueueID", default)]
    pub queue_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NameEntry {
    #[serde(rename = "DisplayName", default)]
    pub display_name: String,
    #[serde(rename = "GameName", default)]
    pub game_name: String,
    #[serde(rename = "TagLine", default)]
    pub tag_line: String,
    #[serde(rename = "Subject", default)]
    pub subject: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LatestCompetitiveUpdate {
    #[serde(rename = "TierAfterUpdate", default)]
    pub tier: i64,
    #[serde(rename = "RankedRatingAfterUpdate", default)]
    pub rr: i64,
    #[serde(rename = "RankedRatingEarned", default)]
    pub rr_earned: i64,
    #[serde(rename = "SeasonID", default)]
    pub season_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Mmr {
    #[serde(rename = "LatestCompetitiveUpdate", default)]
    pub latest: LatestCompetitiveUpdate,
    #[serde(rename = "QueueSkills", default)]
    pub queue_skills: QueueSkills,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QueueSkills {
    #[serde(rename = "competitive", default)]
    pub competitive: CompetitiveSkills,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CompetitiveSkills {
    #[serde(rename = "SeasonalInfoBySeasonID", default)]
    pub seasonal: HashMap<String, SeasonInfo>,
}

/// Everything the client itself uses to render a rank: tier, RR and win rate
/// for the *current act*.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SeasonInfo {
    #[serde(rename = "CompetitiveTier", default)]
    pub tier: i64,
    #[serde(rename = "RankedRating", default)]
    pub rr: i64,
    #[serde(rename = "NumberOfGames", default)]
    pub games: i64,
    #[serde(rename = "NumberOfWins", default)]
    pub wins: i64,
}

/// `pd.../match-details/v1/matches/{id}` — the only live source of party IDs.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchDetails {
    #[serde(default)]
    pub players: Vec<MatchDetailPlayer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MatchDetailPlayer {
    #[serde(default)]
    pub subject: String,
    #[serde(rename = "partyId", default)]
    pub party_id: String,
}
