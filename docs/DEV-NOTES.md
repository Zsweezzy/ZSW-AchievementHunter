# Developer notes

Human-facing notes for `achievement-hunter`. The `.rs` sources carry no
comments; this file and `AI-COMMENTS.md` are the only place explanations live.

## What this is

A single-binary egui/eframe desktop app for Steam achievement hunting. It
shows the user's Steam library, pulls achievement data and icons, reads
completion time and difficulty from the SteamHunters JSON API, caches cover art
from the Steam CDN, and lets the user annotate achievements, reorder them, and
draw a flowgraph of them.

## Layout

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | eframe launcher, window size 1280x820. |
| `src/lib.rs` | Module list; `test_env_lock()` serialises env-mutating tests. |
| `src/app.rs` | The entire GUI: `App`, the Games/Settings tab bar, the per-game sub-tabs (Achievements/Flow/Sort), the shared tab painter, background threads, image widgets. |
| `src/model.rs` | Plain data: `Achievement`, `GuideInfo`, `Game`, `FlowNode`, `FlowGraph`, `Tab`, `GameTab`, `SortMode`, filter/sort/move helpers. |
| `src/store.rs` | Loads/saves the persisted documents; atomic writes; triggers backups; merges refreshed Steam data into existing records. |
| `src/settings.rs` | `Settings`, `Theme` (Tokyo Night default), `FontSettings`, and the `apply(ctx)` entry point that pushes theme, fonts, spacing, and zoom into egui. |
| `src/paths.rs` | All filesystem locations (data/art/icons/backup dirs, settings/library/flow/sort paths) plus Steam root discovery and art/icon URLs. |
| `src/backup.rs` | Timestamped backups with ring pruning. |
| `src/http.rs` | One shared blocking `reqwest` client, query encoding, JSON GET/POST (with optional headers), and `.part`-swap downloads. |
| `src/steam.rs` | Steam Web API request builders + parsers (owned games, achievements, global percentages, schema/icons, vanity URL). |
| `src/vdf.rs` | Valve Data Format parsing for `libraryfolders.vdf`, `appmanifest_*.acf`, and `loginusers.vdf`. |
| `src/providers.rs` | Third-party sources: the SteamHunters JSON API, a HowLongToBeat link-only fallback, and SteamGridDB autocomplete/grids. |

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
- **Persisted structs are container-`#[serde(default)]`.** Every persisted
  struct (`Settings`, `FontSettings`, `Game`, `GuideInfo`, `Achievement`, …)
  carries a container-level `#[serde(default)]`. Without it, adding or removing
  a field makes deserialisation fail, `Store::load` silently falls back to
  `Settings::default()`, and the user's saved Steam API key is wiped.
- **Refresh merges, never drops cached data.** `Store::replace_games` carries a
  game's cached `achievements`, `art`, and `guides` across a refresh. The
  owned-games payload carries no achievements, so when a fresh record has none
  the cache is kept verbatim; a fresh record that *does* carry achievements
  replaces the cache, and existing notes are re-applied by `api_name`. Removing
  this rule makes every Refresh wipe every achievement and note.
- **Comments stay out of `.rs`.** Enforced by convention and by review.

## Guide sources

- **SteamHunters JSON API (default, keyless).**
  `GET https://steamhunters.com/api/apps/{appid}` → achievement count, median
  and fastest completion minutes, and player/perfected counts, from which a
  difficulty band is derived (`easy`/`moderate`/`hard`/`very hard`/`extreme`).
  `GET https://steamhunters.com/api/apps/{appid}/achievements` → per-achievement
  name, description, and global percentage, used to fill in achievements when
  the Steam schema is unavailable.
- **HowLongToBeat — link only.** Its search API is behind a WAF that rejects
  non-browser clients, so the app emits a link and never scrapes.
- **PSNProfiles — not implemented.** Cloudflare blocks every request regardless
  of user agent, and a server-side renderer proxy (`r.jina.ai`) gets the same
  block, so no client-side fetch can succeed. Achievement *type* (story, online,
  missable) has no public source in any of these APIs and is not shown.
  SteamHunters' per-achievement `obtainability` field is `0` across every game
  sampled, so it cannot stand in for a type either.

## UI behaviour

- **`hidden` achievements.** The `hidden` flag comes from the Steam
  `GetSchemaForGame` payload (numeric `0`/`1`) and is stored on `Achievement`. It
  is only known when a schema fetch succeeded, i.e. when a Steam API key is
  present; with no key it stays `false` rather than being guessed.
- **Notes are opt-in.** The note editor renders only for a row the user opened
  from its right-click menu ("Add / edit note"); a non-empty note renders as weak
  text under the description. The open row lives in `App::note_edit`.
- **Unsynced counts.** A game with no cached achievements reads "achievements not
  synced" (or "no achievements" when Steam says it has none) instead of a
  misleading `0 / 0`.
- **Locked/Unlocked toggles live in the game page**, next to the achievement
  list, not on the library page.
- **Installed-game precache.** `App::prefetch_installed` walks installed games
  that have no cached achievements, once per session, and loads them one at a
  time via `prefetch` (queue) and `prefetch_busy` (single in-flight guard). This
  also downloads their achievement icons. It is sequential on purpose and reuses
  the same `busy` guard as a manual load.
- **Library filters.** `model::Filters` carries installed-only, hide-no-achievements,
  hide-completed (every achievement unlocked), and hide-unstarted (none
  unlocked). A game with no cached achievements is never treated as 0% or 100% —
  unsynced is not a value, so it survives both filters. This also means the two
  percentage filters only bite once achievements have been fetched.
