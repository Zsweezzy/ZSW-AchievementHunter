use crate::model::{self, FlowGraph, FlowNode, Game, GameTab, GuideInfo, SortMode, Tab};
use crate::providers;
use crate::steam;
use crate::store::Store;
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

const ICON_SIZE: f32 = 64.0;
const CELL_WIDTH: f32 = 162.0;
const CELL_HEIGHT: f32 = 270.0;

pub struct App {
    store: Store,
    ctx: egui::Context,
    tab: Tab,
    game_tab: GameTab,
    selected: Option<u32>,
    status: String,
    busy: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    dirty: Dirty,
    save_at: Option<Instant>,
    flow_drag: Option<usize>,
    flow_link: Option<usize>,
    auto_library: bool,
    auto_achievements: HashSet<u32>,
    art_requested: HashSet<u32>,
    icons_requested: HashSet<u32>,
    note_edit: Option<String>,
    prefetch: VecDeque<u32>,
    prefetch_busy: bool,
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
            game_tab: GameTab::Achievements,
            selected: None,
            status: String::new(),
            busy: false,
            tx,
            rx,
            dirty: Dirty::default(),
            save_at: None,
            flow_drag: None,
            flow_link: None,
            auto_library: false,
            auto_achievements: HashSet::new(),
            art_requested: HashSet::new(),
            icons_requested: HashSet::new(),
            note_edit: None,
            prefetch: VecDeque::new(),
            prefetch_busy: false,
        };
        app.status = app.ready_status();
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
                    let count = achievements.len();
                    if let Some(game) = self.store.game_mut(appid) {
                        game.achievements = achievements;
                    }
                    self.dirty.library = true;
                    self.busy = false;
                    self.prefetch_busy = false;
                    self.status = if status.is_empty() {
                        format!("Loaded {count} achievements.")
                    } else {
                        status
                    };
                    self.spawn_icons(appid);
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
            .filter(|g| g.art.is_empty() && !self.art_requested.contains(&g.appid))
            .map(|g| (g.appid, g.name.clone()))
            .collect();
        if missing.is_empty() {
            return;
        }
        for (appid, _) in &missing {
            self.art_requested.insert(*appid);
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

    fn spawn_icons(&mut self, appid: u32) {
        if !self.icons_requested.insert(appid) {
            return;
        }
        let jobs: Vec<(String, bool, String)> = self
            .store
            .game(appid)
            .map(|game| {
                game.achievements
                    .iter()
                    .flat_map(|a| {
                        [
                            (a.api_name.clone(), false, a.icon.clone()),
                            (a.api_name.clone(), true, a.icon_gray.clone()),
                        ]
                    })
                    .filter(|(_, _, url)| !url.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if jobs.is_empty() {
            return;
        }
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            for (api_name, gray, url) in jobs {
                let dest = crate::paths::icon_path(appid, &api_name, gray);
                if dest.exists() {
                    continue;
                }
                if crate::http::download(&url, &dest).is_ok() {
                    ctx.request_repaint();
                }
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
            if let Ok(schema) = steam::schema(&settings.steam_api_key, appid) {
                steam::apply_schema(&mut achievements, &schema);
            } else if status.is_empty() {
                status = "Loaded achievements without names: the game schema was unavailable."
                    .to_owned();
            }
            match steam::global_percentages(appid) {
                Ok(percentages) => {
                    for achievement in &mut achievements {
                        achievement.global_percent =
                            percentages.get(&achievement.api_name).copied();
                    }
                }
                Err(_) => {
                    if let Ok(rows) = providers::steamhunters_achievements(appid) {
                        let percentages: std::collections::HashMap<String, f32> = rows
                            .into_iter()
                            .map(|(api, _, _, percent)| (api, percent))
                            .collect();
                        for achievement in &mut achievements {
                            achievement.global_percent =
                                percentages.get(&achievement.api_name).copied();
                        }
                    }
                }
            }
            let _ = tx.send(Msg::Achievements(appid, achievements, status));
            ctx.request_repaint();
        });
    }

    fn prefetch_installed(&mut self) {
        if !self.store.settings.has_steam_credentials() {
            return;
        }
        for game in &self.store.games {
            if game.installed
                && game.has_achievements != Some(false)
                && game.achievements.is_empty()
                && !self.auto_achievements.contains(&game.appid)
            {
                self.auto_achievements.insert(game.appid);
                self.prefetch.push_back(game.appid);
            }
        }
        if self.prefetch_busy || self.busy {
            return;
        }
        if let Some(appid) = self.prefetch.pop_front() {
            self.prefetch_busy = true;
            self.load_achievements(appid);
            self.status = format!("Caching installed games, {} left...", self.prefetch.len());
        }
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
        let style = TabStyle {
            base_fill: ui.visuals().widgets.inactive.bg_fill,
            hover_fill: ui.visuals().widgets.inactive.bg_fill.linear_multiply(1.6),
            active_fill: ui.visuals().panel_fill,
            text: ui.visuals().weak_text_color(),
            active_text: ui.visuals().text_color(),
            accent: ui.visuals().hyperlink_color,
            radius: egui::CornerRadius::same(8),
            active_radius: egui::CornerRadius {
                nw: 9,
                ne: 9,
                sw: 0,
                se: 0,
            },
            height: 36.0,
            accent_line: true,
        };
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for tab in Tab::ALL {
                if paint_tab(ui, &style, tab.label(), self.tab == tab).clicked() {
                    clicked = Some(tab);
                }
            }
        });
        if let Some(tab) = clicked {
            self.tab = tab;
        }
    }

    fn sub_tab_bar(&mut self, ui: &mut egui::Ui) {
        let style = TabStyle {
            base_fill: egui::Color32::TRANSPARENT,
            hover_fill: ui.visuals().widgets.hovered.weak_bg_fill,
            active_fill: ui.visuals().widgets.inactive.bg_fill,
            text: ui.visuals().weak_text_color(),
            active_text: ui.visuals().text_color(),
            accent: ui.visuals().hyperlink_color,
            radius: egui::CornerRadius::same(7),
            active_radius: egui::CornerRadius::same(7),
            height: 30.0,
            accent_line: false,
        };
        let mut clicked = None;
        ui.horizontal(|ui| {
            for tab in GameTab::ALL {
                if paint_tab(ui, &style, tab.label(), self.game_tab == tab).clicked() {
                    clicked = Some(tab);
                }
            }
        });
        if let Some(tab) = clicked {
            self.game_tab = tab;
        }
    }

    fn games_tab(&mut self, ui: &mut egui::Ui) {
        let settings = self.store.settings.clone();
        if !self.auto_library && self.store.games.is_empty() && settings.has_steam_credentials() {
            self.auto_library = true;
            self.refresh();
        }
        self.spawn_art();
        self.prefetch_installed();
        let filters = model::Filters {
            installed_only: settings.installed_only,
            hide_no_achievements: settings.hide_no_achievements,
            hide_completed: settings.hide_completed,
            hide_unstarted: settings.hide_unstarted,
        };
        let sort_mode = settings.sort_mode;
        let total = self.store.games.len();
        let shown = self
            .store
            .games
            .iter()
            .filter(|g| model::visible(g, filters))
            .count();
        let mut refresh = false;
        let mut filters_changed = false;
        ui.horizontal_wrapped(|ui| {
            refresh = ui.button("Refresh library").clicked();
            ui.separator();
            ui.label(format!("{shown} / {total} games"));
            ui.separator();
            filters_changed |= ui
                .checkbox(&mut self.store.settings.installed_only, "Installed only")
                .changed();
            filters_changed |= ui
                .checkbox(
                    &mut self.store.settings.hide_no_achievements,
                    "Has achievements",
                )
                .changed();
            filters_changed |= ui
                .checkbox(&mut self.store.settings.hide_completed, "Hide 100%")
                .on_hover_text("Hide games with every achievement unlocked")
                .changed();
            filters_changed |= ui
                .checkbox(&mut self.store.settings.hide_unstarted, "Hide 0%")
                .on_hover_text("Hide games with no achievement unlocked yet")
                .changed();
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Guides from:");
            filters_changed |= ui
                .checkbox(&mut self.store.settings.guide_hltb, "HowLongToBeat")
                .changed();
            filters_changed |= ui
                .checkbox(&mut self.store.settings.guide_steamhunters, "SteamHunters")
                .changed();
        });
        if filters_changed {
            self.dirty.settings = true;
        }
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
        let cells: Vec<(u32, String, String, String, u64)> = self
            .store
            .games
            .iter()
            .filter(|g| model::visible(g, filters))
            .map(|g| {
                let progress = if !g.achievements.is_empty() {
                    format!(
                        "{} / {} achievements",
                        g.unlocked_count(),
                        g.achievements.len()
                    )
                } else if g.has_achievements == Some(false) {
                    "no achievements".to_owned()
                } else {
                    "achievements not synced".to_owned()
                };
                (
                    g.appid,
                    g.name.clone(),
                    g.art.clone(),
                    progress,
                    g.last_played,
                )
            })
            .collect();
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(14.0, 16.0);
                    for (appid, name, art, progress, last_played) in &cells {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(CELL_WIDTH, CELL_HEIGHT),
                            egui::Sense::click(),
                        );
                        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if response.clicked() {
                            clicked = Some(*appid);
                        }
                        let mut cell = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        cell.spacing_mut().item_spacing = egui::vec2(0.0, 6.0);
                        let art = art_widget(&mut cell, name, art, egui::vec2(150.0, 200.0));
                        if response.hovered() {
                            cell.painter().rect_filled(
                                art.rect,
                                6.0,
                                cell.visuals().selection.bg_fill.gamma_multiply(0.25),
                            );
                        }
                        cell.add(egui::Label::new(name.as_str()).truncate());
                        cell.add(
                            egui::Label::new(egui::RichText::new(progress.as_str()).weak())
                                .truncate(),
                        );
                        if sort_mode == SortMode::LastPlayed {
                            cell.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "played {}",
                                        model::humanize_ago(*last_played)
                                    ))
                                    .weak(),
                                )
                                .truncate(),
                            );
                        }
                    }
                });
            });
        if let Some(appid) = clicked {
            self.selected = Some(appid);
            self.game_tab = GameTab::Achievements;
        }
    }

    fn game_detail(&mut self, ui: &mut egui::Ui, appid: u32) {
        if !self.auto_achievements.contains(&appid)
            && self.store.settings.has_steam_credentials()
            && self
                .store
                .game(appid)
                .map(|game| game.achievements.is_empty())
                .unwrap_or(false)
        {
            self.auto_achievements.insert(appid);
            self.load_achievements(appid);
        }
        let mut back = false;
        let mut load = false;
        let mut guides = false;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("< Back").clicked();
            if let Some(game) = self.store.game(appid) {
                ui.heading(game.name.as_str());
                let progress = if !game.achievements.is_empty() {
                    format!(
                        "{} / {} achievements ({:.0}%)",
                        game.unlocked_count(),
                        game.achievements.len(),
                        game.completion_percent()
                    )
                } else if game.has_achievements == Some(false) {
                    "no achievements".to_owned()
                } else {
                    "achievements not synced".to_owned()
                };
                ui.label(format!(
                    "{progress} \u{2022} {} hours \u{2022} played {}",
                    game.playtime_minutes / 60,
                    model::humanize_ago(game.last_played)
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
        let mut filters_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Show:");
            filters_changed |= ui
                .checkbox(&mut self.store.settings.filter_locked, "Locked")
                .changed();
            filters_changed |= ui
                .checkbox(&mut self.store.settings.filter_unlocked, "Unlocked")
                .changed();
            ui.separator();
            ui.weak("Right-click an achievement to add a note.");
        });
        if filters_changed {
            self.dirty.settings = true;
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
        ui.add_space(6.0);
        self.sub_tab_bar(ui);
        ui.separator();
        match self.game_tab {
            GameTab::Achievements => self.achievement_list(ui, appid),
            GameTab::Flow => self.flow_tab(ui, appid),
            GameTab::Sort => self.sort_tab(ui, appid),
        }
    }

    fn achievement_list(&mut self, ui: &mut egui::Ui, appid: u32) {
        let (show_locked, show_unlocked) = (
            self.store.settings.filter_locked,
            self.store.settings.filter_unlocked,
        );
        let mut touched = false;
        let mut close_edit = false;
        let mut note_edit = self.note_edit.clone();
        let Some(game) = self.store.game_mut(appid) else {
            return;
        };
        if game.achievements.is_empty() {
            ui.label("No achievement data cached yet. Press Load achievements.");
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for achievement in &mut game.achievements {
                    if !model::passes_filter(achievement, show_locked, show_unlocked) {
                        continue;
                    }
                    let editing = note_edit.as_deref() == Some(achievement.api_name.as_str());
                    let mut icon_rect = None;
                    let row = egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(0, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ICON_SIZE, ICON_SIZE),
                                    egui::Sense::hover(),
                                );
                                icon_rect = Some(rect);
                                ui.vertical(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(achievement.name.as_str());
                                        if achievement.hidden {
                                            ui.colored_label(ui.visuals().warn_fg_color, "hidden");
                                        }
                                        if achievement.unlocked {
                                            ui.weak("\u{2714} unlocked");
                                        }
                                        if let Some(percent) = achievement.global_percent {
                                            ui.weak(format!("{percent:.1}% of players"));
                                        }
                                    });
                                    if !achievement.description.is_empty() {
                                        ui.label(achievement.description.as_str());
                                    }
                                    if editing {
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut achievement.note)
                                                    .hint_text("note")
                                                    .desired_width(f32::INFINITY),
                                            )
                                            .lost_focus()
                                        {
                                            touched = true;
                                            if achievement.note.is_empty() {
                                                close_edit = true;
                                            }
                                        }
                                    } else if !achievement.note.is_empty() {
                                        ui.weak(achievement.note.as_str());
                                    }
                                });
                            })
                        });
                    if let Some(rect) = icon_rect {
                        let centered = egui::Rect::from_center_size(
                            egui::pos2(rect.center().x, row.response.rect.center().y),
                            egui::vec2(ICON_SIZE, ICON_SIZE),
                        );
                        achievement_icon(ui, appid, achievement, centered);
                    }
                    let response = row.response.interact(egui::Sense::click());
                    response.context_menu(|ui| {
                        if ui.button("Add / edit note").clicked() {
                            note_edit = Some(achievement.api_name.clone());
                            ui.close();
                        }
                        if ui.button("Clear note").clicked() {
                            achievement.note.clear();
                            touched = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                }
            });
        self.note_edit = if close_edit { None } else { note_edit };
        if touched {
            self.dirty.library = true;
        }
    }

    fn sort_tab(&mut self, ui: &mut egui::Ui, appid: u32) {
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

    fn flow_tab(&mut self, ui: &mut egui::Ui, appid: u32) {
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
                ui.horizontal_wrapped(|ui| {
                    ui.label("Library sort:");
                    for mode in SortMode::ALL {
                        let active = settings.sort_mode == mode;
                        if ui.selectable_label(active, mode.label()).clicked() && !active {
                            settings.sort_mode = mode;
                            model::sort_games(&mut self.store.games, mode);
                            self.dirty.settings = true;
                            self.dirty.library = true;
                        }
                    }
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
                    ui.label("Interface scale");
                    structural |= ui
                        .add(
                            egui::Slider::new(&mut settings.ui_scale, 0.75..=2.0)
                                .fixed_decimals(2),
                        )
                        .changed();
                    if ui.button("Reset").clicked() {
                        settings.ui_scale = 1.0;
                        theme_dirty = true;
                    }
                    ui.separator();
                    ui.label("Backups kept").on_hover_text(
                        "Before every save the app copies the previous settings.toml / library.json / \
                         flow.json / sort.json into the backups folder and keeps the newest N.",
                    );
                    structural |= ui
                        .add(egui::DragValue::new(&mut settings.backups_kept).range(1..=200))
                        .changed();
                });
                ui.label("The game grid fills the window and adds columns as it widens.");
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
        egui::Panel::top("ah_tabs")
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().widgets.inactive.bg_fill)
                    .inner_margin(egui::Margin {
                        left: 10,
                        right: 10,
                        top: 6,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.tab_bar(ui));
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
            game_tab: GameTab::Achievements,
            selected: None,
            status: String::new(),
            busy: false,
            tx,
            rx,
            dirty: Dirty::default(),
            save_at: None,
            flow_drag: None,
            flow_link: None,
            auto_library: false,
            auto_achievements: HashSet::new(),
            art_requested: HashSet::new(),
            icons_requested: HashSet::new(),
            note_edit: None,
            prefetch: VecDeque::new(),
            prefetch_busy: false,
        }
    }
}

