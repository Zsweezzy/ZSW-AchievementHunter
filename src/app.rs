use crate::model::{self, FlowGraph, FlowNode, Game, GuideInfo, Tab};
use crate::providers;
use crate::steam;
use crate::store::Store;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

pub struct App {
    store: Store,
    ctx: egui::Context,
    tab: Tab,
    selected: Option<u32>,
    status: String,
    busy: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    dirty: Dirty,
    save_at: Option<Instant>,
    sort_game: Option<u32>,
    flow_game: Option<u32>,
    flow_drag: Option<usize>,
    flow_link: Option<usize>,
}

#[derive(Clone, Copy, Default)]
struct Dirty {
    settings: bool,
    library: bool,
    flow: bool,
    sort: bool,
}

impl Dirty {
    fn any(self) -> bool {
        self.settings || self.library || self.flow || self.sort
    }
}

enum Msg {
    Games(Vec<Game>, String),
    Achievements(u32, Vec<crate::model::Achievement>, String),
    Guides(u32, Vec<GuideInfo>, String),
    Art(u32, String),
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut store = Store::load();
        store.settings.merge_env();
        if store.settings.steam_id.is_empty()
            && let Some(id) = steam::local_steam_id()
        {
            store.settings.steam_id = id;
        }
        store.settings.apply(&cc.egui_ctx);
        let (tx, rx) = channel();
        let mut app = Self {
            store,
            ctx: cc.egui_ctx.clone(),
            tab: Tab::Games,
            selected: None,
            status: String::new(),
            busy: false,
            tx,
            rx,
            dirty: Dirty::default(),
            save_at: None,
            sort_game: None,
            flow_game: None,
            flow_drag: None,
            flow_link: None,
        };
        app.status = app.ready_status();
        app.spawn_art();
        app
    }

    fn ready_status(&self) -> String {
        if self.store.settings.has_steam_credentials() {
            format!(
                "{} games cached. Press Refresh to pull from Steam.",
                self.store.games.len()
            )
        } else {
            format!(
                "{} games cached. Add a Steam Web API key in Settings to pull your full library.",
                self.store.games.len()
            )
        }
    }

    fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Games(games, status) => {
                    self.store.replace_games(games);
                    self.dirty.library = true;
                    self.busy = false;
                    self.status = status;
                    self.spawn_art();
                }
                Msg::Achievements(appid, achievements, status) => {
                    if let Some(game) = self.store.game_mut(appid) {
                        game.achievements = achievements;
                    }
                    self.dirty.library = true;
                    self.busy = false;
                    if !status.is_empty() {
                        self.status = status;
                    }
                }
                Msg::Guides(appid, guides, status) => {
                    if let Some(game) = self.store.game_mut(appid) {
                        game.guides = guides;
                    }
                    self.dirty.library = true;
                    self.busy = false;
                    if !status.is_empty() {
                        self.status = status;
                    }
                }
                Msg::Art(appid, path) => {
                    if let Some(game) = self.store.game_mut(appid) {
                        game.art = path;
                    }
                    self.dirty.library = true;
                }
            }
        }
    }

    fn refresh(&mut self) {
        self.busy = true;
        self.status = "Refreshing library...".to_owned();
        let settings = self.store.settings.clone();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let installed = steam::installed_apps();
            let (mut games, mut status) = if settings.has_steam_credentials() {
                match steam::owned_games(&settings.steam_api_key, &settings.steam_id) {
                    Ok(games) => {
                        let count = games.len();
                        (games, format!("Loaded {count} owned games from Steam."))
                    }
                    Err(error) => (
                        steam::games_from_installed_only(&installed),
                        format!("Steam API failed ({error}); showing locally installed games."),
                    ),
                }
            } else {
                (
                    steam::games_from_installed_only(&installed),
                    format!(
                        "No Steam API key; showing {} locally installed games.",
                        installed.len()
                    ),
                )
            };
            steam::apply_installed(&mut games, &installed);
            if games.is_empty() {
                status = "Nothing found. Set a Steam API key or a Steam install path.".to_owned();
            }
            let _ = tx.send(Msg::Games(games, status));
            ctx.request_repaint();
        });
    }

    fn spawn_art(&mut self) {
        let missing: Vec<(u32, String)> = self
            .store
            .games
            .iter()
            .filter(|g| g.art.is_empty())
            .map(|g| (g.appid, g.name.clone()))
            .collect();
        if missing.is_empty() {
            return;
        }
        let settings = self.store.settings.clone();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            for (appid, name) in missing {
                let dest = crate::paths::art_path(appid);
                if !dest.exists() {
                    let mut done = false;
                    if settings.use_steamgriddb
                        && !settings.steamgriddb_key.is_empty()
                        && let Ok(url) =
                            providers::steamgriddb_grid(&name, &settings.steamgriddb_key)
                    {
                        done = crate::http::download(&url, &dest).is_ok();
                    }
                    if !done {
                        done = crate::http::download(&crate::paths::art_url(appid), &dest).is_ok();
                    }
                    if !done {
                        continue;
                    }
                }
                let _ = tx.send(Msg::Art(appid, dest.display().to_string()));
                ctx.request_repaint();
            }
        });
    }

    fn load_achievements(&mut self, appid: u32) {
        let settings = self.store.settings.clone();
        if !settings.has_steam_credentials() {
            self.status = "Add a Steam Web API key in Settings to load achievements.".to_owned();
            return;
        }
        self.busy = true;
        self.status = "Loading achievements...".to_owned();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut status = String::new();
            let mut achievements =
                match steam::achievements(&settings.steam_api_key, &settings.steam_id, appid) {
                    Ok(achievements) => achievements,
                    Err(error) => {
                        status = format!("Achievements unavailable: {error}");
                        Vec::new()
                    }
                };
            if let Ok(percentages) = steam::global_percentages(appid) {
                for achievement in &mut achievements {
                    achievement.global_percent = percentages.get(&achievement.api_name).copied();
                }
            }
            let _ = tx.send(Msg::Achievements(appid, achievements, status));
            ctx.request_repaint();
        });
    }

    fn load_guides(&mut self, appid: u32) {
        let settings = self.store.settings.clone();
        let Some(game) = self.store.game(appid).cloned() else {
            return;
        };
        self.busy = true;
        self.status = "Fetching guides...".to_owned();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let (guides, errors) = providers::fetch_guides(&game, &settings);
            let status = if errors.is_empty() {
                format!("Guides refreshed for {}.", game.name)
            } else {
                format!("Guides for {}: {}", game.name, errors.join("; "))
            };
            let _ = tx.send(Msg::Guides(appid, guides, status));
            ctx.request_repaint();
        });
    }

    fn flush(&mut self) {
        if self.dirty.settings {
            let _ = self.store.save_settings();
        }
        if self.dirty.library {
            let _ = self.store.save_library();
        }
        if self.dirty.flow {
            let _ = self.store.save_flow();
        }
        if self.dirty.sort {
            let _ = self.store.save_sort();
        }
        self.dirty = Dirty::default();
        self.save_at = None;
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if !self.dirty.any() {
            self.save_at = None;
            return;
        }
        let started = *self.save_at.get_or_insert_with(Instant::now);
        if started.elapsed().as_secs_f32() >= 1.0 {
            self.flush();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tab in Tab::ALL {
                if ui.selectable_label(self.tab == tab, tab.label()).clicked() {
                    self.tab = tab;
                }
            }
        });
    }

    fn pick_game(&mut self, ui: &mut egui::Ui, salt: &str) -> Option<u32> {
        if self.store.games.is_empty() {
            return None;
        }
        let mut selected = if salt == "sort" {
            self.sort_game
        } else {
            self.flow_game
        };
        if selected
            .map(|id| self.store.game(id).is_none())
            .unwrap_or(true)
        {
            selected = Some(self.store.games[0].appid);
        }
        let current = selected
            .and_then(|id| self.store.game(id))
            .map(|g| g.name.clone())
            .unwrap_or_default();
        let games = self.store.games.clone();
        egui::ComboBox::from_id_salt(format!("pick_{salt}"))
            .selected_text(current)
            .width(320.0)
            .show_ui(ui, |ui| {
                for game in &games {
                    ui.selectable_value(&mut selected, Some(game.appid), game.name.as_str());
                }
            });
        if salt == "sort" {
            self.sort_game = selected;
        } else {
            self.flow_game = selected;
        }
        selected
    }

    fn games_tab(&mut self, ui: &mut egui::Ui) {
        let total = self.store.games.len();
        let shown = if self.store.settings.installed_only {
            self.store.games.iter().filter(|g| g.installed).count()
        } else {
            total
        };
        let mut refresh = false;
        ui.horizontal_wrapped(|ui| {
            refresh = ui.button("Refresh library").clicked();
            ui.separator();
            ui.label(format!("{shown} / {total} games"));
            ui.separator();
            ui.checkbox(&mut self.store.settings.installed_only, "Installed only");
            ui.checkbox(&mut self.store.settings.filter_locked, "Locked");
            ui.checkbox(&mut self.store.settings.filter_unlocked, "Unlocked");
            ui.separator();
            ui.label("Guides from:");
            ui.checkbox(&mut self.store.settings.guide_hltb, "HowLongToBeat");
            ui.checkbox(&mut self.store.settings.guide_steamhunters, "SteamHunters");
        });
        if refresh {
            self.refresh();
        }
        ui.separator();
        if let Some(appid) = self.selected {
            self.game_detail(ui, appid);
            return;
        }
        if self.store.games.is_empty() {
            ui.add_space(24.0);
            ui.label(
                "No games yet. Press Refresh library, or add a Steam Web API key in Settings.",
            );
            return;
        }
        let columns = self.store.settings.grid_columns.max(1);
        let installed_only = self.store.settings.installed_only;
        let cells: Vec<(u32, String, String, usize, usize)> = self
            .store
            .games
            .iter()
            .filter(|g| !installed_only || g.installed)
            .map(|g| {
                (
                    g.appid,
                    g.name.clone(),
                    g.art.clone(),
                    g.unlocked_count(),
                    g.achievements.len(),
                )
            })
            .collect();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("games_grid")
                    .num_columns(columns)
                    .spacing(egui::vec2(12.0, 12.0))
                    .show(ui, |ui| {
                        for (index, (appid, name, art, unlocked, count)) in cells.iter().enumerate()
                        {
                            ui.vertical(|ui| {
                                let response = art_widget(ui, name, art, egui::vec2(150.0, 200.0));
                                if response.clicked() {
                                    clicked = Some(*appid);
                                }
                                let label = truncate(name, 24);
                                if ui.selectable_label(false, label).clicked() {
                                    clicked = Some(*appid);
                                }
                                ui.weak(format!("{unlocked} / {count} achievements"));
                            });
                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        if let Some(appid) = clicked {
            self.selected = Some(appid);
        }
    }

    fn game_detail(&mut self, ui: &mut egui::Ui, appid: u32) {
        let mut back = false;
        let mut load = false;
        let mut guides = false;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("< Back").clicked();
            if let Some(game) = self.store.game(appid) {
                ui.heading(game.name.as_str());
                ui.label(format!(
                    "{} / {} achievements ({:.0}%) \u{2022} {} hours",
                    game.unlocked_count(),
                    game.achievements.len(),
                    game.completion_percent(),
                    game.playtime_minutes / 60
                ));
            }
            load = ui.button("Load achievements").clicked();
            guides = ui.button("Fetch guides").clicked();
        });
        if back {
            self.selected = None;
            return;
        }
        if load {
            self.load_achievements(appid);
        }
        if guides {
            self.load_guides(appid);
        }
        let mut open: Option<String> = None;
        if let Some(game) = self.store.game(appid) {
            for guide in &game.guides {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("[{}]", guide.source));
                    ui.label(guide.summary.as_str());
                    if ui.link("open").clicked() {
                        open = Some(guide.url.clone());
                    }
                });
            }
        }
        if let Some(url) = open {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        ui.separator();
        let (show_locked, show_unlocked) = (
            self.store.settings.filter_locked,
            self.store.settings.filter_unlocked,
        );
        let mut touched = false;
        let Some(game) = self.store.game_mut(appid) else {
            return;
        };
        if game.achievements.is_empty() {
            ui.label("No achievement data cached. Press Load achievements.");
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for achievement in &mut game.achievements {
                    if !model::passes_filter(achievement, show_locked, show_unlocked) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(if achievement.unlocked { "[x]" } else { "[ ]" });
                        ui.strong(achievement.name.as_str());
                        if let Some(percent) = achievement.global_percent {
                            ui.weak(format!("{percent:.1}% of players"));
                        }
                    });
                    if !achievement.description.is_empty() {
                        ui.label(achievement.description.as_str());
                    }
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut achievement.note)
                            .hint_text("note")
                            .desired_width(f32::INFINITY),
                    );
                    if response.lost_focus() {
                        touched = true;
                    }
                    ui.separator();
                }
            });
        if touched {
            self.dirty.library = true;
        }
    }

    fn sort_tab(&mut self, ui: &mut egui::Ui) {
        let Some(appid) = self.pick_game(ui, "sort") else {
            ui.label("No games loaded. Refresh on the Games tab first.");
            return;
        };
        let rows: Vec<(String, String, bool)> = match self.store.game(appid) {
            Some(game) => {
                let order = self.store.sort.get(&appid).cloned().unwrap_or_default();
                model::sort_order_for(&game.achievements, &order)
                    .into_iter()
                    .map(|a| (a.api_name.clone(), a.name.clone(), a.unlocked))
                    .collect()
            }
            None => Vec::new(),
        };
        let mut reset = false;
        ui.horizontal(|ui| {
            reset = ui.button("Reset to default order").clicked();
            ui.label("Drag a row to relocate it.");
        });
        if reset {
            self.store.sort.remove(&appid);
            self.dirty.sort = true;
        }
        if rows.is_empty() {
            ui.label("No achievements cached for this game. Load them from the Games tab.");
            return;
        }
        let mut order: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let mut pending = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, (api_name, name, unlocked)) in rows.iter().enumerate() {
                    let (_, dropped) = ui.dnd_drop_zone::<usize, _>(egui::Frame::NONE, |ui| {
                        ui.dnd_drag_source(
                            egui::Id::new(("sort_row", appid, index)),
                            index,
                            |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(if *unlocked { "[x]" } else { "[ ]" });
                                    ui.label(name.as_str());
                                    ui.weak(api_name.as_str());
                                });
                            },
                        );
                    });
                    if let Some(from) = dropped {
                        pending = Some((*from, index));
                    }
                    ui.separator();
                }
            });
        if let Some((from, to)) = pending {
            model::move_element(&mut order, from, to);
            self.store.sort.insert(appid, order);
            self.dirty.sort = true;
        }
    }

    fn flow_tab(&mut self, ui: &mut egui::Ui) {
        let Some(appid) = self.pick_game(ui, "flow") else {
            ui.label("No games loaded. Refresh on the Games tab first.");
            return;
        };
        let mut seed = false;
        let mut clear = false;
        ui.horizontal_wrapped(|ui| {
            seed = ui.button("Stamp achievement nodes").clicked();
            clear = ui.button("Clear graph").clicked();
            let linking = self.flow_link.is_some();
            if ui.selectable_label(linking, "linking...").clicked() {
                self.flow_link = None;
            }
            ui.label(
                "Drag to move \u{2022} click two nodes to link \u{2022} right-click to delete",
            );
        });
        if clear {
            self.store.flow.insert(appid, FlowGraph::default());
            self.flow_link = None;
            self.dirty.flow = true;
        }
        if seed {
            self.flow_seed(appid);
        }
        let labels: std::collections::HashMap<String, String> = self
            .store
            .game(appid)
            .map(|game| {
                game.achievements
                    .iter()
                    .map(|a| (a.api_name.clone(), a.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let mut graph = self.store.flow.remove(&appid).unwrap_or_default();
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        let radius = 46.0;
        let accent = ui.visuals().hyperlink_color;
        let fill = ui.visuals().widgets.inactive.bg_fill;
        let text = ui.visuals().text_color();
        let pointer = response.interact_pointer_pos();
        if graph.nodes.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Stamp achievement nodes to start a flowgraph.",
                egui::FontId::proportional(14.0),
                ui.visuals().weak_text_color(),
            );
        }
        for (from, to) in &graph.edges {
            if let (Some(a), Some(b)) = (graph.nodes.get(*from), graph.nodes.get(*to)) {
                painter.line_segment(
                    [
                        rect.min + egui::vec2(a.x, a.y),
                        rect.min + egui::vec2(b.x, b.y),
                    ],
                    egui::Stroke::new(2.0, accent),
                );
            }
        }
        for (index, node) in graph.nodes.iter().enumerate() {
            let center = rect.min + egui::vec2(node.x, node.y);
            let active = self.flow_drag == Some(index) || self.flow_link == Some(index);
            painter.circle_filled(center, radius, fill);
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(if active { 3.0 } else { 1.0 }, accent),
            );
            let label = labels
                .get(&node.api_name)
                .cloned()
                .unwrap_or_else(|| node.api_name.clone());
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                truncate(&label, 16),
                egui::FontId::proportional(11.0),
                text,
            );
        }
        if response.drag_started() {
            self.flow_drag = pointer.and_then(|p| hit_node(&graph, rect, p, radius));
        }
        if response.dragged()
            && let Some(index) = self.flow_drag
        {
            let delta = response.drag_delta();
            if let Some(node) = graph.nodes.get_mut(index) {
                node.x = (node.x + delta.x).clamp(radius, (rect.width() - radius).max(radius));
                node.y = (node.y + delta.y).clamp(radius, (rect.height() - radius).max(radius));
                self.dirty.flow = true;
            }
        }
        if response.drag_stopped() {
            self.flow_drag = None;
        }
        if response.clicked()
            && let Some(index) = pointer.and_then(|p| hit_node(&graph, rect, p, radius))
        {
            match self.flow_link {
                None => self.flow_link = Some(index),
                Some(from) if from != index => {
                    if !graph.edges.contains(&(from, index)) {
                        graph.edges.push((from, index));
                        self.dirty.flow = true;
                    }
                    self.flow_link = None;
                }
                Some(_) => {}
            }
        }
        if response.secondary_clicked()
            && let Some(index) = pointer.and_then(|p| hit_node(&graph, rect, p, radius))
        {
            graph.nodes.remove(index);
            graph.edges.retain(|(a, b)| *a != index && *b != index);
            graph.edges = graph
                .edges
                .iter()
                .map(|(a, b)| {
                    (
                        if *a > index { *a - 1 } else { *a },
                        if *b > index { *b - 1 } else { *b },
                    )
                })
                .collect();
            self.flow_link = None;
            self.flow_drag = None;
            self.dirty.flow = true;
        }
        self.store.flow.insert(appid, graph);
    }

    fn flow_seed(&mut self, appid: u32) {
        let names: Vec<String> = self
            .store
            .game(appid)
            .map(|game| {
                game.achievements
                    .iter()
                    .map(|a| a.api_name.clone())
                    .collect()
            })
            .unwrap_or_default();
        if names.is_empty() {
            self.status = "Load achievements first so there are nodes to stamp.".to_owned();
            return;
        }
        let graph = self.store.flow.entry(appid).or_default();
        for (index, api_name) in names.iter().enumerate() {
            if graph.nodes.iter().any(|n| &n.api_name == api_name) {
                continue;
            }
            let column = (index % 5) as f32;
            let row = (index / 5) as f32;
            graph.nodes.push(FlowNode {
                api_name: api_name.clone(),
                x: 70.0 + column * 120.0,
                y: 70.0 + row * 110.0,
            });
        }
        self.dirty.flow = true;
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let ctx = self.ctx.clone();
        let theme = self.store.settings.theme.clone();
        let mut settings = self.store.settings.clone();
        let mut save_now = false;
        let mut structural = false;
        let mut theme_dirty = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("Steam");
                egui::Grid::new("steam_grid")
                    .num_columns(2)
                    .spacing(egui::vec2(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label("Steam Web API key");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut settings.steam_api_key)
                                .password(true)
                                .desired_width(430.0),
                        );
                        structural |= response.lost_focus();
                        ui.end_row();
                        ui.label("SteamID64 or vanity name");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut settings.steam_id).desired_width(430.0),
                        );
                        structural |= response.lost_focus();
                        ui.end_row();
                        ui.label("SteamGridDB API key");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut settings.steamgriddb_key)
                                .password(true)
                                .desired_width(430.0),
                        );
                        structural |= response.lost_focus();
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut settings.use_steamgriddb,
                    "Use SteamGridDB for grid artwork",
                );
                ui.horizontal(|ui| {
                    ui.checkbox(&mut settings.guide_hltb, "Guides: HowLongToBeat");
                    ui.checkbox(&mut settings.guide_steamhunters, "Guides: SteamHunters");
                });
                ui.separator();

                ui.heading("Appearance");
                egui::Grid::new("theme_grid")
                    .num_columns(2)
                    .spacing(egui::vec2(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label("Theme name");
                        structural |= ui
                            .add(egui::TextEdit::singleline(&mut settings.theme.name))
                            .lost_focus();
                        ui.end_row();
                        theme_dirty |= color_row(ui, "Background", &mut settings.theme.background);
                        theme_dirty |= color_row(ui, "Panel", &mut settings.theme.panel);
                        theme_dirty |= color_row(ui, "Field", &mut settings.theme.field);
                        theme_dirty |= color_row(ui, "Text", &mut settings.theme.text);
                        theme_dirty |= color_row(ui, "Accent", &mut settings.theme.accent);
                        theme_dirty |= color_row(ui, "Accent 2", &mut settings.theme.accent2);
                        theme_dirty |= color_row(ui, "Success", &mut settings.theme.ok);
                        theme_dirty |= color_row(ui, "Warning", &mut settings.theme.warning);
                        theme_dirty |= color_row(ui, "Error", &mut settings.theme.error);
                    });
                if ui.button("Reset colors to Tokyo Night").clicked() {
                    settings.theme = theme;
                    theme_dirty = true;
                }
                ui.separator();

                ui.heading("Fonts");
                egui::Grid::new("font_grid")
                    .num_columns(2)
                    .spacing(egui::vec2(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label("Proportional font file (optional)");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut settings.fonts.proportional_path)
                                .desired_width(430.0),
                        );
                        theme_dirty |= response.lost_focus();
                        ui.end_row();
                        ui.label("Monospace font file (optional)");
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut settings.fonts.monospace_path)
                                .desired_width(430.0),
                        );
                        theme_dirty |= response.lost_focus();
                        ui.end_row();
                        ui.label("Proportional size");
                        theme_dirty |= ui
                            .add(
                                egui::DragValue::new(&mut settings.fonts.proportional_size)
                                    .range(9.0..=28.0)
                                    .speed(0.1),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Monospace size");
                        theme_dirty |= ui
                            .add(
                                egui::DragValue::new(&mut settings.fonts.monospace_size)
                                    .range(9.0..=28.0)
                                    .speed(0.1),
                            )
                            .changed();
                        ui.end_row();
                    });
                ui.separator();

                ui.heading("Data");
                ui.horizontal(|ui| {
                    ui.label("Games per row");
                    structural |= ui
                        .add(egui::DragValue::new(&mut settings.grid_columns).range(1..=12))
                        .changed();
                    ui.separator();
                    ui.label("Backups kept");
                    structural |= ui
                        .add(egui::DragValue::new(&mut settings.backups_kept).range(1..=200))
                        .changed();
                });
                ui.label(format!(
                    "Data folder: {}",
                    crate::paths::data_dir().display()
                ));
                ui.label(format!("Backups: {}", crate::paths::backup_dir().display()));
                ui.separator();
                ui.horizontal(|ui| {
                    save_now = ui.button("Save all now").clicked();
                });
            });
        self.store.settings = settings;
        if theme_dirty {
            self.store.settings.apply(&ctx);
        }
        if save_now {
            self.dirty.settings = true;
            self.dirty.library = true;
        }
        if structural {
            self.dirty.settings = true;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.flush();
    }
}

