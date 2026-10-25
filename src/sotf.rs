//! Sons Of The Forest save scanner.
//!
//! Turns the save's opaque `consumed.<id>` / clothing ids into item names so the
//! FOODIE and FASHIONISTA guide checklists can be ticked automatically.
//!
//! Save layout: `SaveData.zip` holds one JSON per system, each shaped like
//! `{"Version":..,"Data":{"<System>":"<json string>"}}` — the inner value is a
//! JSON *string* and needs a second parse.

use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const APPID: u32 = 1326470;

/// item id -> display name, lifted out of the game's `resources.assets`.
const ITEMS: &str = include_str!("data/sotf_items.json");

/// guide wording -> item name in the table, where the guide paraphrases the game.
const ALIASES: &[(&str, &str)] = &[
    ("human arm", "severed arm"),
    ("human leg", "severed leg"),
    // The guide copies the in-game spelling, the item table has its own typos.
    ("camouflage suit", "camoflage suit"),
    ("gold armor", "golden armour"),
];

/// Every food and clothing item the newest save says the player has.
pub fn scan() -> Result<BTreeSet<String>, String> {
    let path = newest_save().ok_or("no Sons Of The Forest save found under Steam")?;
    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let table: HashMap<u32, String> = serde_json::from_str(ITEMS).map_err(|e| e.to_string())?;

    let mut found = BTreeSet::new();
    // consumed.<id> covers food; isNewItemInInventory_<id> covers anything ever picked up.
    let state = system(&mut zip, "PlayerStateSaveData.json", "PlayerState")?;
    for name in entry_names(&state) {
        for prefix in ["consumed.", "isNewItemInInventory_"] {
            if let Some(id) = id_after(&name, prefix) {
                if let Some(item) = table.get(&id) {
                    found.insert(item_key(item));
                }
            }
        }
    }
    // The wardrobe block is empty in most saves, so parse it defensively rather
    // than trusting a shape. `isNewItemInInventory_` above already covers owned clothing.
    if let Ok(clothing) = system(
        &mut zip,
        "PlayerClothingSystemSaveData.json",
        "PlayerClothingSystem",
    ) {
        let mut ids = Vec::new();
        collect_clothing_ids(&clothing, &mut ids);
        for id in ids {
            if let Some(item) = table.get(&id) {
                found.insert(item_key(item));
            }
        }
    }
    Ok(found)
}

/// Whether a guide checklist line is answered by an item the save contains.
pub fn line_done(line: &str, found: &BTreeSet<String>) -> bool {
    // The guide annotates some lines with a location: "WET SUIT(Shovel cave)".
    let head = line.split(['(', '[']).next().unwrap_or(line);
    item_done(line, found) || (head != line && item_done(head, found))
}

fn item_done(line: &str, found: &BTreeSet<String>) -> bool {
    let key = item_key(line);
    if found.contains(&key) {
        return true;
    }
    if let Some((_, target)) = ALIASES.iter().find(|(guide, _)| item_key(guide) == key) {
        return found.contains(&item_key(target));
    }
    // A one-word guide item ("oyster") is spelled out in the game name ("king oyster").
    line.split_whitespace().count() == 1
        && found
            .iter()
            .any(|f| f.ends_with(&key) || f.starts_with(&key))
}

/// Loose key so "MRE Packs" (game) matches "MRE pack" (guide), and "WET SUIT"
/// (guide) matches "Wetsuit" (game) — spaces and punctuation are not reliable.
pub fn item_key(name: &str) -> String {
    let joined: String = name
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    match joined.strip_suffix('s') {
        Some(stem) => stem.to_owned(),
        None => joined,
    }
}

type Zip = zip::ZipArchive<std::fs::File>;

/// Read one system file and parse both JSON layers.
fn system(zip: &mut Zip, file: &str, key: &str) -> Result<Value, String> {
    let mut entry = zip.by_name(file).map_err(|e| format!("{file}: {e}"))?;
    let mut raw = String::new();
    use std::io::Read;
    entry.read_to_string(&mut raw).map_err(|e| e.to_string())?;
    let outer: Value = serde_json::from_str(&raw).map_err(|e| format!("{file}: {e}"))?;
    let inner = outer
        .get("Data")
        .and_then(|d| d.get(key))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{file}: no Data.{key}"))?;
    serde_json::from_str(inner).map_err(|e| format!("{file}: {e}"))
}

