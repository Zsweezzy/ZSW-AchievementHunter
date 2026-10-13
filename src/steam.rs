use crate::model::{Achievement, Game};
use crate::vdf;
use serde_json::Value;
use std::collections::HashMap;

pub fn installed_apps() -> Vec<(u32, String)> {
    crate::paths::steam_root()
        .map(|root| vdf::parse_installed_apps(&root))
        .unwrap_or_default()
}

pub fn local_steam_id() -> Option<String> {
    crate::paths::steam_root().and_then(|root| vdf::parse_most_recent_steam_id(&root))
}

pub fn parse_owned_games(value: &Value) -> Vec<Game> {
    let games = match value.get("response").and_then(|r| r.get("games")) {
        Some(Value::Array(items)) => items,
        _ => return Vec::new(),
    };
    games
        .iter()
        .filter_map(|item| {
            let appid = item.get("appid")?.as_u64()? as u32;
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_owned();
            let playtime_minutes = item
                .get("playtime_forever")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let last_played = item
                .get("rtime_last_played")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(Game {
                appid,
                name,
                playtime_minutes,
                last_played,
                has_achievements: item
                    .get("has_community_visible_stats")
                    .and_then(|v| v.as_bool()),
                ..Default::default()
            })
        })
        .collect()
}

pub fn parse_achievements(value: &Value) -> Result<Vec<Achievement>, String> {
    let stats = value
        .get("playerstats")
        .ok_or_else(|| "no playerstats in response".to_owned())?;
    if let Some(error) = stats.get("error").and_then(|v| v.as_str()) {
        return Err(error.to_owned());
    }
    let items = stats
        .get("achievements")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "game has no achievement schema".to_owned())?;
    Ok(items
        .iter()
        .map(|item| {
            let api_name = item
                .get("apiname")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let unlocked = item.get("achieved").and_then(|v| v.as_u64()) == Some(1);
            let unlock_time = item
                .get("unlocktime")
                .and_then(|v| v.as_u64())
                .filter(|t| *t > 0);
            Achievement {
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&api_name)
                    .to_owned(),
                description: item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                api_name,
                unlocked,
                unlock_time,
                ..Default::default()
            }
        })
        .collect())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchemaEntry {
    pub display_name: String,
    pub description: String,
    pub icon: String,
    pub icon_gray: String,
}

pub fn parse_schema(value: &Value, appid: u32) -> HashMap<String, SchemaEntry> {
    let mut out = HashMap::new();
    let Some(items) = value
        .get("game")
        .and_then(|g| g.get("availableGameStats"))
        .and_then(|s| s.get("achievements"))
        .and_then(|v| v.as_array())
    else {
        return out;
    };
    for item in items {
        let Some(api_name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        out.insert(
            api_name.to_owned(),
            SchemaEntry {
                display_name: item
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or(api_name)
                    .to_owned(),
                description: item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                icon: icon_url(
                    item.get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                    appid,
                ),
                icon_gray: icon_url(
                    item.get("icongray")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default(),
                    appid,
                ),
            },
        );
    }
    out
}

fn icon_url(raw: &str, appid: u32) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if raw.starts_with("http") {
        return raw.to_owned();
    }
    format!(
        "https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/{appid}/{raw}.jpg"
    )
}

pub fn apply_schema(achievements: &mut [Achievement], schema: &HashMap<String, SchemaEntry>) {
    for achievement in achievements.iter_mut() {
        let Some(entry) = schema.get(&achievement.api_name) else {
            continue;
        };
        if achievement.name == achievement.api_name || achievement.name.is_empty() {
            achievement.name = entry.display_name.clone();
        }
        if achievement.description.is_empty() {
            achievement.description = entry.description.clone();
        }
        achievement.icon = entry.icon.clone();
        achievement.icon_gray = entry.icon_gray.clone();
    }
}

pub fn schema(key: &str, appid: u32) -> Result<HashMap<String, SchemaEntry>, String> {
    if key.is_empty() {
        return Err("no steam api key".to_owned());
    }
    let url = format!(
        "https://api.steampowered.com/ISteamUserStats/GetSchemaForGame/v2/?key={key}&appid={appid}&l=en&format=json"
    );
    Ok(parse_schema(&crate::http::get_json(&url)?, appid))
}

pub fn parse_global_percentages(value: &Value) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    let items = value
        .get("achievementpercentages")
        .and_then(|v| v.get("achievements"))
        .and_then(|v| v.as_array());
    if let Some(items) = items {
        for item in items {
            if let (Some(name), Some(percent)) = (
                item.get("name").and_then(|v| v.as_str()),
                item.get("percent").and_then(|v| v.as_f64()),
            ) {
                out.insert(name.to_owned(), percent as f32);
            }
        }
    }
    out
}

