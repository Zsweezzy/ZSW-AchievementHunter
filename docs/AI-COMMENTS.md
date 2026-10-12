# AI working notes

Read this before editing. It is the entry point for any agent or model working
in this repo.

## Hard rules

1. **No comments in `.rs` files.** Not `//`, not `/* */`, not doc comments.
   Explanations go in `docs/DEV-NOTES.md` (human) or this file (AI). The one
   exception is a `shortcut:` marker required by the project style guide when a
   known limitation is shipped deliberately.
2. **Do not add a dependency for something the standard library or an installed
   crate already does.**
3. **Every persistence write goes through `Store`** so backups and atomic
   rename still happen. Never call `std::fs::write` on a store path.
4. **No async runtime.** Background work is `std::thread` + `mpsc` + a
   `ctx.request_repaint()`. Keep it that way.
5. **Never commit an API key or Steam ID.** Credentials come from env or
   `settings.toml`, which is user data and is not in the repo.

## Version pinning

- Rust toolchain: 1.98.1, edition 2024.
- egui / eframe / egui_extras: 0.36.x.
- reqwest: 0.13 (`features = ["blocking","json","rustls","webpki-roots","system-proxy"]`).
  Note: the feature is `rustls`, not `rustls-tls` in this version.
- toml 1.x. `image` 0.25 with `png`, `jpeg`, `webp`.

Before using an egui API from memory, check the vendored source under
`~/.cargo/registry/src/*/egui-0.36.2/`. The 0.36 API differs from older
tutorials in several ways.

## egui 0.36 API facts that bite

- `eframe::App` is implemented via `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)`
  (not the older `update(&mut self, ctx, frame)` shape). Optional `logic` and
  `save` hooks exist alongside it.
- Panels: `egui::Panel::top(id)` / `Panel::bottom(id)` take an id and `.show(ui, …)`.
- `egui::CentralPanel::default_margins().show(ui, …)`.
- `ctx.all_styles_mut(|style| …)`, `ctx.set_theme(egui::ThemePreference::Dark)`,
  `ctx.set_fonts(...)`.
- Drag and drop: `ui.dnd_drop_zone(egui::Frame::NONE, |ui| …)` — the frame is
  the first argument. Wrap the source in `ui.dnd_drag_source(id, payload, |ui| …)`.
- `ui.selectable_label(checked, text)` takes `impl IntoAtoms`.
- `egui::Image::new(impl Into<ImageSource>)`, then `.fit_to_exact_size`,
  `.corner_radius`, `.interact(egui::Sense::click())`. For `file://` art to
  load, `egui_extras` needs the `image` **and** `file` features.
- `egui::DragValue::range(...)`, `ui.color_edit_button_srgb(&mut [u8; 3])`.
- Headless testing: `egui::Context::default()` then
  `ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))` and finally
  `.drop_without_applying_deltas()`. Skipping the drop panics on the pending
  `TexturesDelta`.

## Verification loop

Run all of these before declaring a change done:

```
cargo fmt
cargo clippy --all-targets   # must be warning-free
cargo test --lib             # all crucial_* pass
cargo build
```

Optional live smoke (needs a display; exit 124 means it stayed up):

```
ACHIEVEMENT_HUNTER_DATA=/tmp/opencode/ah-run DISPLAY=:0 timeout 6 ./target/debug/achievement-hunter
```

## Where to make a change

- New field on a persisted struct: add to `model.rs` or `settings.rs`, give it
  a `#[serde(default)]`, and round-trip it in a `crucial_*` test.
- New data source: add to `providers.rs`, return a link-only `GuideInfo`
  instead of erroring, and keep the network call inside the `http.rs` helpers.
- New UI action: read the current state from `App`, push a `Msg` to the worker
  thread, and apply the result in the `Msg` handler in `draw`.
- New file location: add it to `paths.rs`; do not build paths inline.