struct TabStyle {
    base_fill: egui::Color32,
    hover_fill: egui::Color32,
    active_fill: egui::Color32,
    text: egui::Color32,
    active_text: egui::Color32,
    accent: egui::Color32,
    radius: egui::CornerRadius,
    active_radius: egui::CornerRadius,
    height: f32,
    accent_line: bool,
}

fn paint_tab(ui: &mut egui::Ui, style: &TabStyle, label: &str, active: bool) -> egui::Response {
    let font = egui::FontId::proportional(15.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), style.text);
    let size = egui::vec2((galley.size().x + 40.0).max(104.0), style.height);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if active {
        style.active_fill
    } else if response.hovered() {
        style.hover_fill
    } else {
        style.base_fill
    };
    let radius = if active {
        style.active_radius
    } else {
        style.radius
    };
    ui.painter().rect_filled(rect, radius, fill);
    if active && style.accent_line {
        ui.painter().line_segment(
            [
                rect.left_top() + egui::vec2(6.0, 1.0),
                rect.right_top() + egui::vec2(-6.0, 1.0),
            ],
            egui::Stroke::new(2.0, style.accent),
        );
    }
    let color = if active {
        style.active_text
    } else {
        style.text
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

fn achievement_icon(
    ui: &mut egui::Ui,
    appid: u32,
    achievement: &crate::model::Achievement,
    rect: egui::Rect,
) {
    let gray = !achievement.unlocked;
    let path = crate::paths::icon_path(appid, &achievement.api_name, gray);
    if path.exists() {
        ui.put(
            rect,
            egui::Image::new(format!("file://{}", path.display()))
                .fit_to_exact_size(egui::vec2(ICON_SIZE, ICON_SIZE))
                .corner_radius(6.0),
        );
        return;
    }
    ui.painter()
        .rect_filled(rect, 6.0, ui.visuals().widgets.inactive.bg_fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if achievement.unlocked {
            "\u{2714}"
        } else {
            "?"
        },
        egui::FontId::proportional(22.0),
        ui.visuals().weak_text_color(),
    );
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
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
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
                    icon_gray: String::new(),
                    hidden: false,
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
            ..Default::default()
        }
    }

    #[test]
    fn crucial_library_grid_cells_are_flush() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };
        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        let named = |appid: u32, name: &str| Game {
            appid,
            name: name.to_owned(),
            installed: true,
            has_achievements: Some(true),
            achievements: vec![Achievement {
                api_name: "A".to_owned(),
                name: "A".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        };
        app.store.games = vec![
            named(1, "A"),
            named(2, "Portal 2"),
            named(3, "Tom Clancy's Rainbow Six Siege"),
            named(4, "ELDEN RING"),
            named(5, "The Witcher 3: Wild Hunt"),
            named(6, "Hades"),
        ];
        app.tab = Tab::Games;
        app.store.settings.sort_mode = SortMode::LastPlayed;
        model::sort_games(&mut app.store.games, SortMode::LastPlayed);

        let screen = egui::vec2(1000.0, 800.0);
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        let mut cells: Vec<(f32, f32)> = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - 150.0).abs() < 0.5
                        && (r.rect.height() - 200.0).abs() < 0.5 =>
                {
                    Some((r.rect.min.x, r.rect.min.y))
                }
                _ => None,
            })
            .collect();
        out.drop_without_applying_deltas();
        assert_eq!(cells.len(), 6, "one artwork placeholder per game");

        cells.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        let mut rows: Vec<(f32, Vec<f32>)> = Vec::new();
        for (x, y) in cells {
            match rows.last_mut() {
                Some((row_y, xs)) if (y - *row_y).abs() < 0.5 => xs.push(x),
                _ => rows.push((y, vec![x])),
            }
        }
        assert!(
            rows.len() >= 2,
            "expected a wrapped grid at 1000 px: {rows:?}"
        );

        let col_pitch = CELL_WIDTH + 14.0;
        let row_pitch = CELL_HEIGHT + 16.0;
        let first_x = rows[0].1[0];
        for (row, (y, xs)) in rows.iter().enumerate() {
            let expected_y = rows[0].0 + row as f32 * row_pitch;
            assert!(
                (y - expected_y).abs() < 0.5,
                "row {row} starts at y={y}, expected {expected_y}: {rows:?}"
            );
            for (col, x) in xs.iter().enumerate() {
                let expected_x = first_x + col as f32 * col_pitch;
                assert!(
                    (x - expected_x).abs() < 0.5,
                    "cell ({row},{col}) starts at x={x}, expected {expected_x}: {rows:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_library_cell_stays_hoverable_over_the_artwork() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-hover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };
        let ctx = egui::Context::default();
        let screen = egui::vec2(1000.0, 800.0);
        let raw = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
            events,
            ..Default::default()
        };
        let art_size = egui::vec2(150.0, 200.0);

        // The whole cell is the click target, so the artwork must never sense clicks:
        // an interactive widget becomes the topmost hit and egui then hovers *it*
        // instead of the cell, which killed the highlight over the art.
        ctx.run_ui(raw(Vec::new()), |ui| {
            assert!(
                !art_widget(ui, "Portal 2", "", art_size)
                    .sense
                    .senses_click(),
                "the art placeholder must not be a click target"
            );
            assert!(
                !art_widget(ui, "Portal 2", "620.jpg", art_size)
                    .sense
                    .senses_click(),
                "the artwork image must not be a click target"
            );
        })
        .drop_without_applying_deltas();

        let mut app = App::headless(ctx.clone());
        app.store.games = vec![Game {
            appid: 1,
            name: "Portal 2".to_owned(),
            installed: true,
            has_achievements: Some(true),
            achievements: vec![Achievement {
                api_name: "A".to_owned(),
                name: "A".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        }];
        app.tab = Tab::Games;
        let art_rects = |out: &egui::FullOutput| -> Vec<egui::Rect> {
            out.shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Rect(r) if r.rect.size() == art_size => Some(r.rect),
                    _ => None,
                })
                .collect()
        };

        let out = ctx.run_ui(raw(Vec::new()), |ui| app.draw(ui));
        let art = art_rects(&out);
        out.drop_without_applying_deltas();
        assert_eq!(art.len(), 1, "expected one artwork placeholder rect");

        let out = ctx.run_ui(raw(Vec::new()), |ui| app.draw(ui));
        assert_eq!(
            art_rects(&out),
            art,
            "no highlight while the pointer is away"
        );
        out.drop_without_applying_deltas();

        let out = ctx.run_ui(
            raw(vec![egui::Event::PointerMoved(art[0].center())]),
            |ui| app.draw(ui),
        );
        assert_eq!(
            art_rects(&out),
            vec![art[0], art[0]],
            "hovering the artwork must tint it (a second rect at the art position)"
        );
        out.drop_without_applying_deltas();

        let out = ctx.run_ui(
            raw(vec![egui::Event::PointerMoved(egui::pos2(900.0, 700.0))]),
            |ui| app.draw(ui),
        );
        assert_eq!(
            art_rects(&out),
            art,
            "the highlight must clear when the pointer leaves"
        );
        out.drop_without_applying_deltas();
        let _ = std::fs::remove_dir_all(&dir);
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

        for tab in Tab::ALL {
            app.tab = tab;
            ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
                .drop_without_applying_deltas();
        }

        app.tab = Tab::Games;
        app.selected = Some(620);
        for game_tab in GameTab::ALL {
            app.game_tab = game_tab;
            ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
                .drop_without_applying_deltas();
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
