# Developer notes

Human-facing notes for `achievement-hunter`. The `.rs` sources carry no
comments; this file and `AI-COMMENTS.md` are the only place explanations live.

## What this is

A single-binary egui/eframe desktop app for Steam achievement hunting. It
shows the user's Steam library, pulls achievement data, aggregates guides from
HowLongToBeat and SteamHunters, caches cover art from the Steam CDN, and lets
the user annotate achievements, reorder them, and draw a flowgraph of them.

## Layout

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | eframe launcher, window size 1280x820. |
| `src/lib.rs` | Module list; `test_env_lock()` serialises env-mutating tests. |
| `src/app.rs` | The entire GUI: `App`, tabs (Games/Flow/Sort/Settings), background threads, image widgets. |
| `src/model.rs` | Plain data: `Achievement`, `GuideInfo`, `Game`, `FlowNode`, `FlowGraph`, `Tab`, filter/sort/move helpers. |
| `src/store.rs` | Loads/saves the persisted documents; atomic writes; triggers backups; merges refreshed Steam data into existing records. |
| `src/settings.rs` | `Settings`, `Theme` (Tokyo Night default), `FontSettings`, and the `apply(ctx)` entry point that pushes theme+fonts into egui. |
| `src/paths.rs` | All filesystem locations (data/art/backup dirs, settings/library/flow/sort paths) plus Steam root discovery and art URLs. |
| `src/backup.rs` | Timestamped backups with ring pruning. |
| `src/http.rs` | One shared blocking `reqwest` client, query encoding, JSON GET/POST, and `.part`-swap downloads. |
| `src/steam.rs` | Steam Web API request builders + parsers (owned games, achievements, global percentages, vanity URL). |
| `src/vdf.rs` | Valve Data Format parsing for `libraryfolders.vdf`, `appmanifest_*.acf`, and `loginusers.vdf`. |
| `src/providers.rs` | Third-party scrapers: HowLongToBeat search, SteamHunters HTML, SteamGridDB autocomplete/grids. |

## Invariants

- **One writer per file.** All persistence goes through `Store` (settings,
  library, flow, sort). Do not write those paths anywhere else.
- **Atomic writes.** Every save writes a temp file then renames. Never write
  in place; a crash mid-write must not leave a truncated document.
- **Backup before overwrite.** `Store::save` calls `backup::backup_file` first.
  The keep-count is user-configurable (`backups_kept`).
- **Network never on the UI thread.** `app.rs` spawns `std::thread`s and
  reports results over `mpsc`; each result calls `ctx.request_repaint()`.
  There is no async runtime in this project.
- **Data dir override.** `ACHIEVEMENT_HUNTER_DATA` forces the data directory.
  Tests and headless runs rely on it; never hardcode paths.
- **Comments stay out of `.rs`.** Enforced by convention and by review.

## Environment / config

- Credentials: `STEAM_API_KEY` and `STEAM_ID` (or `STEAM_ID64`) env vars, or
  the `steam_api_key` / `steam_id` fields in `settings.toml`. Env wins on
  first launch via `Settings::merge_env`; after that the stored value is used.
- Offline behaviour: with no key, the library is populated from the local
  Steam install scan (`vdf.rs`) and art falls back to the keyless Steam CDN
  (`library_600x900.jpg`). Guides degrade to plain links.
- Steam roots probed: `~/.local/share/Steam`, `~/.steam/steam`,
  `~/.var/app/com.valvesoftware.Steam/...`, plus `libraryfolders.vdf` entries.

## Persistence format

- `settings.toml` — TOML (`paths::settings_path`).
- `library.json`, `flow.json`, `sort.json` — JSON (`paths::library_path` etc.).
- Backups land in `paths::backup_dir()` as `<stem>.<utc-stamp>.<ext>`.

## Testing

- `cargo test --lib` — unit tests, named `crucial_*`.
- The GUI render test (`app.rs::tests`) builds a headless `egui::Context`,
  runs all four tabs plus the detail view through `ctx.run_ui`, and drops the
  output with `drop_without_applying_deltas()`. Forgetting the drop call
  panics with "Dropped TexturesDelta with 1 unapplied deltas".
- Tests that mutate process env must hold `crate::test_env_lock()`.

## Known limits

- Guide scraping depends on third-party HTML/JSON shapes; when those change,
  only `providers.rs` needs updating. Parsers return a link-only guide rather
  than erroring, so a broken scrape degrades instead of failing the app.
- Flowgraph and sort are per-install, not per-user-profile.
- No screenshot-based visual test; CI runs the headless render test instead.