impl App {
    fn draw(&mut self, ui: &mut egui::Ui) {
        self.pump();
        if let Some(appid) = self.selected
            && self.store.game(appid).is_none()
        {
            self.selected = None;
        }
        egui::Panel::top("ah_tabs").show(ui, |ui| self.tab_bar(ui));
        egui::Panel::bottom("ah_status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.busy {
                    ui.spinner();
                }
                ui.label(self.status.as_str());
            });
        });
        egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
            Tab::Games => self.games_tab(ui),
            Tab::Flow => self.flow_tab(ui),
            Tab::Sort => self.sort_tab(ui),
            Tab::Settings => self.settings_tab(ui),
        });
        self.autosave(ui.ctx());
    }

    #[cfg(test)]
    pub(crate) fn headless(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            store: Store::load(),
            ctx,
            tab: Tab::Games,
            selected: None,
            status: String::new(),
            busy: false,
            tx,
            rx,
            dirty: Dirty::default(),
            save_at: None,
            sort_game: None,
            flow_game: None,
            flow_drag: None,
            flow_link: None,
        }
    }
}

fn hit_node(
    graph: &FlowGraph,
    rect: egui::Rect,
    pointer: egui::Pos2,
    radius: f32,
) -> Option<usize> {
    graph.nodes.iter().position(|node| {
        let center = rect.min + egui::vec2(node.x, node.y);
        (center - pointer).length() <= radius
    })
}

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed = ui.color_edit_button_srgb(color).changed();
    });
    changed
}

