use crate::model::{FlowGraph, Game};
use crate::paths;
use crate::settings::Settings;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::io;
use std::path::Path;

pub struct Store {
    pub settings: Settings,
    pub games: Vec<Game>,
    pub flow: HashMap<u32, FlowGraph>,
    pub sort: HashMap<u32, Vec<String>>,
}

impl Store {
    pub fn load() -> Self {
        let _ = paths::ensure_dirs();
        let settings = read_toml(&paths::settings_path()).unwrap_or_default();
        Self {
            settings,
            games: read_json(&paths::library_path()).unwrap_or_default(),
            flow: read_json(&paths::flow_path()).unwrap_or_default(),
            sort: read_json(&paths::sort_path()).unwrap_or_default(),
        }
    }

    pub fn save_settings(&self) -> io::Result<()> {
        write_atomic(
            &paths::settings_path(),
            &self.settings,
            self.settings.backups_kept,
            |v| toml::to_string_pretty(v).map_err(io::Error::other),
        )
    }

    pub fn save_library(&self) -> io::Result<()> {
        write_atomic(
            &paths::library_path(),
            &self.games,
            self.settings.backups_kept,
            |v| serde_json::to_string_pretty(v).map_err(io::Error::other),
        )
    }

    pub fn save_flow(&self) -> io::Result<()> {
        write_atomic(
            &paths::flow_path(),
            &self.flow,
            self.settings.backups_kept,
            |v| serde_json::to_string_pretty(v).map_err(io::Error::other),
        )
    }

    pub fn save_sort(&self) -> io::Result<()> {
        write_atomic(
            &paths::sort_path(),
            &self.sort,
            self.settings.backups_kept,
            |v| serde_json::to_string_pretty(v).map_err(io::Error::other),
        )
    }

    pub fn save_all(&self) -> io::Result<()> {
        self.save_settings()?;
        self.save_library()?;
        self.save_flow()?;
        self.save_sort()?;
        Ok(())
    }

    pub fn game(&self, appid: u32) -> Option<&Game> {
        self.games.iter().find(|g| g.appid == appid)
    }

    pub fn game_mut(&mut self, appid: u32) -> Option<&mut Game> {
        self.games.iter_mut().find(|g| g.appid == appid)
    }

    pub fn replace_games(&mut self, fresh: Vec<Game>) {
        for game in fresh {
            match self.game_mut(game.appid) {
                Some(existing) => {
                    let art = std::mem::take(&mut existing.art);
                    let guides = std::mem::take(&mut existing.guides);
                    let old_achievements = std::mem::take(&mut existing.achievements);
                    let notes: HashMap<String, String> = old_achievements
                        .into_iter()
                        .filter(|a| !a.note.is_empty())
                        .map(|a| (a.api_name, a.note))
                        .collect();
                    *existing = game;
                    existing.art = art;
                    existing.guides = guides;
                    for achievement in &mut existing.achievements {
                        if let Some(note) = notes.get(&achievement.api_name) {
                            achievement.note = note.clone();
                        }
                    }
                }
                None => self.games.push(game),
            }
        }
        crate::model::sort_games(&mut self.games, self.settings.sort_mode);
    }
}

fn read_toml<T: DeserializeOwned + Default>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_atomic<T, F>(path: &Path, value: &T, keep: usize, encode: F) -> io::Result<()>
where
    T: ?Sized,
    F: FnOnce(&T) -> io::Result<String>,
{
    crate::backup::backup_file(path, keep)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let encoded = encode(value)?;
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("dat")
    ));
    std::fs::write(&tmp, encoded)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Achievement;
    use std::path::PathBuf;

    fn game(appid: u32, playtime: u64) -> Game {
        Game {
            appid,
            name: format!("Game {appid}"),
            playtime_minutes: playtime,
            ..Default::default()
        }
    }

    fn temp_store(tag: &str) -> (std::sync::MutexGuard<'static, ()>, PathBuf, Store) {
        let guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };
        let store = Store::load();
        (guard, dir, store)
    }

    #[test]
    fn crucial_save_then_load_roundtrips_every_store() {
        let (_guard, dir, mut store) = temp_store("roundtrip");
        store.games = vec![game(620, 100)];
        store.sort.insert(620, vec!["a".to_owned()]);
        store.flow.insert(
            620,
            FlowGraph {
                nodes: vec![crate::model::FlowNode {
                    api_name: "a".to_owned(),
                    x: 1.0,
                    y: 2.0,
                }],
                edges: vec![(0, 0)],
            },
        );
        store.settings.steam_id = "42".to_owned();
        store.save_all().unwrap();

        let reloaded = Store::load();
        assert_eq!(reloaded.games.len(), 1);
        assert_eq!(reloaded.settings.steam_id, "42");
        assert_eq!(reloaded.sort[&620], vec!["a".to_owned()]);
        assert_eq!(reloaded.flow[&620].nodes[0].x, 1.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_refresh_preserves_notes_art_and_guides() {
        let (_guard, dir, mut store) = temp_store("merge");
        let mut existing = game(620, 100);
        existing.art = "/tmp/a.jpg".to_owned();
        existing.guides.push(crate::model::GuideInfo {
            source: "hltb".to_owned(),
            ..Default::default()
        });
        existing.achievements.push(Achievement {
            api_name: "win".to_owned(),
            note: "my note".to_owned(),
            ..Default::default()
        });
        store.games = vec![existing];

        let mut fresh = game(620, 200);
        fresh.achievements.push(Achievement {
            api_name: "win".to_owned(),
            unlocked: true,
            ..Default::default()
        });
        store.replace_games(vec![fresh]);

        let merged = store.game(620).unwrap();
        assert_eq!(merged.playtime_minutes, 200);
        assert_eq!(merged.art, "/tmp/a.jpg");
        assert_eq!(merged.guides.len(), 1);
        assert_eq!(merged.achievements[0].note, "my note");
        assert!(merged.achievements[0].unlocked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_refresh_keeps_unmatched_games() {
        let (_guard, dir, mut store) = temp_store("unmatched");
        store.games = vec![game(1, 5)];
        store.replace_games(vec![game(2, 9)]);
        assert_eq!(store.games.len(), 2);
        assert_eq!(store.games[0].appid, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_replace_games_sorts_by_playtime_desc() {
        let (_guard, dir, mut store) = temp_store("sort");
        store.replace_games(vec![game(1, 5), game(2, 900), game(3, 60)]);
        assert_eq!(
            store.games.iter().map(|g| g.appid).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_corrupt_store_loads_defaults() {
        let (_guard, dir, _store) = temp_store("corrupt");
        std::fs::write(dir.join("settings.toml"), "not = toml = at all").unwrap();
        let store = Store::load();
        assert_eq!(store.settings, Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
