use crate::model::{Game, GuideInfo};
use crate::settings::Settings;
use serde_json::Value;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn local_psnprofiles_guide(appid: u32) -> Option<(String, String)> {
    let path = crate::paths::psnprofiles_guides_dir().join(format!("{appid}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let markdown = value
        .get("markdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if markdown.trim().is_empty() {
        None
    } else {
        Some((title, markdown))
    }
}

pub fn psnprofiles_import_path() -> Option<std::path::PathBuf> {
    let data = crate::paths::data_dir().join("psn-guides.json");
    if data.is_file() {
        return Some(data);
    }
    crate::paths::home()
        .map(|home| home.join("Downloads/psn-guides.json"))
        .filter(|path| path.is_file())
}

pub fn local_psnprofiles_url(appid: u32) -> Option<String> {
    let path = crate::paths::psnprofiles_guides_dir().join(format!("{appid}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let url = value.get("url").and_then(|v| v.as_str())?.trim();
    if url.is_empty() {
        return None;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(url.to_owned());
    }
    Some(format!(
        "https://psnprofiles.com/{}",
        url.trim_start_matches('/')
    ))
}

pub fn import_psnprofiles(
    path: &std::path::Path,
    games: &[Game],
) -> Result<(usize, usize), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no results array in collector JSON".to_owned())?;
    let by_name: std::collections::HashMap<String, u32> = games
        .iter()
        .map(|g| (g.name.to_lowercase(), g.appid))
        .collect();
    std::fs::create_dir_all(crate::paths::psnprofiles_guides_dir()).map_err(|e| e.to_string())?;
    let (mut written, mut skipped) = (0usize, 0usize);
    for item in results {
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let markdown = item
            .get("markdown")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let Some(appid) = by_name.get(&title.to_lowercase()).copied() else {
            skipped += 1;
            continue;
        };
        if markdown.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let entry = serde_json::json!({
            "source": "psnprofiles",
            "title": item.get("pickedTitle").and_then(|v| v.as_str()).unwrap_or(title),
            "url": item.get("pickedUrl").and_then(|v| v.as_str()).unwrap_or_default(),
            "markdown": markdown,
        });
        let dest = crate::paths::psnprofiles_guides_dir().join(format!("{appid}.json"));
        std::fs::write(dest, entry.to_string()).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok((written, skipped))
}

pub fn hltb_search_url(name: &str) -> String {
    format!(
        "https://howlongtobeat.com/?q={}",
        crate::http::encode_query(name)
    )
}

pub fn steamhunters_url(appid: u32) -> String {
    format!("https://steamhunters.com/apps/{appid}")
}

pub fn steamhunters_app_url(appid: u32) -> String {
    format!("https://steamhunters.com/api/apps/{appid}")
}

pub fn steamhunters_achievements_url(appid: u32) -> String {
    format!("https://steamhunters.com/api/apps/{appid}/achievements")
}

pub fn fetch_guides(game: &Game, settings: &Settings) -> (Vec<GuideInfo>, Vec<String>) {
    let mut guides = Vec::new();
    let mut errors = Vec::new();
    if settings.guide_steamhunters {
        let (guide, error) = steamhunters(game);
        if let Some(error) = error {
            errors.push(format!("steamhunters: {error}"));
        }
        guides.push(guide);
    }
    if settings.guide_hltb {
        let (guide, error) = howlongtobeat(game);
        if let Some(error) = error {
            errors.push(format!("howlongtobeat: {error}"));
        }
        guides.push(guide);
    }
    (guides, errors)
}

fn link_only(
    source: &str,
    url: String,
    summary: String,
    error: String,
) -> (GuideInfo, Option<String>) {
    (
        GuideInfo {
            source: source.to_owned(),
            url,
            summary,
            fetched_at: now_secs(),
            title: String::new(),
            markdown: String::new(),
            main_hours: None,
            extra_hours: None,
            completionist_hours: None,
            difficulty: String::new(),
        },
        Some(error),
    )
}

fn howlongtobeat(game: &Game) -> (GuideInfo, Option<String>) {
    let url = hltb_search_url(&game.name);
    let terms: Vec<&str> = game
        .name
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();
    let body = serde_json::json!({
        "searchType": "games",
        "searchTerms": terms,
        "size": 8,
        "searchPage": 1,
    });
    let headers = [
        ("Referer", "https://howlongtobeat.com/"),
        ("Origin", "https://howlongtobeat.com"),
        ("Content-Type", "application/json"),
    ];
    match crate::http::post_json("https://howlongtobeat.com/api/search", &body, &headers) {
        Ok(value) => {
            let guide = parse_hltb(&value, &game.name, url);
            if guide.main_hours.is_none() && guide.completionist_hours.is_none() {
                let matches = guide.summary.clone();
                let only = link_only(
                    "howlongtobeat",
                    guide.url,
                    matches,
                    "title not found".to_owned(),
                );
                (only.0, None)
            } else {
                (guide, None)
            }
        }
        Err(error) => (
            link_only(
                "howlongtobeat",
                url,
                format!("HowLongToBeat lookup unavailable ({error}). Open the search page."),
                error,
            )
            .0,
            Some("api unavailable".to_owned()),
        ),
    }
}

pub fn parse_hltb(value: &Value, game_name: &str, url: String) -> GuideInfo {
    let empty = Vec::new();
    let items = value
        .get("data")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let lower = game_name.to_lowercase();
    let chosen = items
        .iter()
        .find(|item| {
            item.get("game_name")
                .and_then(|v| v.as_str())
                .map(|n| n.to_lowercase() == lower)
                .unwrap_or(false)
        })
        .or_else(|| items.first());

    let Some(item) = chosen else {
        return GuideInfo {
            source: "howlongtobeat".to_owned(),
            url,
            summary: format!("No HowLongToBeat entry for \"{game_name}\"."),
            fetched_at: now_secs(),
            title: String::new(),
            markdown: String::new(),
            main_hours: None,
            extra_hours: None,
            completionist_hours: None,
            difficulty: String::new(),
        };
    };

    let match_name = item
        .get("game_name")
        .and_then(|v| v.as_str())
        .unwrap_or(game_name);
    let hours = |key: &str| -> Option<f32> {
        item.get(key)
            .and_then(|v| v.as_f64())
            .filter(|s| *s > 0.0)
            .map(|s| ((s / 3600.0) as f32 * 10.0).round() / 10.0)
    };
    let main = hours("comp_main");
    let extra = hours("comp_plus");
    let completionist = hours("comp_100");
    let summary = format!(
        "{match_name}: main {}, main+extra {}, completionist {}",
        fmt_hours(main),
        fmt_hours(extra),
        fmt_hours(completionist)
    );
    GuideInfo {
        source: "howlongtobeat".to_owned(),
        url,
        summary,
        main_hours: main,
        extra_hours: extra,
        completionist_hours: completionist,
        difficulty: String::new(),
        fetched_at: now_secs(),
        title: String::new(),
        markdown: String::new(),
    }
}

fn fmt_hours(hours: Option<f32>) -> String {
    match hours {
        Some(h) => format!("{h} h"),
        None => "n/a".to_owned(),
    }
}

fn steamhunters(game: &Game) -> (GuideInfo, Option<String>) {
    let url = steamhunters_url(game.appid);
    match crate::http::get_json_with_headers(
        &steamhunters_app_url(game.appid),
        &[("Accept", "application/json")],
    ) {
        Ok(value) => {
            let guide = parse_steamhunters_app(&value, &game.name, url);
            let error = if guide.summary.is_empty() {
                Some("no stats parsed".to_owned())
            } else {
                None
            };
            (guide, error)
        }
        Err(error) => (
            link_only(
                "steamhunters",
                url,
                format!("SteamHunters unavailable ({error}). Open the page directly."),
                error,
            )
            .0,
            Some("unreachable".to_owned()),
        ),
    }
}

pub fn parse_steamhunters_app(value: &Value, fallback_name: &str, url: String) -> GuideInfo {
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_name);
    let achievements = value
        .get("achievementCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let completion = value
        .get("medianCompletionTime")
        .and_then(|v| v.as_f64())
        .filter(|m| *m > 0.0)
        .map(|m| ((m / 60.0) as f32 * 10.0).round() / 10.0);
    let perfected = value
        .get("playersPerfectedCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let started = value
        .get("playersStartedCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let points = value.get("points").and_then(|v| v.as_u64()).unwrap_or(0);
    let difficulty = steamhunters_difficulty(perfected, started);
    let mut summary = format!("{name}: {achievements} achievements");
    if let Some(hours) = completion {
        summary.push_str(&format!(" \u{2022} ~{hours} h to complete"));
    }
    if !difficulty.is_empty() {
        summary.push_str(&format!(" \u{2022} est. difficulty {difficulty}"));
    }
    if points > 0 {
        summary.push_str(&format!(" \u{2022} {points} points"));
    }
    GuideInfo {
        source: "steamhunters".to_owned(),
        url,
        summary,
        main_hours: completion,
        completionist_hours: completion,
        difficulty,
        fetched_at: now_secs(),
        title: String::new(),
        markdown: String::new(),
        extra_hours: None,
    }
}

pub fn steamhunters_difficulty(perfected: u64, started: u64) -> String {
    if started == 0 {
        return String::new();
    }
    let rate = perfected as f64 / started as f64 * 100.0;
    let word = match rate {
        r if r >= 30.0 => "easy",
        r if r >= 12.0 => "moderate",
        r if r >= 4.0 => "hard",
        r if r >= 1.0 => "very hard",
        _ => "extreme",
    };
    format!("{word} ({rate:.1}% perfected)")
}

pub fn parse_steamhunters_achievements(value: &Value) -> Vec<(String, String, String, f32)> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let api_name = item.get("apiName").and_then(|v| v.as_str())?.to_owned();
            Some((
                api_name,
                item.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                item.get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                item.get("steamPercentage")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
            ))
        })
        .collect()
}

pub fn steamhunters_achievements(appid: u32) -> Result<Vec<(String, String, String, f32)>, String> {
    let value = crate::http::get_json_with_headers(
        &steamhunters_achievements_url(appid),
        &[("Accept", "application/json")],
    )?;
    Ok(parse_steamhunters_achievements(&value))
}

pub fn steamgriddb_grid(name: &str, key: &str) -> Result<String, String> {
    if key.trim().is_empty() {
        return Err("no steamgriddb api key".to_owned());
    }
    let auth = format!("Bearer {key}");
    let search_url = format!(
        "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
        crate::http::encode_query(name)
    );
    let search =
        crate::http::get_json_with_headers(&search_url, &[("Authorization", auth.as_str())])?;
    let game_id = parse_steamgriddb_search(&search)
        .ok_or_else(|| format!("no steamgriddb match for '{name}'"))?;
    let grids_url = format!(
        "https://www.steamgriddb.com/api/v2/grids/game/{game_id}?dimensions=600x900&types=static"
    );
    let grids =
        crate::http::get_json_with_headers(&grids_url, &[("Authorization", auth.as_str())])?;
    parse_steamgriddb_grids(&grids).ok_or_else(|| "no grid art returned".to_owned())
}

pub fn parse_steamgriddb_search(value: &Value) -> Option<u32> {
    value
        .get("data")?
        .as_array()?
        .first()?
        .get("id")?
        .as_u64()
        .map(|id| id as u32)
}

pub fn parse_steamgriddb_grids(value: &Value) -> Option<String> {
    value
        .get("data")?
        .as_array()?
        .first()?
        .get("url")?
        .as_str()
        .map(|s| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(name: &str) -> Game {
        Game {
            appid: 620,
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn crucial_hltb_selects_exact_name_over_first_hit() {
        let value: Value = serde_json::from_str(
            r#"{"data":[
                {"game_name":"Portal 2 Beta","comp_main":3600,"comp_plus":0,"comp_100":7200},
                {"game_name":"portal 2","comp_main":36000,"comp_plus":54000,"comp_100":90000}]}"#,
        )
        .unwrap();
        let guide = parse_hltb(&value, "Portal 2", "url".to_owned());
        assert_eq!(guide.main_hours, Some(10.0));
        assert_eq!(guide.extra_hours, Some(15.0));
        assert_eq!(guide.completionist_hours, Some(25.0));
    }

    #[test]
    fn crucial_hltb_falls_back_to_first_hit() {
        let value: Value =
            serde_json::from_str(r#"{"data":[{"game_name":"Portal 2","comp_main":36000}]}"#)
                .unwrap();
        let guide = parse_hltb(&value, "Some Other Game", "url".to_owned());
        assert_eq!(guide.main_hours, Some(10.0));
    }

    #[test]
    fn crucial_hltb_zero_seconds_means_unknown() {
        let value: Value = serde_json::from_str(
            r#"{"data":[{"game_name":"Portal 2","comp_main":0,"comp_100":1800}]}"#,
        )
        .unwrap();
        let guide = parse_hltb(&value, "Portal 2", "url".to_owned());
        assert_eq!(guide.main_hours, None);
        assert_eq!(guide.completionist_hours, Some(0.5));
    }

    #[test]
    fn crucial_hltb_empty_payload_is_reported_not_matched() {
        let value: Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        let guide = parse_hltb(&value, "Portal 2", "url".to_owned());
        assert!(guide.summary.contains("No HowLongToBeat"));
        assert_eq!(guide.main_hours, None);
    }

    #[test]
    fn crucial_hltb_always_carries_a_url() {
        assert!(hltb_search_url("Portal 2").ends_with("q=Portal+2"));
        assert_eq!(steamhunters_url(620), "https://steamhunters.com/apps/620");
        assert_eq!(
            steamhunters_app_url(620),
            "https://steamhunters.com/api/apps/620"
        );
    }

    #[test]
    fn crucial_steamhunters_app_summarises_stats() {
        let value: Value = serde_json::from_str(
            r#"{"appId":620,"name":"Portal 2","achievementCount":51,"points":10571,
                "playersPerfectedCount":6684,"playersStartedCount":84197,
                "medianCompletionTime":2011,"fastestCompletionTime":152}"#,
        )
        .unwrap();
        let guide = parse_steamhunters_app(&value, "fallback", "u".to_owned());
        assert_eq!(guide.main_hours, Some(33.5));
        assert_eq!(guide.difficulty, "hard (7.9% perfected)");
        assert!(guide.summary.contains("51 achievements"));
        assert!(guide.summary.contains("~33.5 h to complete"));
        assert!(guide.summary.contains("10571 points"));
        assert_eq!(guide.source, "steamhunters");
    }

    #[test]
    fn crucial_steamhunters_app_without_stats_still_has_a_name() {
        let value: Value = serde_json::from_str(r#"{"appId":620}"#).unwrap();
        let guide = parse_steamhunters_app(&value, "Portal 2", "u".to_owned());
        assert_eq!(guide.main_hours, None);
        assert!(guide.difficulty.is_empty());
        assert!(guide.summary.starts_with("Portal 2:"));
    }

    #[test]
    fn crucial_difficulty_scales_with_perfect_rate() {
        assert_eq!(steamhunters_difficulty(0, 0), "");
        assert!(steamhunters_difficulty(90, 100).starts_with("easy"));
        assert!(steamhunters_difficulty(25, 100).starts_with("moderate"));
        assert!(steamhunters_difficulty(10, 100).starts_with("hard"));
        assert!(steamhunters_difficulty(2, 100).starts_with("very hard"));
        assert!(steamhunters_difficulty(1, 1000).starts_with("extreme"));
    }

    #[test]
    fn crucial_steamhunters_achievements_payload() {
        let value: Value = serde_json::from_str(
            r#"[{"achievementId":1,"apiName":"ACH.SURVIVE","name":"Wake Up Call",
                "description":"Survive","steamPercentage":74.0,"points":21}]"#,
        )
        .unwrap();
        let rows = parse_steamhunters_achievements(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "ACH.SURVIVE");
        assert_eq!(rows[0].1, "Wake Up Call");
        assert_eq!(rows[0].3, 74.0);
        assert!(parse_steamhunters_achievements(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn crucial_steamgriddb_parses_search_and_grid() {
        let search: Value =
            serde_json::from_str(r#"{"success":true,"data":[{"id":99,"name":"Portal 2"}]}"#)
                .unwrap();
        assert_eq!(parse_steamgriddb_search(&search), Some(99));
        let grids: Value =
            serde_json::from_str(r#"{"success":true,"data":[{"url":"https://x/y.jpg"}]}"#).unwrap();
        assert_eq!(
            parse_steamgriddb_grids(&grids).as_deref(),
            Some("https://x/y.jpg")
        );
    }

    #[test]
    fn crucial_steamgriddb_missing_key_is_error() {
        assert!(steamgriddb_grid("Portal 2", "  ").is_err());
    }

    #[test]
    fn crucial_guides_respect_enabled_providers() {
        let settings = Settings {
            guide_hltb: false,
            guide_steamhunters: false,
            ..Default::default()
        };
        let (guides, errors) = fetch_guides(&game("Portal 2"), &settings);
        assert!(guides.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn crucial_guide_summaries_never_panic_on_odd_names() {
        let value: Value = serde_json::from_str(r#"{"data":[{}]}"#).unwrap();
        let guide = parse_hltb(&value, "", "url".to_owned());
        assert!(guide.summary.contains("main"));
    }
}
