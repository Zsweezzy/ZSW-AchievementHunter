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
| `src/app.rs` | The entire GUI: `App`, the Games/Settings tab bar, the per-game sub-tabs (Achievements/Flow/Sort/Guide), the shared tab painter, background threads, image widgets. |
| `src/model.rs` | Plain data: `Achievement`, `GuideInfo`, `Game`, `FlowNode`, `FlowGraph`, `Tab`, `GameTab`, `SortMode`, filter/sort/move helpers. |
| `src/store.rs` | Loads/saves the persisted documents; atomic writes; triggers backups; merges refreshed Steam data into existing records. |
| `src/settings.rs` | `Settings`, `Theme` (Tokyo Night default), `FontSettings`, and the `apply(ctx)` entry point that pushes theme, fonts, spacing, and zoom into egui. |
| `src/paths.rs` | All filesystem locations (data/art/icons/backup dirs, settings/library/flow/sort paths) plus Steam root discovery and art/icon URLs. |
| `src/backup.rs` | Timestamped backups with ring pruning. |
| `src/http.rs` | One shared blocking `reqwest` client, query encoding, JSON GET/POST (with optional headers), and `.part`-swap downloads. |
| `src/steam.rs` | Steam Web API request builders + parsers (owned games, achievements, global percentages, schema/icons, vanity URL). |
| `src/vdf.rs` | Valve Data Format parsing for `libraryfolders.vdf`, `appmanifest_*.acf`, and `loginusers.vdf`. |
| `src/providers.rs` | Third-party sources: the SteamHunters JSON API, a HowLongToBeat link-only fallback, and SteamGridDB autocomplete/grids. Also the local PSNProfiles guide reader (import + per-appid cache under `guides/psnprofiles/`). |

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
- **PSNProfiles — link only.** Cloudflare blocks every request regardless of
  user agent, and a server-side renderer proxy (`r.jina.ai`) gets the same block.
  Importing a real signed-in session (with a valid `cf_clearance`) does not fix
  it either: the clearance is bound to the browser fingerprint and IP that earned
  it. Measured: 1 of 3 browser-tier attempts returned the page. So the app builds
  a link to the site's own guide search and never fetches.
  The URL pattern is not guessed — it is taken from the site's own JS
  (`history.pushState(null, null, '/search/'+type+'?q='+encodeURIComponent(q))`,
  types `games|guides|trophies|users`), so the app opens
  `https://psnprofiles.com/search/guides?q=<url-encoded title>`. Deep-linking a
  game's guide page needs its numeric PSNProfiles id, which only a lookup through
  the Cloudflare wall could supply, and one title maps to several platform
  entries (e.g. PS3/PS4 versions), so the search page is the honest target.
  Achievement *type* (story, online,
  missable) has no public source in any of these APIs and is not shown.
  SteamHunters' per-achievement `obtainability` field is `0` across every game
  sampled, so it cannot stand in for a type either.
- **PSNProfiles guide text — imported, read-only, local.** The app never fetches
  PSNProfiles (Cloudflare, above). Instead the user runs a collector script in
  their own signed-in browser (`psnprofiles-collect.js`), which writes a JSON file
  shaped `{ "results": [ { "title", "pickedTitle", "pickedUrl", "markdown", … } ] }`.
  The Guide sub-tab imports it: it maps each entry's `title` to a Steam `appid`
  by case-insensitive name against the loaded library, skips entries with no match
  or empty `markdown`, and writes one file per game to
  `<data>/guides/psnprofiles/<appid>.json`. The import source is the first of
  `<data>/psn-guides.json` or `~/Downloads/psn-guides.json` that exists. Guide text
  lives only in the data dir and is never committed to the repo. `markdown` is
  rendered line-based (headings from leading `#`, image markdown dropped, link
  markdown reduced to its text) with no markdown-renderer dependency.
