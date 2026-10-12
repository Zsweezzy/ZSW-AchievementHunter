use crate::model::{Game, GuideInfo};
use crate::settings::Settings;
use serde_json::Value;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

pub fn fetch_guides(game: &Game, settings: &Settings) -> (Vec<GuideInfo>, Vec<String>) {
    let mut guides = Vec::new();
    let mut errors = Vec::new();
    if settings.guide_hltb {
        let (guide, error) = howlongtobeat(game);
        if let Some(error) = error {
            errors.push(format!("howlongtobeat: {error}"));
        }
        guides.push(guide);
    }
    if settings.guide_steamhunters {
        let (guide, error) = steamhunters(game);
        if let Some(error) = error {
            errors.push(format!("steamhunters: {error}"));
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
            ..Default::default()
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
            ..Default::default()
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
    match crate::http::get_text(&url) {
        Ok(html) => {
            let guide = parse_steamhunters_html(&html, url);
            let error = if guide.summary.is_empty() {
                Some("page reached but no summary parsed".to_owned())
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

pub fn parse_steamhunters_html(html: &str, url: String) -> GuideInfo {
    let summary = find_meta_description(html)
        .or_else(|| find_title(html))
        .unwrap_or_default();
    GuideInfo {
        source: "steamhunters".to_owned(),
        url,
        summary,
        difficulty: find_difficulty(html).unwrap_or_default(),
        fetched_at: now_secs(),
        ..Default::default()
    }
}

fn attribute(html: &str, key: &str) -> Option<String> {
    let index = html.find(key)?;
    let rest = &html[index + key.len()..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_owned())
}

fn char_window(text: &str, start: usize, end: usize) -> &str {
    let mut low = start.min(text.len());
    while low > 0 && !text.is_char_boundary(low) {
        low -= 1;
    }
    let mut high = end.clamp(low, text.len());
    while high > low && !text.is_char_boundary(high) {
        high -= 1;
    }
    &text[low..high]
}

fn find_meta_description(html: &str) -> Option<String> {
    let index = html.find("name=\"description\"")?;
    let window = char_window(html, index.saturating_sub(400), html.len());
    attribute(window, "content=")
}

fn find_title(html: &str) -> Option<String> {
    let start = html.find("<title>")? + "<title>".len();
    let end = html[start..].find("</title>")? + start;
    Some(html[start..end].trim().to_owned())
}

fn find_difficulty(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let index = lower.find("difficulty")?;
    let window = char_window(html, index, index + 200);
    let digits: String = window
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '/')
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
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
    }

    #[test]
    fn crucial_steamhunters_reads_meta_description() {
        let html = r#"<html><head><meta name="description" content="Portal 2 achievements guide"></head></html>"#;
        let guide = parse_steamhunters_html(html, "u".to_owned());
        assert_eq!(guide.summary, "Portal 2 achievements guide");
        assert_eq!(guide.source, "steamhunters");
    }

    #[test]
    fn crucial_steamhunters_falls_back_to_title() {
        let html = "<html><head><title> Portal 2 - SteamHunters </title></head></html>";
        let guide = parse_steamhunters_html(html, "u".to_owned());
        assert_eq!(guide.summary, "Portal 2 - SteamHunters");
    }

    #[test]
    fn crucial_steamhunters_empty_html_yields_no_summary() {
        let guide = parse_steamhunters_html("<html></html>", "u".to_owned());
        assert!(guide.summary.is_empty());
    }

    #[test]
    fn crucial_steamhunters_survives_multibyte_before_meta() {
        let html = format!(
            "{}<meta name=\"description\" content=\"ok\">",
            "€".repeat(200)
        );
        let guide = parse_steamhunters_html(&html, "u".to_owned());
        assert_eq!(guide.summary, "ok");
    }

    #[test]
    fn crucial_steamhunters_survives_multibyte_before_difficulty() {
        let html = format!("{}difficulty 7/10 {}", "€".repeat(100), "€".repeat(200));
        let guide = parse_steamhunters_html(&html, "u".to_owned());
        assert_eq!(guide.difficulty, "7/10");
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
