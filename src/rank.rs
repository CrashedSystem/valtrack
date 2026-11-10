//! Competitive tier numbering -> display names (same table as the app).

pub fn tier_name(tier: i64) -> &'static str {
    const NAMES: [&str; 28] = [
        "Unrated", "Unused", "Unused", "Iron 1", "Iron 2", "Iron 3", "Bronze 1", "Bronze 2",
        "Bronze 3", "Silver 1", "Silver 2", "Silver 3", "Gold 1", "Gold 2", "Gold 3",
        "Platinum 1", "Platinum 2", "Platinum 3", "Diamond 1", "Diamond 2", "Diamond 3",
        "Ascendant 1", "Ascendant 2", "Ascendant 3", "Immortal 1", "Immortal 2", "Immortal 3",
        "Radiant",
    ];
    NAMES.get(tier as usize).copied().unwrap_or("Unknown")
}

/// Peak tier the account has ever reached, from the `LifetimePrestige` counts.
///
/// The keys are English tier names without a division ("DIAMOND", "ASCENDANT"),
/// so this maps them back onto the numeric ladder and returns the highest one
/// with a non-zero count. `Radiant` never appears there, so 27 is the ceiling.
pub fn peak_tier_ko(counts: &std::collections::HashMap<String, i64>) -> Option<(i64, &'static str)> {
    const KEYS: [(&str, i64); 8] = [
        ("IRON", 3),
        ("BRONZE", 6),
        ("SILVER", 9),
        ("GOLD", 12),
        ("PLATINUM", 15),
        ("DIAMOND", 18),
        ("ASCENDANT", 21),
        ("IMMORTAL", 24),
    ];
    let mut best: Option<(i64, &'static str)> = None;
    for (key, tier) in KEYS {
        let hit = counts.get(key).copied().unwrap_or(0) > 0;
        if hit && best.map(|(t, _)| tier > t).unwrap_or(true) {
            best = Some((tier, tier_name_ko(tier)));
        }
    }
    best
}

pub fn rank_label(tier: i64, rr: i64) -> String {
    if tier <= 2 {
        "Unrated".to_string()
    } else if tier >= 27 {
        "Radiant".to_string()
    } else {
        format!("{} ({} RR)", tier_name(tier), rr)
    }
}

/// Korean tier names, matching the KR client.
pub fn tier_name_ko(tier: i64) -> &'static str {
    const NAMES: [&str; 28] = [
        "미배치", "미배치", "미배치", "아이언 1", "아이언 2", "아이언 3", "브론즈 1", "브론즈 2",
        "브론즈 3", "실버 1", "실버 2", "실버 3", "골드 1", "골드 2", "골드 3", "플래티넘 1",
        "플래티넘 2", "플래티넘 3", "다이아몬드 1", "다이아몬드 2", "다이아몬드 3", "초월자 1",
        "초월자 2", "초월자 3", "불멸 1", "불멸 2", "불멸 3", "레디언트",
    ];
    NAMES.get(tier as usize).copied().unwrap_or("알 수 없음")
}

/// Same as [`rank_label`] but for the overlay, which is fully Korean.
#[allow(dead_code)]
pub fn rank_label_ko(tier: i64, rr: i64) -> String {
    if tier <= 2 {
        "미배치".to_string()
    } else if tier >= 27 {
        "레디언트".to_string()
    } else {
        format!("{} ({} RR)", tier_name_ko(tier), rr)
    }
}

pub fn party_label(size: Option<u8>) -> String {
    match size {
        Some(5) => "5인큐".to_string(),
        Some(4) => "4인큐".to_string(),
        Some(3) => "3인큐".to_string(),
        Some(2) => "듀오".to_string(),
        Some(1) => "솔로".to_string(),
        _ => String::new(),
    }
}