- **Library sort lives in Settings.** The Games page keeps only Refresh, the
  game count, the two filters, and the guide-source toggles. Sort mode (playtime
  / last played / name) is a Settings row because it re-sorts `library.json`
  itself; changing it marks both `settings` and `library` dirty so the choice
  survives a restart.
- **Responsive grid.** Art cells are fixed-width and placed in
  `ui.horizontal_wrapped`, so the grid reflows on resize; it is not an
  `egui::Grid` (which stretches its columns to the window and leaves uneven
  gaps).

## Environment / config

- Credentials: `STEAM_API_KEY` and `STEAM_ID` (or `STEAM_ID64`) env vars, or
  the `steam_api_key` / `steam_id` fields in `settings.toml`. Env wins on
  first launch via `Settings::merge_env`; after that the stored value is used.
- Offline behaviour: with no key, the library is populated from the local
  Steam install scan (`vdf.rs`) and art falls back to the keyless Steam CDN
  (`library_600x900.jpg`). SteamHunters guide data still works (keyless);
  per-achievement unlock state and icons do not.
- Steam roots probed: `~/.local/share/Steam`, `~/.steam/steam`,
  `~/.var/app/com.valvesoftware.Steam/...`, plus `libraryfolders.vdf` entries.

## Persistence format

- `settings.toml` — TOML (`paths::settings_path`).
- `library.json`, `flow.json`, `sort.json` — JSON (`paths::library_path` etc.).
- Backups land in `paths::backup_dir()` as `<stem>.<utc-stamp>.<ext>`.

## Testing

- `cargo test --lib` — unit tests, named `crucial_*`.
- The GUI render test (`app.rs::tests`) builds a headless `egui::Context`,
  runs every top-level `Tab` and every per-game `GameTab` (with a game selected)
  through `ctx.run_ui`, and drops the output with
  `drop_without_applying_deltas()`. Forgetting the drop call panics with
  "Dropped TexturesDelta with 1 unapplied deltas".
- Tests that mutate process env must hold `crate::test_env_lock()`.

## Known limits

- Guide data depends on a third-party JSON API; when its shape changes, only
  `providers.rs` needs updating. Parsers return a link-only guide rather than
  erroring, so a broken source degrades instead of failing the app.
- Flowgraph and sort are stored per app id, not per-user-profile.
- No screenshot-based visual test; CI runs the headless render test instead.
- The `hidden` badge needs a Steam API key: without one the schema is never
  fetched, so no achievement is marked hidden.
- Effective guide sources are SteamHunters and links outward; HowLongToBeat and
  PSNProfiles block non-browser requests (see "Guide sources").

## Security review

Scope: this application only, local desktop threat model. Reviewed surfaces are
credential handling, network I/O, untrusted response parsing, filesystem
writes, and the local Steam files we read.

Fixed:

- **Provider parsers panicked on multi-byte HTML.** `find_meta_description` and
  `find_difficulty` sliced the page at fixed byte offsets, which lands
  mid-character on any page with non-ASCII text before the target. The panic
  happened in the worker thread and the guide was silently lost. Both helpers,
  their tests, and the whole HTML-scraping path were later deleted when the
  providers moved to the SteamHunters JSON API.
  (`bugfix/provider-robustness`)
- **Steam API key leaked into error messages.** The key travels in the query
  string, and `reqwest`'s error `Display` embeds the request URL, so a failed
  request put the key in the status bar. Errors are now formatted with
  `Error::without_url()`.
- **Unbounded response bodies.** Every body read is now capped at 16 MiB.
  (`bugfix/http-error-hardening`)
- **Icon cache filename traversal.** The cached icon filename is derived from
  the provider-supplied `api_name`, which is attacker-controlled text. It is
  sanitized down to alphanumerics, `_`, and `-` before being joined onto
  `icons_dir()`, so `..` or `/` cannot escape the directory. Covered by
  `crucial_icon_path_never_escapes_the_icons_dir`.

Accepted, with rationale:

- **Font loading reads an arbitrary path.** The path is typed by the user for
  their own machine; there is no privilege boundary crossed.
- **Cover art URLs come from SteamGridDB's response,** so the app will fetch
  whatever URL that API returns. It only ever writes to `art_path(appid)`
  (derived from a `u32` app id), so there is no path traversal, and the request
  is a plain GET. Restricting the host was rejected because it would break
  legitimate custom grids.
- **Icon URLs come from the Steam schema response.** A bare hash is expanded to
  the Steam CDN; the file is only written under `icons/`, keyed on the app id
  and the sanitized achievement name.
- **SteamHunters responses are untrusted input.** They are deserialised into
  plain structs and only rendered as text; the one string that reaches the
  filesystem (the achievement name) goes through the sanitizer above.
- **Downloaded images are decoded by the `image` crate.** Decoder memory-safety
  is upstream's responsibility; responses are size-capped as above.
- **`steam.rs` passes the API key in the URL query string.** That is the Steam
  Web API's contract; requests are TLS-only and the key is never logged.
- **Deeply nested VDF input could recurse far**, but the only VDF files we read
  are ones the local Steam client wrote.
- **Backups use one-second timestamp resolution**, so two writes inside the
  same second overwrite the same backup file. The autosave debounce makes this
  unreachable in practice.

No `unsafe` outside tests, no `unwrap`/`expect`/`panic!` outside tests, and no
debug printing that could leak credentials.