fn art_widget(ui: &mut egui::Ui, name: &str, art: &str, size: egui::Vec2) -> egui::Response {
    if art.is_empty() {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let weak = ui.visuals().weak_text_color();
        ui.painter()
            .rect_filled(rect, 6.0, ui.visuals().widgets.inactive.bg_fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            truncate(name, 20),
            egui::FontId::proportional(13.0),
            weak,
        );
        ui.painter().text(
            rect.center() + egui::vec2(0.0, 20.0),
            egui::Align2::CENTER_CENTER,
            "art pending",
            egui::FontId::proportional(11.0),
            weak,
        );
        response
    } else {
        ui.add(
            egui::Image::new(format!("file://{art}"))
                .fit_to_exact_size(size)
                .corner_radius(6.0),
        )
        .interact(egui::Sense::click())
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Achievement;

    fn sample_game() -> Game {
        Game {
            appid: 620,
            name: "Portal 2".to_owned(),
            playtime_minutes: 600,
            installed: true,
            art: "620.jpg".to_owned(),
            achievements: vec![
                Achievement {
                    api_name: "A".to_owned(),
                    name: "Awake".to_owned(),
                    description: "Wake up".to_owned(),
                    unlocked: true,
                    unlock_time: Some(1),
                    global_percent: Some(50.0),
                    icon: String::new(),
                    note: "my note".to_owned(),
                },
                Achievement {
                    api_name: "B".to_owned(),
                    name: "Beat it".to_owned(),
                    description: String::new(),
                    ..Default::default()
                },
            ],
            guides: vec![GuideInfo {
                source: "howlongtobeat".to_owned(),
                url: "https://example.com".to_owned(),
                summary: "10 h".to_owned(),
                main_hours: Some(10.0),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn crucial_every_tab_and_detail_render_without_panicking() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        app.store.games = vec![sample_game()];
        app.store.flow.insert(
            620,
            FlowGraph {
                nodes: vec![
                    FlowNode {
                        api_name: "A".to_owned(),
                        x: 80.0,
                        y: 80.0,
                    },
                    FlowNode {
                        api_name: "B".to_owned(),
                        x: 220.0,
                        y: 170.0,
                    },
                ],
                edges: vec![(0, 1)],
            },
        );
        app.store
            .sort
            .insert(620, vec!["B".to_owned(), "A".to_owned()]);
        app.sort_game = Some(620);
        app.flow_game = Some(620);

        for tab in [Tab::Games, Tab::Flow, Tab::Sort, Tab::Settings] {
            app.tab = tab;
            ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
                .drop_without_applying_deltas();
        }

        app.tab = Tab::Games;
        app.selected = Some(620);
        ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
            .drop_without_applying_deltas();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