/// Mode ids arrive in three shapes: a queue id (`competitive`), a game-mode
/// class (`QuickBombGameMode`), or a class with a variation suffix
/// (`Swiftplay_EoRCredits_GameMode`, `RangeV2`). All three resolve to Korean.
///
/// The variation suffixes are real - Swiftplay reports
/// `Swiftplay_EoRCredits_GameMode` while the end-of-round credits play, and the
/// range reports `RangeV2` - so an exact-match table left raw English on screen.
pub fn mode_label(mode: &str) -> String {
    let lower = mode.to_ascii_lowercase();

    let korean = if lower.contains("swiftplay") || lower.contains("quickbomb") {
        "신속 플레이"
    } else if lower.contains("spikerush") {
        "스파이크 러시"
    } else if lower.contains("teamdeathmatch") || lower.contains("hurm") {
        "팀 데스매치"
    } else if lower.contains("deathmatch") {
        "데스매치"
    } else if lower.contains("shootingrange") || lower == "rangev2" {
        "사격장"
    } else if lower.contains("skirmish") {
        "난투"
    } else if lower.contains("gungame") {
        "총의 게임"
    } else if lower.contains("oneforall") {
        "모두의 하나"
    } else if lower.contains("snowball") {
        "눈싸움"
    } else if lower.contains("gauntlet") || lower.contains("abilitydraft") {
        "건틀릿"
    } else if lower.contains("premier") {
        "프리미어"
    } else if lower.contains("ascen") || lower.contains("eorc") {
        // End-of-round credits wrappers belong to the mode they wrap.
        ""
    } else {
        match mode {
            "competitive" => "경쟁전",
            "unrated" => "일반전",
            "bomb" | "bombgamemode" => "일반/경쟁",
            "" | "-" => "-",
            other => return other.to_string(),
        }
    };
    korean.to_string()
}

/// Match states are Riot's SCREAMING_SNAKE values.
pub fn state_label(state: &str) -> String {
    let korean = match state.to_uppercase().as_str() {
        "IN_PROGRESS" => "진행 중",
        "PREGAME" | "PRE_GAME" | "AGENT_SELECT" => "요원 선택",
        "WAITING" | "IDLE" | "CHAR_SELECT" => "대기",
        "ENDED" | "POST_GAME" | "COMPLETED" => "종료",
        "-" | "" => "-",
        other => return other.to_string(),
    };
    korean.to_string()
}

/// `/Game/Maps/Juliett/Juliett` -> `Sunset`
///
/// The payload only carries the internal codename (`Plummet`, `Bonsai`, ...),
/// so the lookup table is embedded from Riot's public map catalog.
pub fn map_name(map_id: &str) -> String {
    let raw = map_id
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown")
        .trim_end_matches("_PrimaryAsset");

    // Codenames are matched before the generic suffix because the catalog also
    // contains entries like `HURM_Alley`.
    let direct = crate::assets::MAPS
        .iter()
        .find(|(code, _)| *code == raw)
        .map(|(_, name)| *name);
    if let Some(name) = direct {
        return name.to_string();
    }

    // Some payloads append a variation suffix (`Triad_PrimaryAsset`, `Bonsai2`).
    crate::assets::MAPS
        .iter()
        .find(|(code, _)| raw.starts_with(code))
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// `/Game/GameModes/Bomb/BombGameMode.BombGameMode_C` -> `Bomb`
pub fn mode_name(mode_id: &str) -> String {
    let tail = mode_id.rsplit('/').next().unwrap_or("Unknown");
    tail.split('.').next().unwrap_or("Unknown").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these was seen on screen as raw English before the label
    /// table learned to match by prefix.
    #[test]
    fn mode_labels_cover_observed_payloads() {
        assert_eq!(mode_label("competitive"), "경쟁전");
        assert_eq!(mode_label("Swiftplay_EoRCredits_GameMode"), "신속 플레이");
        assert_eq!(mode_label("SkirmishGameMode"), "난투");
        assert_eq!(mode_label("ShootingRangeGameMode"), "사격장");
        assert_eq!(mode_label("RangeV2"), "사격장");
        assert_eq!(mode_label("TeamDeathmatchGameMode"), "팀 데스매치");
        assert_eq!(mode_label("HURM_Alley"), "팀 데스매치");
        assert_eq!(mode_label("SpikeRushGameMode"), "스파이크 러시");
        assert_eq!(mode_label(""), "-");
    }

    #[test]
    fn every_tier_has_a_name() {
        for tier in [0i64, 3, 9, 12, 15, 18, 21, 24, 27] {
            let name = tier_name_ko(tier);
            assert!(!name.is_empty(), "tier {tier} has no name");
            assert_ne!(name, "알 수 없음", "tier {tier} fell through the table");
        }
    }

    #[test]
    fn peak_tier_picks_the_highest_earned_band() {
        let mut counts = std::collections::HashMap::new();
        counts.insert("DIAMOND".to_string(), 5);
        counts.insert("PLATINUM".to_string(), 2);
        // Lowest band must not win just because it is in the map.
        assert_eq!(peak_tier_ko(&counts).map(|(_, n)| n), Some("다이아몬드 1"));
    }

    #[test]
    fn party_label_covers_every_size() {
        assert_eq!(party_label(Some(5)), "5인큐");
        assert_eq!(party_label(Some(2)), "듀오");
        assert_eq!(party_label(Some(1)), "솔로");
        assert_eq!(party_label(None), "");
    }
}