pub fn parse_vanity(value: &Value) -> Option<String> {
    let response = value.get("response")?;
    if response.get("success").and_then(|v| v.as_u64()) != Some(1) {
        return None;
    }
    response
        .get("steamid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
}

pub fn owned_games(key: &str, steam_id: &str) -> Result<Vec<Game>, String> {
    let url = format!(
        "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key={key}&steamid={steam_id}&include_appinfo=1&include_played_free_games=1&format=json"
    );
    let value = crate::http::get_json(&url)?;
    let games = parse_owned_games(&value);
    if games.is_empty() {
        return Err(
            "no games returned; check the API key, the SteamID64, and that your game details are public"
                .to_owned(),
        );
    }
    Ok(games)
}

pub fn achievements(key: &str, steam_id: &str, appid: u32) -> Result<Vec<Achievement>, String> {
    let url = format!(
        "https://api.steampowered.com/ISteamUserStats/GetPlayerAchievements/v1/?key={key}&steamid={steam_id}&appid={appid}&l=en&format=json"
    );
    parse_achievements(&crate::http::get_json(&url)?)
}

pub fn global_percentages(appid: u32) -> Result<HashMap<String, f32>, String> {
    let url = format!(
        "https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={appid}&format=json"
    );
    Ok(parse_global_percentages(&crate::http::get_json(&url)?))
}

pub fn resolve_vanity(key: &str, vanity: &str) -> Result<String, String> {
    let url = format!(
        "https://api.steampowered.com/ISteamUser/ResolveVanityURL/v1/?key={key}&vanityurl={vanity}&format=json"
    );
    parse_vanity(&crate::http::get_json(&url)?)
        .ok_or_else(|| format!("could not resolve vanity url '{vanity}'"))
}

pub fn apply_installed(games: &mut [Game], installed: &[(u32, String)]) {
    let installed: HashMap<u32, &str> = installed
        .iter()
        .map(|(appid, name)| (*appid, name.as_str()))
        .collect();
    for game in games.iter_mut() {
        game.installed = installed.contains_key(&game.appid);
    }
}

pub fn games_from_installed_only(installed: &[(u32, String)]) -> Vec<Game> {
    installed
        .iter()
        .map(|(appid, name)| Game {
            appid: *appid,
            name: name.clone(),
            installed: true,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crucial_parses_owned_games_payload() {
        let value: Value = serde_json::from_str(
            r#"{"response":{"game_count":2,"games":[
                {"appid":620,"name":"Portal 2","playtime_forever":320},
                {"appid":440,"name":"Team Fortress 2","playtime_forever":0}]}}"#,
        )
        .unwrap();
        let games = parse_owned_games(&value);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].appid, 620);
        assert_eq!(games[0].playtime_minutes, 320);
    }

    #[test]
    fn crucial_parses_last_played_and_achievement_flag() {
        let value: Value = serde_json::from_str(
            r#"{"response":{"games":[
                {"appid":620,"name":"Portal 2","playtime_forever":320,"rtime_last_played":1699999999,"has_community_visible_stats":true},
                {"appid":228980,"name":"Steamworks","playtime_forever":0,"rtime_last_played":0,"has_community_visible_stats":false},
                {"appid":1,"name":"Unknown","playtime_forever":0}]}}"#,
        )
        .unwrap();
        let games = parse_owned_games(&value);
        assert_eq!(games[0].last_played, 1_699_999_999);
        assert_eq!(games[0].has_achievements, Some(true));
        assert_eq!(games[1].has_achievements, Some(false));
        assert_eq!(games[2].has_achievements, None);
        assert_eq!(games[2].last_played, 0);
    }

    #[test]
    fn crucial_schema_parses_names_icons_and_percentages_source() {
        let value: Value = serde_json::from_str(
            r#"{"game":{"gameName":"Portal 2","availableGameStats":{"achievements":[
                {"name":"ACH.WIN","displayName":"Winner","hidden":0,"description":"Win it",
                 "icon":"https://cdn.example/a.jpg","icongray":"https://cdn.example/a_gray.jpg"},
                {"name":"ACH.HASHONLY","displayName":"Hashed","description":"","icon":"abc123","icongray":""}]}}}"#,
        )
        .unwrap();
        let schema = parse_schema(&value, 620);
        assert_eq!(schema.len(), 2);
        assert_eq!(schema["ACH.WIN"].display_name, "Winner");
        assert_eq!(schema["ACH.WIN"].icon, "https://cdn.example/a.jpg");
        assert_eq!(
            schema["ACH.HASHONLY"].icon,
            "https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/620/abc123.jpg"
        );
        assert_eq!(schema["ACH.HASHONLY"].icon_gray, "");
    }

    #[test]
    fn crucial_schema_missing_stats_is_empty_not_panic() {
        let value: Value = serde_json::from_str(r#"{"game":{"gameName":"x"}}"#).unwrap();
        assert!(parse_schema(&value, 620).is_empty());
    }

    #[test]
    fn crucial_apply_schema_fills_api_name_fallbacks_and_keeps_notes() {
        let value: Value = serde_json::from_str(
            r#"{"game":{"availableGameStats":{"achievements":[
                {"name":"ACH.WIN","displayName":"Winner","description":"Win it","icon":"i","icongray":"g"}]}}}"#,
        )
        .unwrap();
        let schema = parse_schema(&value, 620);
        let mut achievements = vec![
            Achievement {
                api_name: "ACH.WIN".to_owned(),
                name: "ACH.WIN".to_owned(),
                note: "mine".to_owned(),
                ..Default::default()
            },
            Achievement {
                api_name: "OTHER".to_owned(),
                name: "Keep me".to_owned(),
                ..Default::default()
            },
        ];
        apply_schema(&mut achievements, &schema);
        assert_eq!(achievements[0].name, "Winner");
        assert_eq!(achievements[0].description, "Win it");
        assert_eq!(
            achievements[0].icon,
            "https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/620/i.jpg"
        );
        assert_eq!(
            achievements[0].icon_gray,
            "https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps/620/g.jpg"
        );
        assert_eq!(achievements[0].note, "mine");
        assert_eq!(achievements[1].name, "Keep me");
        assert!(achievements[1].icon.is_empty());
    }

    #[test]
    fn crucial_schema_without_key_is_error() {
        assert!(schema("", 620).is_err());
    }

    #[test]
    fn crucial_owned_games_payload_without_games_is_empty() {
        let value: Value = serde_json::from_str(r#"{"response":{}}"#).unwrap();
        assert!(parse_owned_games(&value).is_empty());
    }

    #[test]
    fn crucial_parses_achievements_with_unlock_time() {
        let value: Value = serde_json::from_str(
            r#"{"playerstats":{"success":true,"achievements":[
                {"apiname":"WIN","achieved":1,"unlocktime":1699999999,"name":"Winner","description":"Win"},
                {"apiname":"LOSE","achieved":0,"unlocktime":0,"name":"Loser","description":"Lose"}]}}"#,
        )
        .unwrap();
        let achievements = parse_achievements(&value).unwrap();
        assert_eq!(achievements.len(), 2);
        assert!(achievements[0].unlocked);
        assert_eq!(achievements[0].unlock_time, Some(1_699_999_999));
        assert!(!achievements[1].unlocked);
        assert_eq!(achievements[1].unlock_time, None);
    }

    #[test]
    fn crucial_achievements_surfaces_steam_error() {
        let value: Value = serde_json::from_str(
            r#"{"playerstats":{"success":false,"error":"Requested app has no stats"}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_achievements(&value).unwrap_err(),
            "Requested app has no stats"
        );
    }

    #[test]
    fn crucial_achievements_without_schema_is_error() {
        let value: Value = serde_json::from_str(r#"{"playerstats":{"success":true}}"#).unwrap();
        assert!(parse_achievements(&value).is_err());
    }

    #[test]
    fn crucial_parses_global_percentages() {
        let value: Value = serde_json::from_str(
            r#"{"achievementpercentages":{"achievements":[{"name":"WIN","percent":12.5}]}}"#,
        )
        .unwrap();
        let percentages = parse_global_percentages(&value);
        assert_eq!(percentages["WIN"], 12.5);
    }

    #[test]
    fn crucial_vanity_resolution_requires_success() {
        let ok: Value =
            serde_json::from_str(r#"{"response":{"success":1,"steamid":"76561199105464144"}}"#)
                .unwrap();
        assert_eq!(parse_vanity(&ok).as_deref(), Some("76561199105464144"));
        let fail: Value = serde_json::from_str(r#"{"response":{"success":42}}"#).unwrap();
        assert_eq!(parse_vanity(&fail), None);
    }

    #[test]
    fn crucial_apply_installed_marks_only_matching_appids() {
        let mut games = crate::steam::games_from_installed_only(&[]);
        games.push(Game {
            appid: 620,
            name: "Portal 2".to_owned(),
            ..Default::default()
        });
        games.push(Game {
            appid: 440,
            name: "TF2".to_owned(),
            ..Default::default()
        });
        apply_installed(&mut games, &[(620, "Portal 2".to_owned())]);
        assert!(games.iter().find(|g| g.appid == 620).unwrap().installed);
        assert!(!games.iter().find(|g| g.appid == 440).unwrap().installed);
    }

    #[test]
    fn crucial_installed_only_creates_installed_games() {
        let games = games_from_installed_only(&[(620, "Portal 2".to_owned())]);
        assert_eq!(games.len(), 1);
        assert!(games[0].installed);
    }
}
