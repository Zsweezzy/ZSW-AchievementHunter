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
- Painting a custom widget: `ui.allocate_exact_size(size, Sense::click())` returns
  `(Rect, Response)`; draw with `ui.painter()`. `rect_filled(rect, radius, fill)`
  takes a `CornerRadius`, which is **u8**-based (`CornerRadius::same(8)` or the
  per-corner literal `{ nw, ne, sw, se }` for a top-only radius); `From<u8>` and
  `From<f32>` both exist. `Margin` is likewise **i8**-based
  (`Margin::same(i8)`, `Margin::symmetric(i8, i8)`).
- Text galley: `ui.painter().layout_no_wrap(text, FontId, color) -> Arc<Galley>`
  then `painter.galley(pos, galley, fallback_color)`.
- A panel's background is set by its `Frame`: `Panel::top(id).frame(Frame::NONE.fill(color).inner_margin(margin))`.
  A `bottom` inner margin of `0` makes a tab's bottom edge meet the panel edge,
  which is how the active tab appears to merge with the content below.
- `ui.put(rect, widget) -> Response` places a widget into an exact rect.
- `ui.horizontal` is `Layout::left_to_right(Align::Center)`, which centres each
  item against the row height known *so far* — so the first item sits at the top
  when a later item is taller. To vertically centre an item on a row whose height
  is only known afterwards, allocate a placeholder first, then re-place the item
  with `ui.put` at `Rect::from_center_size(pos2(placeholder.center().x,
  row.response.rect.center().y), size)`. `ui.put` does not advance the cursor.
- Fixed-width grid cells, the flush way:
  `ui.allocate_exact_size(vec2(W, H), Sense::…)` (the parent advances by exactly
  `W + item_spacing.x`) plus `ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)))`
  to draw inside that rect. `new_child` does **not** advance the parent, so
  content can never resize the cell. Do **not** use
  `ui.allocate_ui_with_layout(vec2(W, H), …)` for grid cells: it advances the
  cursor by the child's *content* rect (`scope_dyn` calls
  `advance_cursor_after_rect(child.min_rect())`), so long names widen their own
  cell and the row drifts, and content taller than `H` changes the wrapped line
  height and staggers the row. `allocate_exact_size` alone over-advances if you
  also wrap the content in a `scope` — pair it with `new_child`, or with nothing
  and just `painter` calls.
- Text inside a fixed cell must be single-line: `egui::Label::new(text).truncate()`
  (there is no `Label::weak()`; use `RichText::new(text).weak()`). Labels wrap by
  default in a vertical layout and wrapping is what made cell heights uneven.
- An `egui::Grid` stretches its columns to the window width and produces uneven
  gaps on fullscreen.
- Hover is exclusive to the topmost click/drag widget
  (`egui-0.36.2/src/interaction.rs`, the `hovered` set): only `hits.click` and
  `hits.drag` are hovered, plus *non-interactive* widgets above them. So a child
  widget with `Sense::click()` on top of a clickable parent steals the parent's
  hover — parent hover still works everywhere else, which makes the bug look
  intermittent. `Image`'s default sense is `Sense::hover()` (non-interactive, so
  it does not steal); `.interact(Sense::click())` is what breaks it. Nest an
  interactive child only when it needs its own hit area.
- A hover highlight over an image must be painted *after* the image (a
  translucent `visuals.selection.bg_fill.gamma_multiply(0.25)`) — a filled
  rect behind the cell only shows in the cell's padding and reads as a lopsided
  block. `Response::on_hover_cursor(CursorIcon::PointingHand)` is opt-in for
  hand-rolled clickable rects.
- Headless geometry check: pass
  `RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h))), .. }`
  to `run_ui` to control the layout width, then read painted rects back out of
  `FullOutput::shapes` (`Shape::Rect(r) => r.rect`) before calling
  `drop_without_applying_deltas()`.
- `Response::interact(Sense)` adds interaction to an existing response, e.g. to
  attach `.context_menu(...)` to a `Frame`'s response. Inside a menu,
  `ui.close()` closes it.
- `egui::Slider::new(&mut v, a..=b).fixed_decimals(2)` is the slider counterpart
  to `DragValue`.
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

- New field on a persisted struct: add to `model.rs` or `settings.rs`, give the
  **struct** a container-level `#[serde(default)]` (not just the new field), and
  round-trip it in a `crucial_*` test. Missing that default wipes saved settings
  on load — see the invariant in `DEV-NOTES.md`.
- New data source: add to `providers.rs`, return a link-only `GuideInfo`
  instead of erroring, and keep the network call inside the `http.rs` helpers.
- New UI action: read the current state from `App`, push a `Msg` to the worker
  thread, and apply the result in the `Msg` handler in `draw`.
- New tab: top-level tabs are the `Tab` enum (Games, Settings); per-game
  sub-tabs are `GameTab` (Achievements, Flow, Sort). Both bars are drawn by the
  shared `paint_tab` helper with a `TabStyle`.
- New cached image: add the destination to `paths.rs`, then check
  `path.exists()` before building the `egui::Image`, and trigger the download
  from a `spawn_*` method guarded by the `*_requested` set so it fires once.
- New per-achievement fact: parse it in `steam.rs` (`SchemaEntry` +
  `apply_schema`), add the field to `model::Achievement` with the container
  `#[serde(default)]`, and render it in `achievement_list`. Extend the
  `crucial_schema_*` test with the new payload key.
- New file location: add it to `paths.rs`; do not build paths inline.
