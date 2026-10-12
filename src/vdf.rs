use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '{' {
            out.push(Token::Open);
            i += 1;
        } else if c == '}' {
            out.push(Token::Close);
            i += 1;
        } else if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    s.push(match chars[i] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    s.push(chars[i]);
                }
                i += 1;
            }
            i += 1;
            out.push(Token::Str(s));
        } else {
            let mut s = String::new();
            while i < chars.len()
                && !chars[i].is_whitespace()
                && chars[i] != '{'
                && chars[i] != '}'
                && chars[i] != '"'
            {
                s.push(chars[i]);
                i += 1;
            }
            out.push(Token::Str(s));
        }
    }
    out
}

fn parse_object(tokens: &[Token], pos: &mut usize) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Close => {
                *pos += 1;
                break;
            }
            Token::Open => *pos += 1,
            Token::Str(key) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::Open) => {
                        *pos += 1;
                        map.insert(key, parse_object(tokens, pos));
                    }
                    Some(Token::Str(value)) => {
                        map.insert(key, serde_json::Value::String(value.clone()));
                        *pos += 1;
                    }
                    _ => {
                        map.insert(key, serde_json::Value::Null);
                    }
                }
            }
        }
    }
    serde_json::Value::Object(map)
}

pub fn parse_vdf(text: &str) -> serde_json::Value {
    let tokens = tokenize(text);
    let mut pos = 0;
    parse_object(&tokens, &mut pos)
}

pub fn read_vdf(path: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(path).ok().map(|t| parse_vdf(&t))
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(|s| s.to_owned())
}

pub fn library_folder_paths(root: &Path) -> Vec<PathBuf> {
    let vdf =
        read_vdf(&root.join("steamapps/libraryfolders.vdf")).unwrap_or(serde_json::Value::Null);
    let mut paths: Vec<PathBuf> = vdf
        .get("libraryfolders")
        .and_then(|v| v.as_object())
        .map(|libs| {
            libs.values()
                .filter_map(|lib| string_field(lib, "path"))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    if !paths.contains(&root.to_path_buf()) && crate::paths::is_steam_library(root) {
        paths.insert(0, root.to_path_buf());
    }
    paths.retain(|p| crate::paths::is_steam_library(p));
    paths
}

pub fn parse_appmanifest(text: &str) -> Option<(u32, String)> {
    let vdf = parse_vdf(text);
    let app = vdf.get("AppState")?;
    let appid = string_field(app, "appid")?.parse::<u32>().ok()?;
    let name = string_field(app, "name").unwrap_or_default();
    Some((appid, name))
}

pub fn parse_installed_apps(root: &Path) -> Vec<(u32, String)> {
    let mut apps = Vec::new();
    for lib in library_folder_paths(root) {
        let steamapps = lib.join("steamapps");
        let entries = match std::fs::read_dir(&steamapps) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Some(app) = parse_appmanifest(&text)
            {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|(appid, _)| *appid);
    apps.dedup_by_key(|(appid, _)| *appid);
    apps
}

pub fn parse_most_recent_steam_id(root: &Path) -> Option<String> {
    let vdf = read_vdf(&root.join("config/loginusers.vdf"))?;
    let users = vdf.get("users")?.as_object()?;
    let mut best: Option<(String, bool)> = None;
    for (steam_id, data) in users {
        let most_recent = data
            .get("MostRecent")
            .and_then(|v| v.as_str())
            .map(|v| v == "1")
            .unwrap_or(false);
        if best
            .as_ref()
            .is_none_or(|(_, recent)| most_recent && !recent)
        {
            best = Some((steam_id.clone(), most_recent));
        }
    }
    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/user/.local/share/Steam"
		"label"		""
	}
	"1"
	{
		"path"		"/home/user/games/SteamLibrary"
	}
}
"#;

    #[test]
    fn crucial_parses_nested_vdf() {
        let v = parse_vdf(LIBRARY);
        assert_eq!(
            v["libraryfolders"]["1"]["path"],
            serde_json::Value::String("/home/user/games/SteamLibrary".into())
        );
    }

    #[test]
    fn crucial_vdf_skips_comments() {
        let v = parse_vdf("// top\n\"a\" \"b\" // trailing\n");
        assert_eq!(v["a"], serde_json::Value::String("b".into()));
    }

    #[test]
    fn crucial_vdf_handles_escaped_quotes() {
        let v = parse_vdf(r#""name" "Say \"hi\"""#);
        assert_eq!(v["name"], serde_json::Value::String("Say \"hi\"".into()));
    }

    #[test]
    fn crucial_parses_appmanifest() {
        let text = "AppState\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal 2\"\n}";
        assert_eq!(parse_appmanifest(text), Some((620, "Portal 2".to_owned())));
    }

    #[test]
    fn crucial_appmanifest_rejects_garbage() {
        assert_eq!(parse_appmanifest("not vdf"), None);
        assert_eq!(parse_appmanifest("AppState { \"appid\" \"abc\" }"), None);
    }

    #[test]
    fn crucial_library_folder_paths_uses_file() {
        let dir = std::env::temp_dir().join(format!("ah-vdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("steamapps")).unwrap();
        std::fs::create_dir_all(dir.join("lib2/steamapps")).unwrap();
        let text = format!(
            "{} {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            "\"libraryfolders\"",
            dir.display(),
            dir.join("lib2").display()
        );
        std::fs::write(dir.join("steamapps/libraryfolders.vdf"), text).unwrap();
        let paths = library_folder_paths(&dir);
        assert_eq!(paths.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_installed_apps_scans_manifests() {
        let dir = std::env::temp_dir().join(format!("ah-vdf-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("steamapps")).unwrap();
        std::fs::write(
            dir.join("steamapps/appmanifest_620.acf"),
            "AppState { \"appid\" \"620\" \"name\" \"Portal 2\" }",
        )
        .unwrap();
        std::fs::write(
            dir.join("steamapps/appmanifest_620b.txt"),
            "AppState { \"appid\" \"621\" \"name\" \"Ignored\" }",
        )
        .unwrap();
        assert_eq!(
            parse_installed_apps(&dir),
            vec![(620, "Portal 2".to_owned())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_most_recent_steam_id_prefers_flagged_user() {
        let dir = std::env::temp_dir().join(format!("ah-vdf-login-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(
            dir.join("config/loginusers.vdf"),
            "\"users\" { \"111\" { \"MostRecent\" \"0\" } \"222\" { \"MostRecent\" \"1\" } }",
        )
        .unwrap();
        assert_eq!(parse_most_recent_steam_id(&dir).as_deref(), Some("222"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