/// `PlayerState._entries` is a list of `{"Name": .., ..}` records.
fn entry_names(state: &Value) -> Vec<String> {
    state
        .get("_entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.get("Name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn id_after(name: &str, prefix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?
        .trim_start_matches('_')
        .parse()
        .ok()
}

/// Any integer under a key that smells like a clothing id. The real shape varies
/// by game version, so this is deliberately loose. `PlayerClothingSystem` keeps
/// the worn pieces as `"Clothing": [499, 444]`, which a plain value check misses.
fn collect_clothing_ids(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if key.to_lowercase().contains("loth") {
                    collect_numbers(val, out);
                } else if key.contains("Id")
                    && let Some(n) = val.as_u64()
                {
                    out.push(n as u32);
                }
                collect_clothing_ids(val, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_clothing_ids(i, out)),
        _ => {}
    }
}

fn collect_numbers(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Number(n) => {
            if let Some(n) = n.as_u64() {
                out.push(n as u32);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_numbers(i, out)),
        _ => {}
    }
}

fn newest_save() -> Option<PathBuf> {
    let root = crate::paths::steam_root()?;
    crate::vdf::library_folder_paths(&root)
        .into_iter()
        .filter_map(|library| newest_in(&library))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// `compatdata/<appid>/pfx/.../Saves/<user>/<mode>/<slot>/SaveData.zip`, newest by mtime.
fn newest_in(library: &Path) -> Option<(SystemTime, PathBuf)> {
    let saves = library
        .join("steamapps/compatdata")
        .join(APPID.to_string())
        .join("pfx/drive_c/users/steamuser/AppData/LocalLow/Endnight/SonsOfTheForest/Saves");
    let mut best: Option<(SystemTime, PathBuf)> = None;
    // `Temp` sits beside the real save folders, so a missing or non-matching
    // directory must be skipped, not abort the walk.
    let users = std::fs::read_dir(&saves)
        .ok()
        .into_iter()
        .flatten()
        .flatten();
    for user in users {
        for mode in ["Multiplayer", "Singleplayer"] {
            let slots = std::fs::read_dir(user.path().join(mode))
                .ok()
                .into_iter()
                .flatten()
                .flatten();
            for slot in slots {
                let path = slot.path().join("SaveData.zip");
                let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                    continue;
                };
                if best.as_ref().is_none_or(|(old, _)| modified > *old) {
                    best = Some((modified, path));
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_game_and_guide_wording() {
        assert_eq!(item_key("MRE Packs"), "mrepack");
        assert_eq!(item_key("Energy Drinks"), "energydrink");
        assert_eq!(item_key("Crunchie Wunchies"), "crunchiewunchie");
        assert_eq!(item_key("Devil's Club"), "devilsclub");
        assert_eq!(item_key("Severed Arm"), "severedarm");
        // The guide writes "WET SUIT", the game item is "Wetsuit".
        assert_eq!(item_key("WET SUIT"), item_key("Wetsuit"));
        assert_eq!(item_key("SPACE SUIT"), item_key("SpaceSuit"));
    }

    #[test]
    fn guide_lines_resolve_against_the_food_the_save_reports() {
        let found: BTreeSet<String> = [
            "crunchie wunchie",
            "energy drink",
            "energy bar",
            "med",
            "mre pack",
            "canned food",
            "ramen noodle",
            "fish",
            "meat",
            "king oyster",
            "severed arm",
            "severed leg",
        ]
        .into_iter()
        .map(item_key)
        .collect();
        for done in [
            "crunchie wunchies",
            "energy drink",
            "energy bar",
            "meds",
            "MRE pack",
            "canned food",
            "ramen noodles",
            "fish",
            "meat",
            "oyster",
            "human arm",
            "human leg",
        ] {
            assert!(line_done(done, &found), "{done:?} should be ticked");
        }
        // never consumed in this save
        for missing in ["bacon bite", "brain bite", "turtle eggs"] {
            assert!(
                !line_done(missing, &found),
                "{missing:?} must stay unticked"
            );
        }
    }

    #[test]
    fn clothing_matches_across_guide_and_game_spelling() {
        let found: BTreeSet<String> = ["Wetsuit", "SpaceSuit", "Camoflage Suit", "Golden Armour"]
            .into_iter()
            .map(item_key)
            .collect();
        for done in ["WET SUIT", "SPACE SUIT", "CAMOUFLAGE SUIT", "GOLD ARMOR"] {
            assert!(line_done(done, &found), "{done:?} should be ticked");
        }
        // The guide appends the location to most clothing lines.
        assert!(line_done("WET SUIT(Shovel cave)", &found));
        assert!(line_done("SPACE SUIT(Katana bunker)", &found));
        assert!(!line_done("TUXEDO(Maintenance A)", &found));
        assert!(!line_done("HOODIE", &found));
    }

    #[test]
    fn wardrobe_list_shape_is_read() {
        let clothing: Value =
            serde_json::from_str(r#"{"Version":"1.0.0","Clothing":[499,444]}"#).unwrap();
        let mut ids = Vec::new();
        collect_clothing_ids(&clothing, &mut ids);
        ids.sort_unstable();
        assert_eq!(ids, vec![444, 499]);
    }

    #[test]
    fn reads_the_real_save_shape_from_a_fixture() {
        let outer: Value = serde_json::from_str(
            r#"{"Version":"0.0.0","Data":{"PlayerState":"{\"Version\":\"1\",\"_entries\":[{\"Name\":\"consumed.00000425\",\"IntValue\":3},{\"Name\":\"isNewItemInInventory_00000490\"}]}"}}"#,
        )
        .unwrap();
        let inner: Value =
            serde_json::from_str(outer["Data"]["PlayerState"].as_str().unwrap()).unwrap();
        let names = entry_names(&inner);
        assert_eq!(id_after(&names[0], "consumed."), Some(425));
        assert_eq!(id_after(&names[1], "isNewItemInInventory_"), Some(490));
    }

    #[test]
    #[ignore = "needs a real Sons Of The Forest install on this machine"]
    fn scans_the_live_save() {
        let found = scan().expect("save found");
        assert!(found.contains("crunchiewunchie"), "{found:?}");
        assert!(found.contains("severedarm"), "{found:?}");
        assert!(found.contains("tacticaljacket"), "{found:?}");
        assert!(line_done("human arm", &found));
        assert!(
            line_done("WET SUIT", &found),
            "wetsuit from the wardrobe list"
        );
        assert!(line_done("oyster", &found));
        assert!(line_done("meds", &found));
        assert!(!line_done("turtle eggs", &found));
        println!("{} items in save", found.len());

        // End to end: real guide slice -> checklist lines -> ticks.
        // Neighbouring achievement names bound each slice, exactly as the app does
        // when it passes every achievement of the game.
        let _guard = crate::test_env_lock();
        let path = crate::paths::data_dir()
            .join("guides/psnprofiles")
            .join(format!("{APPID}.json"));
        let guide: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let markdown = guide["markdown"].as_str().unwrap();
        let names = ["FASHIONISTA", "FOODIE", "THIS CAN'T BE HEALTHY"];
        let achievements: Vec<crate::model::Achievement> = names
            .iter()
            .map(|n| crate::model::Achievement {
                name: (*n).to_owned(),
                ..Default::default()
            })
            .collect();
        let slices = crate::app::guide_slices(markdown, &achievements);

        let food = crate::app::checklist_lines(&slices[1]);
        let ticked: Vec<&str> = food
            .iter()
            .filter(|l| line_done(l, &found))
            .copied()
            .collect();
        println!("FOODIE {}/{} ticked: {ticked:?}", ticked.len(), food.len());
        for want in [
            "MRE pack",
            "canned food",
            "human arm",
            "oyster",
            "fish",
            "meat",
        ] {
            assert!(ticked.contains(&want), "FOODIE missed {want:?}");
        }
        assert!(
            !ticked.contains(&"Pistol"),
            "FOODIE slice leaked past its section"
        );
        assert!(ticked.len() >= 13, "FOODIE matched only {ticked:?}");

        let cloth = crate::app::checklist_lines(&slices[0]);
        let worn: Vec<&str> = cloth
            .iter()
            .filter(|l| line_done(l, &found))
            .copied()
            .collect();
        println!(
            "FASHIONISTA {}/{} ticked: {worn:?}",
            worn.len(),
            cloth.len()
        );
        assert!(
            worn.iter()
                .any(|l| l.eq_ignore_ascii_case("tactical jacket"))
        );
        assert!(
            !worn.contains(&"bacon bite"),
            "FASHIONISTA slice leaked into FOODIE"
        );
    }
}
