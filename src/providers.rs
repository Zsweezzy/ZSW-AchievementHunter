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

/// Turn a copied guide page (HTML source or the browser's plain-text copy)
/// into compact, markdown-ish text: tags, scripts, styles and entities gone,
/// whitespace collapsed. A 1 MB page usually shrinks to a few dozen KB.
pub fn page_to_guide(raw: &str) -> String {
    let text = if looks_like_html(raw) {
        html_to_text(raw)
    } else {
        decode_entities(raw)
    };
    normalize_text(&text)
}

/// Convert `raw` and write it to the per-game file the app already reads
/// (`<data>/guides/psnprofiles/<appid>.json`). Returns (input bytes, file bytes).
pub fn save_guide_page(appid: u32, title: &str, raw: &str) -> Result<(usize, usize), String> {
    let markdown = page_to_guide(raw);
    if markdown.trim().is_empty() {
        return Err("no guide text found in the page".to_owned());
    }
    let entry = serde_json::json!({
        "source": "psnprofiles",
        "title": title,
        "url": "",
        "markdown": markdown,
    });
    let text = entry.to_string();
    let dir = crate::paths::psnprofiles_guides_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{appid}.json")), &text).map_err(|e| e.to_string())?;
    Ok((raw.len(), text.len()))
}

fn looks_like_html(raw: &str) -> bool {
    let head: String = raw.chars().take(4096).collect();
    let head = head.to_ascii_lowercase();
    head.contains("</") || head.contains("<!doctype") || head.contains("<br")
}

fn html_to_text(raw: &str) -> String {
    // Steam guide pages put the actual guide in one container; scoping to it
    // drops the site chrome, table of contents and comments in one step.
    let scoped = extract_div(raw, "class=\"guide subsections\"").unwrap_or_else(|| raw.to_owned());
    let cleaned = strip_element(&scoped, "title");
    let cleaned = strip_element(&cleaned, "script");
    let cleaned = strip_element(&cleaned, "style");
    let cleaned = strip_element(&cleaned, "noscript");
    let cleaned = strip_element(&cleaned, "nav");
    let cleaned = strip_element(&cleaned, "footer");
    let cleaned = strip_comments(&cleaned);
    let mut out = String::with_capacity(cleaned.len());
    let mut rest = cleaned.as_str();
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let name = after[1..gt]
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match name.as_str() {
            "br" | "p" | "div" | "li" | "tr" | "ul" | "ol" | "section" | "article"
            | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.push('\n'),
            "td" | "th" => out.push('\t'),
            _ => {}
        }
        rest = &after[gt + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

/// Return the outer HTML of the first `<div>` whose opening tag contains
/// `marker`, by balancing nested `<div>`/`</div>` tags.
fn extract_div(input: &str, marker: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let at = lower.find(&marker.to_ascii_lowercase())?;
    let start = lower[..at].rfind('<')?;
    let mut depth = 0i32;
    let mut i = start;
    while let Some(offset) = lower[i..].find('<') {
        let pos = i + offset;
        let rest = &lower[pos..];
        let opens = rest.starts_with("<div")
            && rest[4..]
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c == '/' || c.is_whitespace());
        let closes = rest.starts_with("</div")
            && rest[5..]
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c.is_whitespace());
        if opens {
            depth += 1;
        } else if closes {
            depth -= 1;
            if depth == 0 {
                let end = pos + lower[pos..].find('>')? + 1;
                return Some(input[start..end].to_owned());
            }
        }
        i = pos + 1;
    }
    None
}

/// Drop `<element …>…</element>` blocks (case-insensitive).
fn strip_element(input: &str, element: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open = format!("<{element}");
    let close = format!("</{element}");
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while let Some(start) = lower[i..].find(&open) {
        let start = i + start;
        out.push_str(&input[i..start]);
        let Some(close_at) = lower[start..].find(&close) else {
            return out;
        };
        let close_at = start + close_at;
        let Some(end) = lower[close_at..].find('>') else {
            return out;
        };
        i = close_at + end + 1;
    }
    out.push_str(&input[i..]);
    out
}

fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start + 4..].find("-->") else {
            return out;
        };
        rest = &rest[start + 4 + end + 3..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let decoded = tail.find(';').and_then(|semi| {
            if semi > 10 {
                return None;
            }
            let entity = &tail[1..semi];
            let decoded = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                "mdash" => '—',
                "ndash" => '–',
                "hellip" => '…',
                "bull" => '•',
                "middot" => '·',
                "copy" => '©',
                "reg" => '®',
                "trade" => '™',
                "euro" => '€',
                "pound" => '£',
                "yen" => '¥',
                "cent" => '¢',
                "sect" => '§',
                "para" => '¶',
                "deg" => '°',
                "times" => '×',
                "divide" => '÷',
                "laquo" => '«',
                "raquo" => '»',
                "rsquo" => '’',
                "lsquo" => '‘',
                "ldquo" => '“',
                "rdquo" => '”',
                _ => {
                    let number = entity.strip_prefix('#')?;
                    let code = match number.strip_prefix(|c: char| c == 'x' || c == 'X') {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse::<u32>().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((decoded, semi))
        });
        match decoded {
            Some((ch, semi)) => {
                out.push(ch);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn normalize_text(input: &str) -> String {
    let lines: Vec<String> = input
        .replace('\r', "\n")
        .lines()
        .map(|line| {
            let mut buf = String::new();
            let mut space = false;
            for ch in line.chars() {
                if ch == ' ' || ch == '\t' || ch == '\u{a0}' {
                    space = true;
                } else {
                    if space && !buf.is_empty() {
                        buf.push(' ');
                    }
                    space = false;
                    if !ch.is_control() {
                        buf.push(ch);
                    }
                }
            }
            buf
        })
        .collect();
    let start = lines.iter().position(|line| !line.is_empty());
    let end = lines.iter().rposition(|line| !line.is_empty());
    let (Some(start), Some(end)) = (start, end) else {
        return String::new();
    };
    let mut out = String::new();
    let mut blanks = 0;
    for line in &lines[start..=end] {
        if line.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_owned()
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

pub fn extra_guide_links(name: &str, appid: u32) -> Vec<(&'static str, String)> {
    let query = crate::http::encode_query(name);
    vec![
        (
            "Steam Community",
            format!("https://steamcommunity.com/app/{appid}/guides/"),
        ),
        (
            "TrueAchievements",
            format!("https://www.trueachievements.com/searchresults.aspx?searchterm={query}"),
        ),
        ("PowerPyx", format!("https://www.powerpyx.com/?s={query}")),
        ("IGN", format!("https://www.ign.com/search?q={query}")),
        (
            "GameFAQs",
            format!("https://gamefaqs.gamespot.com/search?game={query}"),
        ),
    ]
}

/// One entry in [`GUIDE_SOURCES`]: adding a guide source means writing a
/// `(GuideInfo, Option<String>)` fetch function, adding a `Settings` field, and
/// adding one entry here. See CONTRIBUTING.md.
pub struct GuideSource {
    pub id: &'static str,
    pub label: &'static str,
    pub enabled: fn(&Settings) -> bool,
    pub toggle: fn(&mut Settings) -> &mut bool,
    pub fetch: fn(&Game) -> (GuideInfo, Option<String>),
}

pub const GUIDE_SOURCES: &[GuideSource] = &[
    GuideSource {
        id: "steamhunters",
        label: "SteamHunters",
        enabled: |s| s.guide_steamhunters,
        toggle: |s| &mut s.guide_steamhunters,
        fetch: steamhunters,
    },
    GuideSource {
        id: "howlongtobeat",
        label: "HowLongToBeat",
        enabled: |s| s.guide_hltb,
        toggle: |s| &mut s.guide_hltb,
        fetch: howlongtobeat,
    },
];

pub fn fetch_guides(game: &Game, settings: &Settings) -> (Vec<GuideInfo>, Vec<String>) {
    let mut guides = Vec::new();
    let mut errors = Vec::new();
    for source in GUIDE_SOURCES
        .iter()
        .filter(|source| (source.enabled)(settings))
    {
        let (guide, error) = (source.fetch)(game);
        if let Some(error) = error {
            errors.push(format!("{}: {error}", source.id));
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
    fn crucial_every_guide_source_toggles_its_own_setting() {
        assert!(!GUIDE_SOURCES.is_empty());
        for source in GUIDE_SOURCES {
            let mut settings = Settings::default();
            let before = *((source.toggle)(&mut settings));
            assert_eq!((source.enabled)(&settings), before, "{}", source.id);
            *((source.toggle)(&mut settings)) = !before;
            assert_eq!((source.enabled)(&settings), !before, "{}", source.id);
        }
        let mut settings = Settings::default();
        for source in GUIDE_SOURCES {
            *((source.toggle)(&mut settings)) = false;
        }
        assert!(fetch_guides(&game("Portal 2"), &settings).0.is_empty());
        assert!(
            GUIDE_SOURCES
                .iter()
                .any(|source| (source.enabled)(&Settings::default())),
            "at least one source is on out of the box"
        );
    }

    #[test]
    fn crucial_guide_summaries_never_panic_on_odd_names() {
        let value: Value = serde_json::from_str(r#"{"data":[{}]}"#).unwrap();
        let guide = parse_hltb(&value, "", "url".to_owned());
        assert!(guide.summary.contains("main"));
    }

    #[test]
    fn crucial_extra_guide_links_cover_common_sources() {
        let links = extra_guide_links("\u{14c}kami", 620);
        assert!(links.iter().all(|(_, url)| url.starts_with("https://")));
        assert!(
            links
                .iter()
                .any(|(source, url)| *source == "Steam Community" && url.contains("/app/620/"))
        );
        assert!(links.iter().any(|(_, url)| url.contains("%C5%8Ckami")));
    }

    #[test]
    fn crucial_page_to_guide_strips_html_markup() {
        let html = r#"<!doctype html><html><head>
            <style>.trophy { color: red }</style>
            <script>alert("tracking")</script>
            </head><body>
            <h1>Roadmap</h1><p>First&nbsp;line &amp; more.</p>
            <!-- hidden -->
            <div><br>Second &#39;quoted&#39; line</div>
            </body></html>"#;
        let guide = page_to_guide(html);
        assert!(guide.contains("Roadmap"));
        assert!(guide.contains("First line & more."));
        assert!(guide.contains("Second 'quoted' line"));
        assert!(!guide.contains('<'));
        assert!(!guide.contains("alert"));
        assert!(!guide.contains("color: red"));
        assert!(!guide.contains("hidden"));
    }

    #[test]
    fn crucial_page_to_guide_keeps_plain_text() {
        let text = "   Platinum Trophy\r\n\r\n\r\n  Finish the game.  \r\n";
        assert_eq!(page_to_guide(text), "Platinum Trophy\n\nFinish the game.");
    }

    #[test]
    fn crucial_page_to_guide_decodes_entities() {
        assert_eq!(
            page_to_guide("A &#8212; B &#x2014; C &mdash; D"),
            "A — B — C — D"
        );
        assert_eq!(
            page_to_guide("&lt;tag&gt; &amp; &quot;quote&quot;"),
            "<tag> & \"quote\""
        );
    }

    #[test]
    fn crucial_page_to_guide_keeps_only_the_steam_guide_body() {
        let html = r#"<html><body>
            <div class="responsive_page_menu_ctn">NAVJUNK</div>
            <div class="guide subSections">
                <div class="subSection detailBox" id="1">
                    <div class="subSectionTitle">Trophy One</div>
                    <div class="subSectionDesc">Do the thing.<div>Nested.</div></div>
                </div>
                <div class="subSection detailBox" id="2">
                    <div class="subSectionTitle">Trophy Two</div>
                    <div class="subSectionDesc">Do the other thing.</div>
                </div>
            </div>
            <div class="comments">FOOTJUNK</div>
        </body></html>"#;
        let guide = page_to_guide(html);
        assert!(guide.contains("Trophy One"), "{guide}");
        assert!(guide.contains("Do the thing."));
        assert!(guide.contains("Nested."));
        assert!(guide.contains("Trophy Two"));
        assert!(!guide.contains("NAVJUNK"));
        assert!(!guide.contains("FOOTJUNK"));
    }

    #[test]
    fn crucial_saved_guide_page_is_read_back() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-guide-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        save_guide_page(
            4242,
            "Test Game",
            "<h2>Trophy</h2><p>Do&nbsp;the thing.</p>",
        )
        .unwrap();
        let (title, markdown) = local_psnprofiles_guide(4242).expect("guide not readable");
        assert_eq!(title, "Test Game");
        assert_eq!(markdown, "Trophy\n\nDo the thing.");

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
