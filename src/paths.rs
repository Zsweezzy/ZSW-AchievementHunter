use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("ACHIEVEMENT_HUNTER_DATA")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("achievement-hunter")
}

pub fn home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

pub fn art_dir() -> PathBuf {
    cache_dir().join("art")
}

pub fn icons_dir() -> PathBuf {
    cache_dir().join("icons")
}

pub fn backup_dir() -> PathBuf {
    data_dir().join("backups")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.toml")
}

pub fn library_path() -> PathBuf {
    data_dir().join("library.json")
}

pub fn flow_path() -> PathBuf {
    data_dir().join("flow.json")
}

pub fn guides_dir() -> PathBuf {
    data_dir().join("guides")
}

pub fn psnprofiles_guides_dir() -> PathBuf {
    guides_dir().join("psnprofiles")
}

pub fn sort_path() -> PathBuf {
    data_dir().join("sort.json")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(art_dir())?;
    std::fs::create_dir_all(icons_dir())?;
    std::fs::create_dir_all(backup_dir())?;
    std::fs::create_dir_all(psnprofiles_guides_dir())?;
    Ok(())
}

pub fn steam_root() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("STEAM_ROOT")
        && !explicit.is_empty()
    {
        return Some(PathBuf::from(explicit));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = home() {
        candidates.push(home.join(".local/share/Steam"));
        candidates.push(home.join(".steam/steam"));
        candidates.push(home.join(".steam/root"));
        candidates.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

pub fn is_steam_library(path: &Path) -> bool {
    path.join("steamapps").is_dir()
}

pub fn art_path(appid: u32) -> PathBuf {
    art_dir().join(format!("{appid}.jpg"))
}

pub fn icon_path(appid: u32, api_name: &str, gray: bool) -> PathBuf {
    let safe: String = api_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    icons_dir().join(format!(
        "{appid}_{safe}{}.jpg",
        if gray { "_gray" } else { "" }
    ))
}

pub fn art_url(appid: u32) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/library_600x900.jpg")
}

pub fn header_url(appid: u32) -> String {
    format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/header.jpg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crucial_data_dir_honours_explicit_override() {
        let _guard = crate::test_env_lock();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", "/tmp/ah-test") };
        assert_eq!(data_dir(), PathBuf::from("/tmp/ah-test"));
        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
    }

    #[test]
    fn art_path_is_under_art_dir() {
        assert!(art_path(123).ends_with("123.jpg"));
        assert!(art_url(620).contains("/620/"));
    }

    #[test]
    fn crucial_icon_path_never_escapes_the_icons_dir() {
        let path = icon_path(620, "../../etc/passwd", false);
        assert_eq!(path.parent().unwrap(), icons_dir());
        assert!(
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(std::path::MAIN_SEPARATOR)
        );
        assert!(
            icon_path(620, "A_B", true)
                .to_string_lossy()
                .ends_with("620_A_B_gray.jpg")
        );
    }
}
