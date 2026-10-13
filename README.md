# achievement-hunter

A single-binary desktop companion for Steam achievement hunting. It shows your
Steam library, pulls achievement progress and icons, reads completion time and
guide data from SteamHunters, caches cover art, and lets you take notes, reorder
achievements, and chart them as a flowgraph.

Built with Rust, [egui](https://github.com/emilk/egui) and
[eframe](https://github.com/emilk/egui/tree/master/crates/eframe). No runtime,
no webview, no bundled browser — one native executable.

## Features

- **Library grid** — every owned Steam game with cover art, playtime, and
  completion percentage. The grid reflows to more columns as the window widens.
  Refresh on demand, filter to installed games only, and hide games that have no
  achievements, that you have already finished (100%), or that you have not
  started (0%). Sort by playtime, last played, or name (set in Settings). A game
  whose achievements have not been fetched yet says so instead of showing
  `0 / 0`.
- **Achievement view** — per-game achievement list with the game's own icon for
  each achievement (dimmed when still locked), global unlock rates, a `hidden`
  badge for achievements Steam hides until they are earned, and per-achievement
  notes. Notes stay hidden until you right-click an achievement and pick
  *Add / edit note*.
- **Guides** — completion time, difficulty, and per-achievement data from the
  SteamHunters JSON API, plus a HowLongToBeat link. Toggle which sources are
  queried. A source that cannot be reached degrades to a link, never an error.
- **Flowgraph** — draggable nodes, click-to-link edges, right-click to delete.
  Layout is saved per game.
- **Sort** — drag-and-drop reordering of achievements, saved per game.
- **Themes, fonts, and scale** — Tokyo Night by default, with every color
  editable in the settings tab. Load your own fonts by path, and set a global
  interface scale.
- **Backups** — every persisted file is backed up before it is overwritten,
  with a configurable retention count.

Games, achievements, and cover art are fetched lazily the first time a page is
opened, not at startup.

### Guide sources that no longer work

HowLongToBeat (WAF) and PSNProfiles (Cloudflare) both reject non-browser
requests — PSNProfiles stays blocked even through a server-side renderer proxy —
so no scraper ships for them; a PSNProfiles scraper would fail on every request.
SteamHunters is the working default and supplies completion time, difficulty, and
per-achievement data. Per-achievement *type* (story, online, missable) has no
public source in any of these APIs, so it is not shown rather than guessed.

## Install

Requires Rust 1.98 or newer.

```
git clone https://github.com/Zsweezzy/achievement-hunter
cd achievement-hunter
cargo build --release
./target/release/achievement-hunter
```

The binary is self-contained. Nothing else needs to be installed for the app to
run.

## Steam credentials

Achievement data comes from the Steam Web API and needs a key. Get one at
<https://steamcommunity.com/dev/apikey>, then either export it:

```
export STEAM_API_KEY=your_key
export STEAM_ID=7656119xxxxxxxxxx
```

or paste both into the **Settings** tab inside the app. Environment variables
are read on first launch; after that the stored values are used.

Without a key the app still works: it scans your local Steam install for the
game list, loads cover art from the Steam CDN, and fetches SteamHunters guide
data (that API is keyless), but per-achievement unlock state is not available.

Do not commit your key. `settings.toml` is user data and is not tracked.

## Data and backups

Stored under your platform data directory (override with
`ACHIEVEMENT_HUNTER_DATA`):

| File | Contents |
| --- | --- |
| `settings.toml` | Theme, fonts, interface scale, credentials, filters, sort mode. |
| `library.json` | Games, achievements, notes, guide cache. |
| `flow.json` | Flowgraph nodes and edges. |
| `sort.json` | Custom achievement ordering. |

Achievement and cover-art images are cached under `cache/icons/` and
`cache/art/` in the same tree. Cover art is cached for every game; achievement
icons are precached for installed games (one game at a time, after the library
loads), so they are available offline once fetched.

Backups are timestamped copies kept in the same directory tree, pruned to the
retention count set in Settings. Before each save the app copies the previous
`settings.toml`, `library.json`, `flow.json`, and `sort.json` into the backup
folder, keeping the newest N.

## Development

See [`docs/DEV-NOTES.md`](docs/DEV-NOTES.md) for architecture and invariants and
[`docs/AI-COMMENTS.md`](docs/AI-COMMENTS.md) for the contributor rules (the
`.rs` files are comment-free by design).

```
cargo fmt
cargo clippy --all-targets
cargo test --lib
```

## License

MIT. See [LICENSE](LICENSE).