- **Single-page import.** Instead of the collector, one guide page can be copied
  and converted directly:
  `achievement-hunter --import-guide "<game name|appid>" <page.html>`.
  `providers::page_to_guide` strips tags, scripts, styles, nav/footer, comments
  and decodes entities, then collapses whitespace; a typical page drops to a few
  percent of its size. Steam guide pages are additionally scoped to their
  `guide subSections` container, so site chrome, the table of contents and the
  comments never reach the file. Achievement matching normalises curly
  apostrophes/quotes, since Steam names use `’` while many guides type `'`. The result is written to the same
  `<data>/guides/psnprofiles/<appid>.json`, so the app slices it into each
  achievement exactly like a collector import. The command resolves the game by
  exact or unique partial name (or `appid`) against the loaded library. Run it
  with the app closed or restart afterwards, since guides are attached at startup.

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
- **Responsive grid, exact cell geometry.** Art cells are placed in
  `ui.horizontal_wrapped` (never an `egui::Grid`, which stretches columns to the
  window and leaves uneven gaps), and each cell's rect comes from
  `ui.allocate_exact_size(vec2(CELL_WIDTH, CELL_HEIGHT), …)` drawn into with
  `ui.new_child(...)`. Both halves matter: `allocate_ui_with_layout` advances the
  cursor by the cell's *content* rect, so a wider game name used to widen its
  cell and drift the whole row (`x` pitch 150/152.2/161.5 instead of a constant
  176), and unequal content heights staggered cells vertically. Because the
  advance is now the fixed rect, cell pitch is `CELL_WIDTH + 12` and row pitch is
  `CELL_HEIGHT + 10` regardless of names, fonts, or sort mode. Cells must not
  paint outside that rect — all text is a single `.truncate()`d line, and
  `CELL_HEIGHT` (232) must stay above the tallest cell content
  (art 100×150 + three text lines + 6 pt gaps = ~225 in LastPlayed mode).
  Regression test: `app::tests::crucial_library_grid_cells_are_flush` renders a
  700 px-wide grid (narrow enough that six games must wrap) and reads the
  painted artwork rectangles out of the headless `FullOutput` and asserts the
  placeholder (ART_WIDTH×ART_HEIGHT) as well as the loaded image's painted rect;
  the art box shares the 2:3 source aspect so aspect-preserving fit renders loaded
  art at exactly the placeholder size.
- **Cell hover is the cell's job, not the artwork's.** The whole cell is one
  `Sense::click()` rect (with a pointing-hand cursor) and the artwork is
  deliberately *not* interactive. egui hovers only the single topmost
  click/drag widget plus non-interactive widgets above it, so giving the
  artwork its own `Sense::click` made it the top hit and the cell stopped
  being hovered over the art — the highlight vanished exactly where the user's
  pointer was. The hover fill is a translucent tint of
  `visuals.selection.bg_fill` painted *over* the artwork rect (a fill behind
  the cell rect only shows as an off-centre block in the padding). Regression
  test: `app::tests::crucial_library_cell_stays_hoverable_over_the_artwork`.

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

## Release build

`Cargo.toml`'s `[profile.release]` is tuned for size: `opt-level = "s"`,
fat `lto`, `codegen-units = 1`, `panic = "abort"`, `strip = true`, and only
the renderer/codecs the app uses (`eframe` with `glow`, no `wgpu`; `image`
with png/jpeg only). Plain `cargo build --release` produces the small binary.

If `cc` comes from a hermetic toolchain that ships its own glibc, the linker
records that glibc as the ELF interpreter; its loader does not search
`/usr/lib`, so `dlopen("libwayland-client.so.0")` fails at startup
(`NoWaylandLib`). Build with the system linker instead:

```sh
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/gcc cargo build --release
```

For the smallest artifact, pack the stripped binary with UPX (a packaging
step, not part of `cargo`):

```sh
upx --best --lzma target/release/achievement-hunter \
  -o target/release/achievement-hunter.upx
```

Reference sizes: 36 MB (old profile) -> 12.4 MB (size profile) -> 4.2 MB
(UPX). UPX costs a few ms of startup decompression and is flagged by some
antivirus/heuristic scanners; skip it if that matters.

`[profile.dev] debug = false` keeps `target/debug` from ballooning (debug
info dominated the old 13 GB `target/`). `cargo clean` reclaims all build
artifacts at any time; the installed binary lives in `~/.local/bin`.

## Known limits

- Guide data depends on a third-party JSON API; when its shape changes, only
  `providers.rs` needs updating. Parsers return a link-only guide rather than
  erroring, so a broken source degrades instead of failing the app.
- Flowgraph and sort are stored per app id, not per-user-profile.
- No screenshot-based visual test; CI runs the headless render test instead.
- The `hidden` badge needs a Steam API key: without one the schema is never
  fetched, so no achievement is marked hidden.
- Effective guide sources are SteamHunters and links outward; HowLongToBeat
  blocks non-browser requests and PSNProfiles blocks them entirely, so both are
  link-only (PSNProfiles links to its guide search, see "Guide sources").

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

