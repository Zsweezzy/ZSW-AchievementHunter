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
  notes. Notes, details, and pages stay hidden until you right-click an
  achievement. Sort the list by default order, name, rarity, unlocked first, or
  *Next up* (the locked achievements most players have, i.e. the likeliest to
  pop next) without disturbing the custom order.
- **Achievement details** — a mission/chapter line, a manual typical time to
  unlock (hours), and tags (multiplayer, singleplayer, missable, grind, bugged,
  impossible). Tags show under the achievement and filter the list. Steam has no
  public per-achievement pop-time data, so the typical time is yours to set.
- **Per-achievement pages** — write an HTML page per achievement (headings,
  paragraphs, lists, bold/italic, code, images, links) and the app renders it
  inline; *Open in browser* shows the real file. Pages live as plain `.html`
  files under `pages/<appid>/`, so you can edit them in any editor and keep
  screenshots next to them.
- **Guides** — completion time, difficulty, and per-achievement data from the
  SteamHunters JSON API, plus a HowLongToBeat link. Toggle which sources are
  queried. A source that cannot be reached degrades to a link, never an error.
- **Flowgraph** — draggable nodes, click-to-link edges, right-click to delete.
  Layout is saved per game.
- **Sort** — drag-and-drop reordering of achievements, saved per game.
- **Auto-refresh** — every 15 minutes by default (configurable in Settings,
  `0` turns it off) the app reloads the library, the selected game's
  achievements, and the Sons Of The Forest save, so progress updates without
  pressing Refresh.
- **Themes, fonts, and scale** — Tokyo Night by default, with every color
  editable in the settings tab. Load your own fonts by path, and set a global
  interface scale.
- **Backups** — every persisted file is backed up before it is overwritten,
  with a configurable retention count.

Games, achievements, and cover art are fetched lazily the first time a page is
opened, not at startup.

### Guide sources

SteamHunters is the working JSON API and supplies completion time, difficulty,
and per-achievement data. HowLongToBeat (WAF) and PSNProfiles (Cloudflare) reject
non-browser requests — PSNProfiles stays blocked even through a server-side
renderer proxy and even with an imported, signed-in `cf_clearance` cookie — so no
scraper ships for them. Each game page therefore links out to PSNProfiles' own
guide search (and to HowLongToBeat when that source is enabled), which never fails
because your browser does the work. Per-achievement *type* (story, online,
missable) has no public source in any of these APIs, so the app does not guess
it — the tags on each achievement are what you set yourself.

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

For the smallest binary (~4 MB instead of ~12 MB), `scripts/dist.sh` builds and
packs it with UPX:

```
./scripts/dist.sh
```

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
| `settings.toml` | Theme, fonts, interface scale, credentials, filters, sort modes, auto-refresh interval. |
| `library.json` | Games, achievements, notes, missions, typical times, tags, guide cache. |
| `flow.json` | Flowgraph nodes and edges. |
| `sort.json` | Custom achievement ordering. |
| `pages/<appid>/<api_name>.html` | Your per-achievement pages; relative images sit next to them. |

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
[`docs/AI-COMMENTS.md`](docs/AI-COMMENTS.md) for the agent-facing rules.
[`CONTRIBUTING.md`](CONTRIBUTING.md) has the build/test commands and the recipe
for adding a guide source or a setting.

```
cargo fmt
cargo clippy --all-targets
cargo test --lib
```

## License

MIT. See [LICENSE](LICENSE).
