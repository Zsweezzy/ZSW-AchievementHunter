# Contributing

Thanks for helping. The app is a single Rust binary (egui/eframe); there is no
backend, no webview and no plugin runtime. Most contributions are a small,
self-contained change in `src/` plus a test.

## Set up

Requires Rust 1.98 or newer (edition 2024).

```
cargo build
cargo test --lib
```

Run it from `./target/debug/achievement-hunter`. To try it against a throwaway
data folder instead of your real one:

```
ACHIEVEMENT_HUNTER_DATA=/tmp/ah-dev ./target/debug/achievement-hunter
```

Before opening a pull request, all three must pass:

```
cargo fmt
cargo clippy --all-targets
cargo test --lib
```

## Where things live

| File | What goes there |
| --- | --- |
| `src/app.rs` | The whole GUI: `App`, tabs, background threads, widgets. |
| `src/model.rs` | Plain data types (`Game`, `Achievement`, `Tag`, …) and filter/sort helpers. |
| `src/page.rs` | Per-achievement HTML pages: file paths, save/load, HTML-subset parser. |
| `src/settings.rs` | `Settings`, theme, fonts. |
| `src/store.rs` | Loading/saving the persisted documents. |
| `src/providers.rs` | Third-party sources (guide APIs, SteamGridDB). |
| `src/steam.rs`, `src/sotf.rs`, `src/vdf.rs` | Steam Web API, Sons Of The Forest save, Valve file formats. |

`docs/DEV-NOTES.md` explains architecture and invariants. Read the Invariants
section before touching persistence, refresh or threading — those rules exist
because breaking them silently drops user data.

## Code rules

- **Standard library or an installed crate first.** Do not add a dependency for
  something small. Do not add an async runtime: background work is
  `std::thread` + `mpsc` + `ctx.request_repaint()`, never network on the UI
  thread.
- **Persistence goes through `Store`.** Never `fs::write` a store path yourself.
- **Every persisted struct keeps a container-level `#[serde(default)]`.** Without
  it, adding a field makes loading fail and `Store::load` quietly resets
  `settings.toml`, wiping the user's Steam API key.
- **Comments stay sparse.** One line for a non-obvious *why*; architecture and
  measurements belong in `docs/DEV-NOTES.md`. No commented-out code.
- **Match the surrounding code**, including formatting and naming.

## Tests

`cargo test --lib` must stay green. Tests that mutate environment variables take
`crate::test_env_lock()` first; headless UI tests build the app with
`App::headless(ctx)` and a temporary `ACHIEVEMENT_HUNTER_DATA`.

Name a test `crucial_*` when it guards something that must never regress (data
loss, a persisted format, a layout bug that already shipped). Those tests are
expected to fail loudly when the invariant breaks, so test the real path, not a
convenience wrapper.

## Recipe: add a guide source

A source is one fetch function plus one registry entry.

1. Write the fetch function in `src/providers.rs`. It takes the game and returns
   a `GuideInfo` plus an optional error string, and it must never fail outright:
   if the site is unreachable, return a link-only guide so the UI still shows
   something.

   ```rust
   fn example(game: &Game) -> (GuideInfo, Option<String>) { … }
   ```

2. Add a field to `Settings` in `src/settings.rs` (with a default) and to
   `Settings::default`.

3. Add an entry to `GUIDE_SOURCES` in `src/providers.rs`:

   ```rust
   GuideSource {
       id: "example",
       label: "Example Guides",
       enabled: |s| s.guide_example,
       toggle: |s| &mut s.guide_example,
       fetch: example,
   },
   ```

4. That is it: `fetch_guides` runs the enabled entries and the Settings tab draws
   one checkbox per source from the same table. Run
   `cargo test --lib` — `crucial_every_guide_source_toggles_its_own_setting`
   checks that `enabled` and `toggle` agree.

If the source only produces a link (the site blocks scrapers), still register it:
`extra_guide_links` and the link-only path in `providers.rs` show how. Do not add
a scraper that needs a logged-in browser session; document the block in
`docs/DEV-NOTES.md` instead.

## Recipe: add a setting

Add the field to `Settings` and its default, then add a row in
`app::settings_tab`. Mark the row `structural`/`theme_dirty` if it changes
persisted state or appearance. Keep the old TOML loadable: new fields rely on
the container `#[serde(default)]`.

## Pull requests

- Branch off `main`, use a short-lived branch name (`feature/…`, `fix/…`).
- One change per pull request. Conventional Commits for the subject
  (`feat(achievements): sort options`), body explaining why.
- Say what the user-visible change is and how you tested it. Screenshots help for
  UI changes.
- Do not commit `settings.toml`, `library.json`, API keys, Steam IDs, or the
  guide cache.
