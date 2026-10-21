use crate::model::{
    self, Achievement, AchievementSort, FlowGraph, FlowNode, Game, GameTab, GuideInfo, SortMode,
    Tab, Tag,
};
use crate::page;
use crate::providers;
use crate::sotf;
use crate::steam;
use crate::store::Store;
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

const ICON_SIZE: f32 = 64.0;
const SMALL_ICON_SIZE: f32 = 32.0;
/// Guides longer than this start folded, so one long list cannot dominate the list.
const GUIDE_FOLD_LINES: usize = 8;
const SMALL_ITEM_WIDTH: f32 = 420.0;
const FLOW_CARD_WIDTH: f32 = 240.0;
const FLOW_CARD_MIN_HEIGHT: f32 = 44.0;
const FLOW_CARD_GAP: f32 = 20.0;
const FLOW_ROW_SPACING: f32 = 120.0;
const CELL_WIDTH: f32 = 124.0;
const CELL_HEIGHT: f32 = 232.0;
const ART_HEIGHT: f32 = 150.0;
const ART_WIDTH: f32 = ART_HEIGHT * 2.0 / 3.0;

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
    /// Wall clock of the last periodic refresh; the timer restarts with the app.
    last_auto_refresh: Instant,
    /// Selected game whose achievements reload once the periodic library pull lands.
    reload_after_refresh: Option<u32>,
    /// api_names whose long guide the user expanded this session.
    expanded: HashSet<String>,
    detail_edit: Option<String>,
    page_open: Option<(u32, String, String)>,
    page_html: String,
    page_edit: Option<(u32, String)>,
    page_draft: String,
}

#[derive(Clone, Copy, Default)]
struct Dirty {
    settings: bool,
    library: bool,
    flow: bool,
    sort: bool,
    checks: bool,
}

impl Dirty {
    fn any(self) -> bool {
        self.settings || self.library || self.flow || self.sort || self.checks
    }
}

enum Msg {
    Games(Vec<Game>, String),
    Achievements(u32, Vec<crate::model::Achievement>, String),
    NoAchievements(u32),
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
            last_auto_refresh: Instant::now(),
            reload_after_refresh: None,
            expanded: HashSet::new(),
            detail_edit: None,
            page_open: None,
            page_html: String::new(),
            page_edit: None,
            page_draft: String::new(),
        };
        app.status = app.ready_status();
        app.attach_all_guide_slices();
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
                    if let Some(appid) = self.reload_after_refresh.take()
                        && self.store.game(appid).is_some()
                    {
                        self.load_achievements(appid);
                    }
                }
                Msg::Achievements(appid, achievements, status) => {
                    let count = achievements.len();
                    if let Some(game) = self.store.game_mut(appid) {
                        game.achievements = achievements;
                    }
                    self.attach_guide_slices(appid);
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
                Msg::NoAchievements(appid) => {
                    if let Some(game) = self.store.game_mut(appid) {
                        game.achievements.clear();
                        game.has_achievements = Some(false);
                    }
                    self.dirty.library = true;
                    self.busy = false;
                    self.prefetch_busy = false;
                    self.status = "This game has no achievements.".to_owned();
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

    fn auto_refresh(&mut self, ctx: &egui::Context) {
        let minutes = self.store.settings.auto_refresh_minutes;
        let Some(delay) = refresh_delay(self.last_auto_refresh.elapsed(), minutes) else {
            return;
        };
        if !delay.is_zero() {
            ctx.request_repaint_after(delay);
            return;
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
            return;
        }
        self.last_auto_refresh = Instant::now();
        ctx.request_repaint_after(std::time::Duration::from_secs(minutes * 60));
        if self.selected.is_some() && self.store.settings.has_steam_credentials() {
            self.reload_after_refresh = self.selected;
        }
        if self.store.game(sotf::APPID).is_some() {
            self.scan_sotf(sotf::APPID);
        }
        self.refresh();
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
            let mut achievements =
                match steam::achievements(&settings.steam_api_key, &settings.steam_id, appid) {
                    Ok(achievements) => achievements,
                    Err(error) if steam::no_stats_error(&error) => {
                        let _ = tx.send(Msg::NoAchievements(appid));
                        ctx.request_repaint();
                        return;
                    }
                    Err(error) => {
                        let _ = tx.send(Msg::Achievements(
                            appid,
                            Vec::new(),
                            format!("Achievements unavailable: {error}"),
                        ));
                        ctx.request_repaint();
                        return;
                    }
                };
            let mut status = String::new();
            if let Ok(schema) = steam::schema(&settings.steam_api_key, appid) {
                steam::apply_schema(&mut achievements, &schema);
            } else {
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
        if self.dirty.checks {
            let _ = self.store.save_checks();
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
        if let Some(appid) = self.selected {
            self.game_detail(ui, appid);
            return;
        }
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
            ui.label(format!("{shown} / {total} games"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("\u{2630}", |ui| {
                    if ui.button("Refresh library").clicked() {
                        refresh = true;
                        ui.close();
                    }
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
                    ui.separator();
                    for source in providers::GUIDE_SOURCES {
                        let mut on = *((source.toggle)(&mut self.store.settings));
                        if ui
                            .checkbox(&mut on, format!("{} guides", source.label))
                            .changed()
                        {
                            *((source.toggle)(&mut self.store.settings)) = on;
                            filters_changed = true;
                        }
                    }
                });
            });
        });
        if filters_changed {
            self.dirty.settings = true;
        }
        if refresh {
            self.refresh();
        }
        ui.separator();
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
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
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
                        let art =
                            art_widget(&mut cell, name, art, egui::vec2(ART_WIDTH, ART_HEIGHT));
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
                .map(|game| game.achievements.is_empty() && game.has_achievements != Some(false))
                .unwrap_or(false)
        {
            self.auto_achievements.insert(appid);
            self.load_achievements(appid);
        }
        let header = self.store.game(appid).map(|game| {
            let progress = if !game.achievements.is_empty() {
                let left = game.achievements.len() - game.unlocked_count();
                let left = if left == 0 {
                    String::new()
                } else {
                    format!(" \u{2022} {left} left")
                };
                format!(
                    "{} / {} achievements ({:.0}%){left}",
                    game.unlocked_count(),
                    game.achievements.len(),
                    game.completion_percent()
                )
            } else if game.has_achievements == Some(false) {
                "no achievements".to_owned()
            } else {
                "achievements not synced".to_owned()
            };
            let detail = format!(
                "{progress} \u{2022} {} hours \u{2022} played {}",
                game.playtime_minutes / 60,
                model::humanize_ago(game.last_played)
            );
            let guides: Vec<(String, String, String)> = game
                .guides
                .iter()
                .map(|guide| {
                    (
                        guide.source.clone(),
                        guide.summary.clone(),
                        guide.url.clone(),
                    )
                })
                .collect();
            (game.name.clone(), detail, guides)
        });
        let mut back = false;
        let mut load = false;
        let mut guides = false;
        let mut filters_changed = false;
        let mut open: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            back = ui.button("\u{2039}").clicked();
            let Some((name, detail, _)) = header.as_ref() else {
                return;
            };
            ui.heading(name.as_str());
            ui.label(detail.as_str());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("\u{2630}", |ui| {
                    if ui.button("Load achievements").clicked() {
                        load = true;
                        ui.close();
                    }
                    if ui.button("Fetch guides").clicked() {
                        guides = true;
                        ui.close();
                    }
                    ui.separator();
                    filters_changed |= ui
                        .checkbox(&mut self.store.settings.filter_locked, "Show locked")
                        .changed();
                    filters_changed |= ui
                        .checkbox(&mut self.store.settings.filter_unlocked, "Show unlocked")
                        .changed();
                    let current = self.store.settings.tag_filter;
                    ui.menu_button(
                        format!("Tag: {}", current.map_or("any", |tag| tag.label())),
                        |ui| {
                            if ui.button("any").clicked() {
                                self.store.settings.tag_filter = None;
                                filters_changed = true;
                                ui.close();
                            }
                            for tag in Tag::ALL {
                                let active = current == Some(tag);
                                if ui.selectable_label(active, tag.label()).clicked() && !active {
                                    self.store.settings.tag_filter = Some(tag);
                                    filters_changed = true;
                                    ui.close();
                                }
                            }
                        },
                    );
                    ui.separator();
                    if ui.button("PSNProfiles guide").clicked() {
                        open = Some(
                            providers::local_psnprofiles_url(appid)
                                .unwrap_or_else(|| psnprofiles_guide_url(name)),
                        );
                        ui.close();
                    }
                    for (label, url) in providers::extra_guide_links(name, appid) {
                        if ui.button(format!("{label} search")).clicked() {
                            open = Some(url);
                            ui.close();
                        }
                    }
                });
            });
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
        if filters_changed {
            self.dirty.settings = true;
        }
        if let Some((_, _, guides)) = header {
            for (source, summary, url) in guides {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("[{source}]"));
                    ui.label(summary);
                    if ui.link("open").clicked() {
                        open = Some(url.clone());
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

    /// The Guide tab is gone; keep the one thing it owned that the Achievements tab lacks.
    fn import_guide_button(&mut self, ui: &mut egui::Ui) {
        let Some(path) = providers::psnprofiles_import_path() else {
            return;
        };
        if ui
            .button(format!("Import guide text from {}", path.display()))
            .clicked()
        {
            match providers::import_psnprofiles(&path, &self.store.games) {
                Ok((written, skipped)) => {
                    self.status =
                        format!("Imported {written} PSNProfiles guides ({skipped} skipped).");
                    self.attach_all_guide_slices();
                    self.dirty.library = true;
                }
                Err(error) => self.status = format!("Import failed: {error}"),
            }
        }
    }

    fn attach_all_guide_slices(&mut self) {
        for appid in self
            .store
            .games
            .iter()
            .map(|game| game.appid)
            .collect::<Vec<_>>()
        {
            self.attach_guide_slices(appid);
        }
    }

    fn attach_guide_slices(&mut self, appid: u32) {
        let Some(game) = self.store.game_mut(appid) else {
            return;
        };
        if game.achievements.is_empty() {
            return;
        }
        let Some((_, markdown)) = providers::local_psnprofiles_guide(appid) else {
            for achievement in &mut game.achievements {
                achievement.guide.clear();
            }
            return;
        };
        let slices = guide_slices(&markdown, &game.achievements);
        for (achievement, slice) in game.achievements.iter_mut().zip(slices) {
            achievement.guide = slice;
        }
    }

    /// Tick the guide checklists the save already answers, for Sons Of The Forest.
    fn scan_sotf(&mut self, appid: u32) {
        let guides: Vec<(String, String)> = self
            .store
            .game(appid)
            .map(|game| {
                game.achievements
                    .iter()
                    .filter(|a| !a.guide.is_empty())
                    .map(|a| (a.api_name.clone(), a.guide.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let found = match sotf::scan() {
            Ok(found) => found,
            Err(error) => {
                self.status = format!("Save scan failed: {error}");
                return;
            }
        };
        let checks = self.store.checks.entry(appid).or_default();
        let mut ticked = 0usize;
        for (api_name, guide) in guides {
            let done = checks.entry(api_name).or_default();
            for key in save_tick_lines(&guide, &found) {
                if done.insert(key) {
                    ticked += 1;
                }
            }
        }
        if ticked > 0 {
            self.dirty.checks = true;
        }
        self.status = format!("Save scan: ticked {ticked} checklist items.");
    }

    fn achievement_list(&mut self, ui: &mut egui::Ui, appid: u32) {
        // Never call local_psnprofiles_guide here: it reads and parses the whole guide
        // file, and this runs every frame. Sliced guide text is the cheap equivalent.
        let needs_import = self
            .store
            .game(appid)
            .is_none_or(|game| game.achievements.iter().all(|a| a.guide.is_empty()));
        if needs_import {
            self.import_guide_button(ui);
            ui.separator();
        }
        if appid == sotf::APPID && ui.button("Scan Sons Of The Forest save").clicked() {
            self.scan_sotf(appid);
        }
        let mut sort_changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Sort:");
            for mode in AchievementSort::ALL {
                let active = self.store.settings.achievement_sort == mode;
                if ui.selectable_label(active, mode.label()).clicked() && !active {
                    self.store.settings.achievement_sort = mode;
                    sort_changed = true;
                }
            }
        });
        if sort_changed {
            self.dirty.settings = true;
        }
        ui.label(
            egui::RichText::new("Right-click an achievement name for tags, time and its page.")
                .weak()
                .small(),
        );
        let sort = self.store.settings.achievement_sort;
        let (show_locked, show_unlocked, tag_filter) = (
            self.store.settings.filter_locked,
            self.store.settings.filter_unlocked,
            self.store.settings.tag_filter,
        );
        let mut touched = false;
        let mut details_touched = false;
        let mut close_edit = false;
        let mut note_edit = self.note_edit.clone();
        let mut detail_edit = self.detail_edit.clone();
        let mut checks = self.store.checks.get(&appid).cloned().unwrap_or_default();
        let mut expanded = self.expanded.clone();
        let Some(game) = self.store.game_mut(appid) else {
            return;
        };
        if game.achievements.is_empty() {
            ui.label("No achievement data cached yet. Press Load achievements.");
            return;
        }
        let order = model::achievement_order(&game.achievements, sort);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Without this a long unbreakable run (a guide's "-----" rule) widens the
                // scroll content, and the row separators draw past the viewport edge.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                for index in order {
                    let achievement = &mut game.achievements[index];
                    if !model::passes_filter(achievement, show_locked, show_unlocked, tag_filter) {
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
                                    // Give the text column a definite width so wrapped labels
                                    // report their real height; otherwise the row under-measures
                                    // and the next row paints on top of it.
                                    ui.set_width(ui.available_width());
                                    let mut title = None;
                                    ui.horizontal_wrapped(|ui| {
                                        title = Some(ui.strong(achievement.name.as_str()));
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
                                    if let Some(title) = title {
                                        title.context_menu(|ui| {
                                            if ui.button("Add / edit note").clicked() {
                                                note_edit = Some(achievement.api_name.clone());
                                                ui.close();
                                            }
                                            if ui.button("Edit details").clicked() {
                                                detail_edit = Some(achievement.api_name.clone());
                                                ui.close();
                                            }
                                            if ui.button("Page").clicked() {
                                                self.page_html =
                                                    page::load(appid, &achievement.api_name);
                                                self.page_open = Some((
                                                    appid,
                                                    achievement.api_name.clone(),
                                                    achievement.name.clone(),
                                                ));
                                                ui.close();
                                            }
                                            if ui.button("Clear note").clicked() {
                                                achievement.note.clear();
                                                touched = true;
                                                ui.close();
                                            }
                                        });
                                    }
                                    if !achievement.description.is_empty() {
                                        ui.label(achievement.description.as_str());
                                    }
                                    if !achievement.mission.is_empty()
                                        || achievement.typical_minutes > 0
                                        || !achievement.tags.is_empty()
                                    {
                                        ui.horizontal_wrapped(|ui| {
                                            if !achievement.mission.is_empty() {
                                                ui.weak(achievement.mission.as_str());
                                            }
                                            if achievement.typical_minutes > 0 {
                                                ui.weak(hours_label(achievement.typical_minutes));
                                            }
                                            for tag in &achievement.tags {
                                                ui.colored_label(tag_color(ui, *tag), tag.label());
                                            }
                                        });
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
                                    if detail_edit.as_deref() == Some(achievement.api_name.as_str())
                                    {
                                        ui.add_space(2.0);
                                        egui::Grid::new(("details", &achievement.api_name))
                                            .num_columns(2)
                                            .min_col_width(72.0)
                                            .spacing([8.0, 4.0])
                                            .show(ui, |ui| {
                                                ui.label("Mission");
                                                details_touched |= ui
                                                    .add(
                                                        egui::TextEdit::singleline(
                                                            &mut achievement.mission,
                                                        )
                                                        .hint_text("chapter / mission")
                                                        .desired_width(320.0),
                                                    )
                                                    .changed();
                                                ui.end_row();
                                                ui.label("Typical time");
                                                let mut hours =
                                                    achievement.typical_minutes as f32 / 60.0;
                                                if ui
                                                    .add(
                                                        egui::DragValue::new(&mut hours)
                                                            .range(0.0..=999.0)
                                                            .speed(0.25)
                                                            .suffix(" h"),
                                                    )
                                                    .changed()
                                                {
                                                    achievement.typical_minutes =
                                                        (hours * 60.0).round() as u32;
                                                    details_touched = true;
                                                }
                                                ui.end_row();
                                                ui.label("Tags");
                                                ui.horizontal_wrapped(|ui| {
                                                    for tag in Tag::ALL {
                                                        let mut on =
                                                            achievement.tags.contains(&tag);
                                                        if ui
                                                            .checkbox(&mut on, tag.label())
                                                            .changed()
                                                        {
                                                            if on {
                                                                achievement.tags.push(tag);
                                                            } else {
                                                                achievement
                                                                    .tags
                                                                    .retain(|kept| *kept != tag);
                                                            }
                                                            details_touched = true;
                                                        }
                                                    }
                                                });
                                                ui.end_row();
                                            });
                                    }
                                    if !achievement.guide.is_empty() {
                                        ui.add_space(2.0);
                                        let total = achievement.guide.lines().count();
                                        let long = total > GUIDE_FOLD_LINES;
                                        let folded =
                                            long && !expanded.contains(&achievement.api_name);
                                        if long {
                                            let label = if folded {
                                                format!("Show guide ({total} lines)")
                                            } else {
                                                format!("Hide guide ({total} lines)")
                                            };
                                            if ui.small_button(label).clicked() {
                                                if folded {
                                                    expanded.insert(achievement.api_name.clone());
                                                } else {
                                                    expanded.remove(&achievement.api_name);
                                                }
                                            }
                                        }
                                        if !folded {
                                            let checkable = checklist_lines(&achievement.guide);
                                            let done = checks
                                                .entry(achievement.api_name.clone())
                                                .or_default();
                                            ui.vertical(|ui| {
                                                for line in achievement.guide.lines() {
                                                    if line.trim().is_empty() {
                                                        ui.add_space(2.0);
                                                        continue;
                                                    }
                                                    if checkable.contains(line) {
                                                        let key = normalize_match(line);
                                                        let mut on = done.contains(&key);
                                                        let text =
                                                            egui::RichText::new(line).italics();
                                                        if ui.checkbox(&mut on, text).changed() {
                                                            if on {
                                                                done.insert(key);
                                                            } else {
                                                                done.remove(&key);
                                                            }
                                                            touched = true;
                                                        }
                                                    } else {
                                                        ui.label(
                                                            egui::RichText::new(line).italics(),
                                                        );
                                                    }
                                                }
                                            });
                                        }
                                    }
                                });
                            })
                        });
                    if let Some(rect) = icon_rect {
                        let centered = egui::Rect::from_center_size(
                            egui::pos2(rect.center().x, row.response.rect.center().y),
                            egui::vec2(ICON_SIZE, ICON_SIZE),
                        );
                        achievement_icon(
                            ui,
                            appid,
                            &achievement.api_name,
                            achievement.unlocked,
                            centered,
                            ICON_SIZE,
                        );
                    }
                    ui.separator();
                }
            });
        self.note_edit = if close_edit { None } else { note_edit };
        self.expanded = expanded;
        self.detail_edit = detail_edit;
        if details_touched {
            self.dirty.library = true;
        }
        if touched {
            self.store.checks.insert(appid, checks);
            self.dirty.checks = true;
            self.dirty.library = true;
        }
    }

    fn sort_tab(&mut self, ui: &mut egui::Ui, appid: u32) {
        let rows: Vec<Achievement> = match self.store.game(appid) {
            Some(game) => {
                let order = self.store.sort.get(&appid).cloned().unwrap_or_default();
                model::sort_order_for(&game.achievements, &order)
                    .into_iter()
                    .cloned()
                    .collect()
            }
            None => Vec::new(),
        };
        let mut reset = false;
        ui.horizontal_wrapped(|ui| {
            reset = ui.button("Reset to default order").clicked();
            let mut hide_unlocked = !self.store.settings.filter_unlocked;
            if ui.checkbox(&mut hide_unlocked, "Hide unlocked").changed() {
                self.store.settings.filter_unlocked = !hide_unlocked;
                self.dirty.settings = true;
            }
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
        let (show_locked, show_unlocked, tag_filter) = (
            self.store.settings.filter_locked,
            self.store.settings.filter_unlocked,
            self.store.settings.tag_filter,
        );
        let mut order: Vec<String> = rows.iter().map(|a| a.api_name.clone()).collect();
        let mut pending = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, achievement) in rows.iter().enumerate() {
                    if !model::passes_filter(achievement, show_locked, show_unlocked, tag_filter) {
                        continue;
                    }
                    let (_, dropped) = ui.dnd_drop_zone::<usize, _>(egui::Frame::NONE, |ui| {
                        ui.dnd_drag_source(
                            egui::Id::new(("sort_row", appid, index)),
                            index,
                            |ui| small_achievement_item(ui, appid, achievement),
                        );
                    });
                    if let Some(from) = dropped {
                        pending = Some((*from, index));
                    }
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
            if self.flow_link.is_some()
                && ui
                    .selectable_label(true, "linking\u{2026} cancel")
                    .on_hover_text(
                        "First node picked. Click a second node to connect them, \
                         click the same pair again to disconnect, or click here to cancel.",
                    )
                    .clicked()
            {
                self.flow_link = None;
            }
            let mut hide_unlocked = !self.store.settings.filter_unlocked;
            if ui.checkbox(&mut hide_unlocked, "Hide unlocked").changed() {
                self.store.settings.filter_unlocked = !hide_unlocked;
                self.dirty.settings = true;
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
        let columns = (((ui.available_width() - FLOW_CARD_GAP) / (FLOW_CARD_WIDTH + FLOW_CARD_GAP))
            .floor()
            .max(1.0)) as usize;
        if seed {
            self.flow_seed(appid, columns);
        }
        let achievements: std::collections::HashMap<String, Achievement> = self
            .store
            .game(appid)
            .map(|game| {
                game.achievements
                    .iter()
                    .map(|a| (a.api_name.clone(), a.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let (show_locked, show_unlocked, tag_filter) = (
            self.store.settings.filter_locked,
            self.store.settings.filter_unlocked,
            self.store.settings.tag_filter,
        );
        let mut graph = self.store.flow.remove(&appid).unwrap_or_default();
        let visible: Vec<bool> = graph
            .nodes
            .iter()
            .map(|node| {
                achievements
                    .get(&node.api_name)
                    .map(|achievement| {
                        model::passes_filter(achievement, show_locked, show_unlocked, tag_filter)
                    })
                    .unwrap_or(true)
            })
            .collect();
        let view = ui.available_size();
        let mut content = view;
        for node in &graph.nodes {
            content.x = content
                .x
                .max(node.x + FLOW_CARD_WIDTH / 2.0 + FLOW_CARD_GAP);
            content.y = content.y.max(node.y + FLOW_ROW_SPACING + FLOW_CARD_GAP);
        }
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (response, painter) =
                    ui.allocate_painter(content, egui::Sense::click_and_drag());
                let rect = response.rect;
                let accent = ui.visuals().hyperlink_color;
                let fill = ui.visuals().widgets.inactive.bg_fill;
                let text = ui.visuals().text_color();
                let weak = ui.visuals().weak_text_color();
                let pointer = response.interact_pointer_pos();
                if graph.nodes.is_empty() {
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Stamp achievement nodes to start a flowgraph.",
                        egui::FontId::proportional(14.0),
                        weak,
                    );
                }
                for (from, to) in &graph.edges {
                    if !visible.get(*from).copied().unwrap_or(false)
                        || !visible.get(*to).copied().unwrap_or(false)
                    {
                        continue;
                    }
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
                let pad = 8.0;
                let text_x = pad + SMALL_ICON_SIZE + 8.0;
                let wrap = (FLOW_CARD_WIDTH - text_x - pad).max(1.0);
                let mut card_rects: Vec<egui::Rect> = Vec::with_capacity(graph.nodes.len());
                for (index, node) in graph.nodes.iter().enumerate() {
                    if !visible.get(index).copied().unwrap_or(true) {
                        card_rects.push(egui::Rect::NOTHING);
                        continue;
                    }
                    let center = rect.min + egui::vec2(node.x, node.y);
                    let achievement = achievements.get(&node.api_name);
                    let name =
                        achievement.map_or_else(|| node.api_name.clone(), |a| a.name.clone());
                    let description = achievement
                        .map(|a| a.description.clone())
                        .unwrap_or_default();
                    let name_galley =
                        painter.layout(name, egui::FontId::proportional(13.0), text, wrap);
                    let name_height = name_galley.size().y;
                    let desc_galley = (!description.is_empty()).then(|| {
                        painter.layout(description, egui::FontId::proportional(11.0), weak, wrap)
                    });
                    let mut content = name_height;
                    if let Some(desc) = &desc_galley {
                        content += 3.0 + desc.size().y;
                    }
                    let height =
                        (content.max(SMALL_ICON_SIZE) + pad * 2.0).max(FLOW_CARD_MIN_HEIGHT);
                    let card =
                        egui::Rect::from_center_size(center, egui::vec2(FLOW_CARD_WIDTH, height));
                    card_rects.push(card);
                    let active = self.flow_drag == Some(index) || self.flow_link == Some(index);
                    painter.rect_filled(card, 8.0, fill);
                    painter.rect_stroke(
                        card,
                        8.0,
                        egui::Stroke::new(if active { 3.0 } else { 1.0 }, accent),
                        egui::StrokeKind::Inside,
                    );
                    if let Some(achievement) = achievement {
                        let icon_rect = egui::Rect::from_min_size(
                            egui::pos2(
                                card.left() + pad,
                                card.top() + pad + ((content - SMALL_ICON_SIZE).max(0.0)) / 2.0,
                            ),
                            egui::vec2(SMALL_ICON_SIZE, SMALL_ICON_SIZE),
                        );
                        achievement_icon(
                            ui,
                            appid,
                            &achievement.api_name,
                            achievement.unlocked,
                            icon_rect,
                            SMALL_ICON_SIZE,
                        );
                    }
                    painter.galley(
                        egui::pos2(card.left() + text_x, card.top() + pad),
                        name_galley,
                        text,
                    );
                    if let Some(desc) = desc_galley {
                        painter.galley(
                            egui::pos2(card.left() + text_x, card.top() + pad + name_height + 3.0),
                            desc,
                            weak,
                        );
                    }
                }
                if response.drag_started() {
                    self.flow_drag = pointer.and_then(|p| hit_node(&card_rects, p));
                }
                if response.dragged()
                    && let Some(index) = self.flow_drag
                    && let Some(half) = card_rects.get(index).map(|card| card.size() / 2.0)
                {
                    let delta = response.drag_delta();
                    if let Some(node) = graph.nodes.get_mut(index) {
                        node.x =
                            (node.x + delta.x).clamp(half.x, (rect.width() - half.x).max(half.x));
                        node.y =
                            (node.y + delta.y).clamp(half.y, (rect.height() - half.y).max(half.y));
                        self.dirty.flow = true;
                    }
                }
                if response.drag_stopped() {
                    self.flow_drag = None;
                }
                if response.clicked()
                    && let Some(index) = pointer.and_then(|p| hit_node(&card_rects, p))
                {
                    match self.flow_link {
                        None => self.flow_link = Some(index),
                        Some(from) if from != index => {
                            toggle_edge(&mut graph.edges, from, index);
                            self.dirty.flow = true;
                            self.flow_link = None;
                        }
                        Some(_) => {}
                    }
                }
                if response.secondary_clicked()
                    && let Some(index) = pointer.and_then(|p| hit_node(&card_rects, p))
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
            });
    }

    fn flow_seed(&mut self, appid: u32, columns: usize) {
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
            let (x, y) = flow_node_position(index, columns);
            graph.nodes.push(FlowNode {
                api_name: api_name.clone(),
                x,
                y,
            });
        }
        self.dirty.flow = true;
    }

    fn page_window(&mut self, ctx: &egui::Context) {
        let Some((appid, api_name, title)) = self.page_open.clone() else {
            return;
        };
        let mut open = true;
        let (mut edit, mut browser) = (false, false);
        egui::Window::new(format!("Page: {title}"))
            .open(&mut open)
            .default_size([620.0, 560.0])
            .show(ctx, |ui| {
                if self.page_html.trim().is_empty() {
                    ui.weak("No page for this achievement yet.");
                    if ui.button("Write page (HTML)").clicked() {
                        edit = true;
                    }
                    return;
                }
                ui.horizontal(|ui| {
                    edit |= ui.button("Edit HTML").clicked();
                    browser |= ui.button("Open in browser").clicked();
                });
                ui.separator();
                let base = page::path(appid, &api_name);
                let html = self.page_html.clone();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| render_page(ui, &html, base.parent()));
            });
        if !open {
            self.page_open = None;
        }
        if edit {
            self.page_draft = if self.page_html.trim().is_empty() {
                page::template(&title)
            } else {
                self.page_html.clone()
            };
            self.page_edit = Some((appid, api_name.clone()));
        }
        if browser {
            let url = format!("file://{}", page::path(appid, &api_name).display());
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn page_editor(&mut self, ctx: &egui::Context) {
        let Some((appid, api_name)) = self.page_edit.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Edit page: {api_name}"))
            .open(&mut open)
            .default_size([680.0, 520.0])
            .show(ctx, |ui| {
                ui.weak(
                    "HTML subset: h1-h6, p, ul/li, b, i, code, pre, hr, img, a. \
                     Images are relative to the page file.",
                );
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.page_draft)
                                .code_editor()
                                .desired_rows(20)
                                .desired_width(f32::INFINITY),
                        );
                    });
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        match page::save(appid, &api_name, &self.page_draft) {
                            Ok(_) => {
                                self.page_html = self.page_draft.clone();
                                self.status = "Page saved.".to_owned();
                            }
                            Err(error) => self.status = format!("Page save failed: {error}"),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.page_edit = None;
                    }
                    if ui.button("Open in browser").clicked() {
                        let url = format!("file://{}", page::path(appid, &api_name).display());
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    }
                });
            });
        if !open {
            self.page_edit = None;
        }
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
                    for source in providers::GUIDE_SOURCES {
                        let mut on = *((source.toggle)(&mut settings));
                        if ui
                            .checkbox(&mut on, format!("Guides: {}", source.label))
                            .changed()
                        {
                            *((source.toggle)(&mut settings)) = on;
                            structural = true;
                        }
                    }
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
                ui.horizontal(|ui| {
                    ui.label("Auto-refresh").on_hover_text(
                        "Minutes between automatic library, achievement (selected game) and \
                         save refreshes. 0 turns it off.",
                    );
                    structural |= ui
                        .add(
                            egui::DragValue::new(&mut settings.auto_refresh_minutes)
                                .range(0..=1440)
                                .suffix(" min"),
                        )
                        .changed();
                    ui.weak("0 = off");
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
        self.auto_refresh(ui.ctx());
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
        self.page_window(ui.ctx());
        self.page_editor(ui.ctx());
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
            last_auto_refresh: Instant::now(),
            reload_after_refresh: None,
            expanded: HashSet::new(),
            detail_edit: None,
            page_open: None,
            page_html: String::new(),
            page_edit: None,
            page_draft: String::new(),
        }
    }
}

/// Time until the next periodic refresh, or `None` when auto-refresh is off.
/// A zero duration means the refresh is due now.
fn hours_label(minutes: u32) -> String {
    let hours = minutes as f32 / 60.0;
    if (hours * 10.0).round() % 10.0 == 0.0 {
        format!("~{hours:.0} h")
    } else {
        format!("~{hours:.1} h")
    }
}

fn tag_color(ui: &egui::Ui, tag: Tag) -> egui::Color32 {
    let visuals = ui.visuals();
    match tag {
        Tag::Missable | Tag::Impossible => visuals.warn_fg_color,
        Tag::Bugged => visuals.error_fg_color,
        Tag::Multiplayer => visuals.hyperlink_color,
        Tag::Grind => visuals.selection.stroke.color,
        Tag::Singleplayer => visuals.text_color().gamma_multiply(0.75),
    }
}

fn render_page(ui: &mut egui::Ui, html: &str, base: Option<&std::path::Path>) {
    for block in page::parse(html) {
        match block {
            page::Block::Heading(spans) => {
                ui.add_space(6.0);
                styled_text(ui, &spans, true);
            }
            page::Block::Paragraph(spans) => styled_text(ui, &spans, false),
            page::Block::Item(spans) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label("\u{2022}");
                    styled_text(ui, &spans, false);
                });
            }
            page::Block::Quote(spans) => {
                ui.horizontal(|ui| {
                    ui.label("\u{2502}");
                    ui.vertical(|ui| styled_text(ui, &spans, false));
                });
            }
            page::Block::Code(text) => {
                egui::Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(text).monospace());
                    });
            }
            page::Block::Rule => {
                ui.separator();
            }
            page::Block::Image(src) => {
                if src.starts_with("http://") || src.starts_with("https://") {
                    ui.hyperlink_to(src.as_str(), src.as_str());
                    continue;
                }
                let file = match base {
                    Some(base) => base.join(&src),
                    None => std::path::PathBuf::from(&src),
                };
                if file.exists() {
                    ui.add(
                        egui::Image::new(format!("file://{}", file.display()))
                            .max_size(egui::vec2(ui.available_width().min(560.0), 480.0))
                            .maintain_aspect_ratio(true),
                    );
                } else {
                    ui.weak(format!("image not found: {}", file.display()));
                }
            }
        }
    }
}

fn styled_text(ui: &mut egui::Ui, spans: &[page::Span], heading: bool) {
    let visuals = ui.visuals();
    let style = if heading {
        egui::TextStyle::Heading
    } else {
        egui::TextStyle::Body
    };
    let font = ui
        .style()
        .text_styles
        .get(&style)
        .cloned()
        .unwrap_or_else(|| egui::FontId::proportional(14.0));
    let base = egui::TextFormat {
        font_id: font.clone(),
        color: visuals.text_color(),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut links: Vec<&str> = Vec::new();
    for span in spans {
        let mut format = base.clone();
        let text = match span {
            page::Span::Text(text) => text.as_str(),
            page::Span::Bold(text) => {
                format.color = visuals.text_color().gamma_multiply(1.4);
                text.as_str()
            }
            page::Span::Italic(text) => {
                format.italics = true;
                text.as_str()
            }
            page::Span::Code(text) => {
                format.font_id = egui::FontId::monospace(font.size - 1.0);
                format.background = visuals.extreme_bg_color;
                text.as_str()
            }
            page::Span::Link(text, href) => {
                format.color = visuals.hyperlink_color;
                format.underline = egui::Stroke::new(1.0, visuals.hyperlink_color);
                if !href.is_empty() {
                    links.push(href);
                }
                text.as_str()
            }
        };
        job.append(text, 0.0, format);
    }
    for href in links {
        let format = egui::TextFormat {
            font_id: font.clone(),
            color: visuals.hyperlink_color,
            underline: egui::Stroke::new(1.0, visuals.hyperlink_color),
            ..Default::default()
        };
        job.append(" ", 0.0, base.clone());
        job.append(href, 0.0, format);
    }
    ui.label(job);
}

fn refresh_delay(elapsed: std::time::Duration, minutes: u64) -> Option<std::time::Duration> {
    if minutes == 0 {
        return None;
    }
    Some(std::time::Duration::from_secs(minutes * 60).saturating_sub(elapsed))
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

fn tab_accent_points(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
    const STEPS: usize = 8;
    let r = rect.shrink(1.0);
    let radius = radius.min(r.width() * 0.5).min(r.height());
    let (left, right, top, bottom) = (r.left(), r.right(), r.top(), r.bottom());
    let mut points = vec![egui::pos2(left, bottom)];
    for (center_x, start_angle) in [
        (left + radius, std::f32::consts::PI),
        (right - radius, 3.0 * std::f32::consts::FRAC_PI_2),
    ] {
        for step in 0..=STEPS {
            let angle = start_angle + std::f32::consts::FRAC_PI_2 * (step as f32 / STEPS as f32);
            points.push(egui::pos2(
                center_x + radius * angle.cos(),
                top + radius + radius * angle.sin(),
            ));
        }
    }
    points.push(egui::pos2(right, bottom));
    points
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
        let points = tab_accent_points(rect, style.active_radius.nw as f32);
        ui.painter().add(egui::Shape::line(
            points,
            egui::Stroke::new(2.0, style.accent),
        ));
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
    api_name: &str,
    unlocked: bool,
    rect: egui::Rect,
    size: f32,
) {
    let path = crate::paths::icon_path(appid, api_name, !unlocked);
    if path.exists() {
        // `ui.put` would move the parent cursor back to the icon's bottom, pulling the
        // next row up into this one. A child ui paints without touching the cursor.
        let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        icon_ui.add(
            egui::Image::new(format!("file://{}", path.display()))
                .fit_to_exact_size(egui::vec2(size, size))
                .corner_radius(6.0),
        );
        return;
    }
    ui.painter()
        .rect_filled(rect, 6.0, ui.visuals().widgets.inactive.bg_fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if unlocked { "\u{2714}" } else { "?" },
        egui::FontId::proportional(size * 0.35),
        ui.visuals().weak_text_color(),
    );
}

fn small_achievement_item(ui: &mut egui::Ui, appid: u32, achievement: &Achievement) {
    ui.set_max_width(SMALL_ITEM_WIDTH);
    egui::Frame::NONE
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_max_width(SMALL_ITEM_WIDTH - 20.0);
            let mut icon_rect = None;
            let row = ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(SMALL_ICON_SIZE, SMALL_ICON_SIZE),
                    egui::Sense::hover(),
                );
                icon_rect = Some(rect);
                ui.vertical(|ui| {
                    ui.strong(achievement.name.as_str());
                    if !achievement.description.is_empty() {
                        ui.weak(achievement.description.as_str());
                    }
                });
            });
            if let Some(rect) = icon_rect {
                let centered = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, row.response.rect.center().y),
                    egui::vec2(SMALL_ICON_SIZE, SMALL_ICON_SIZE),
                );
                achievement_icon(
                    ui,
                    appid,
                    &achievement.api_name,
                    achievement.unlocked,
                    centered,
                    SMALL_ICON_SIZE,
                );
            }
        });
}

fn flow_node_position(index: usize, columns: usize) -> (f32, f32) {
    let columns = columns.max(1);
    let column = (index % columns) as f32;
    let row = (index / columns) as f32;
    (
        FLOW_CARD_GAP + FLOW_CARD_WIDTH / 2.0 + column * (FLOW_CARD_WIDTH + FLOW_CARD_GAP),
        FLOW_CARD_GAP + FLOW_CARD_MIN_HEIGHT / 2.0 + row * FLOW_ROW_SPACING,
    )
}

fn hit_node(cards: &[egui::Rect], pointer: egui::Pos2) -> Option<usize> {
    cards.iter().rposition(|card| card.contains(pointer))
}

fn toggle_edge(edges: &mut Vec<(usize, usize)>, from: usize, to: usize) {
    if let Some(position) = edges
        .iter()
        .position(|(a, b)| (*a == from && *b == to) || (*a == to && *b == from))
    {
        edges.remove(position);
    } else {
        edges.push((from, to));
    }
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

fn psnprofiles_guide_url(name: &str) -> String {
    let mut query = String::new();
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                query.push(byte as char);
            }
            _ => query.push_str(&format!("%{byte:02X}")),
        }
    }
    format!("https://psnprofiles.com/search/guides?q={query}")
}

pub(crate) fn guide_slices(markdown: &str, achievements: &[model::Achievement]) -> Vec<String> {
    let lines: Vec<&str> = markdown.lines().collect();
    let lowered: Vec<String> = lines.iter().map(|line| normalize_match(line)).collect();
    let headers: Vec<Option<usize>> = achievements
        .iter()
        .map(|achievement| guide_header(&lowered, achievement))
        .collect();
    let mut marks: Vec<usize> = headers.iter().flatten().copied().collect();
    marks.sort_unstable();
    marks.dedup();
    headers
        .into_iter()
        .map(|header| {
            let Some(index) = header else {
                return String::new();
            };
            let next = marks
                .iter()
                .find(|&&mark| mark > index)
                .copied()
                .unwrap_or(lines.len());
            let end = next.max(index + 1).min(lines.len());
            let slice = &lines[index..end];
            let skip = usize::from(slice.len() > 1 && !slice[0].trim_start().starts_with('#'));
            clean_guide(&slice[skip..])
        })
        .collect()
}

fn guide_header(lowered: &[String], achievement: &model::Achievement) -> Option<usize> {
    let name = normalize_match(&achievement.name);
    if name.is_empty() {
        return None;
    }
    let description = normalize_match(&achievement.description);
    let mut fallback = None;
    for (index, line) in lowered.iter().enumerate() {
        if !line.contains(&name) {
            continue;
        }
        if line.starts_with('#')
            || line.contains('%')
            || (!description.is_empty() && line.contains(&description))
        {
            return Some(index);
        }
        fallback.get_or_insert(index);
    }
    fallback
}

fn normalize_match(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{02bc}' | '`' | '\u{00b4}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
        .trim()
        .to_owned()
}

/// Keeps the guide's line structure: egui collapses newlines inside one label, so the
/// achievement row renders these lines one by one.
/// A guide slice renders as a checklist when it holds a run of >= 3 short bare lines,
/// which is how psnprofiles writes item lists ("bacon bite", "brain bite", ...).
/// Prose guides ("Same as the Tradesman achievement just build with logs.") never match.
/// shortcut: section headings in all caps ("FOOD AND SNACKS") also match; drop them if a
/// guide gains a reliable heading marker.
/// Which checklist lines the save answers, keyed exactly the way the checkbox reads
/// them. Mixing in `sotf::item_key` here would make the scan report ticks the UI
/// never shows, because it folds plurals ("crunchie wunchies" -> "crunchie wunchie").
fn save_tick_lines(guide: &str, found: &std::collections::BTreeSet<String>) -> Vec<String> {
    checklist_lines(guide)
        .into_iter()
        .filter(|line| sotf::line_done(line, found))
        .map(normalize_match)
        .collect()
}

pub(crate) fn checklist_lines(guide: &str) -> HashSet<&str> {
    let lines: Vec<&str> = guide.lines().collect();
    fn bare(line: &str) -> bool {
        let t = line.trim();
        !t.is_empty()
            && !t.starts_with('#')
            && t.chars().count() <= 48
            && !matches!(t.chars().last(), Some('.' | '!' | '?' | ':'))
    }
    let mut out: HashSet<&str> = HashSet::new();
    let mut run = 0usize;
    let mut start = 0usize; // index of the first line in the current run
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue; // blank lines separate list items but do not end the run
        }
        if !bare(line) {
            run = 0;
            continue;
        }
        if run == 0 {
            start = i;
        }
        run += 1;
        if run >= 3 {
            for item in &lines[start..=i] {
                if !item.trim().is_empty() {
                    out.insert(item.trim());
                }
            }
        }
    }
    out
}

fn clean_guide(lines: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let cleaned = strip_markdown_links(line);
        if cleaned.trim().is_empty() {
            if out.last().is_some_and(|l| l.is_empty()) {
                continue;
            }
            out.push(String::new());
            continue;
        }
        // Decorative rules ("-----") have no break points, so egui clips them at the
        // panel edge instead of wrapping. Drop them; blank-line spacing reads the same.
        if cleaned.trim().chars().all(|c| "-=_*~ ".contains(c)) {
            continue;
        }
        out.push(cleaned.trim().to_owned());
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn strip_markdown_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'['
            && let Some(end) = find_closing(bytes, i + 2, b')')
        {
            i = end + 1;
            continue;
        }
        if bytes[i] == b'['
            && let Some(close) = find_closing(bytes, i + 1, b']')
        {
            let text = &line[i + 1..close];
            if close + 1 < bytes.len() && bytes[close + 1] == b'(' {
                if let Some(paren) = find_closing(bytes, close + 2, b')') {
                    out.push_str(text);
                    i = paren + 1;
                    continue;
                }
            } else {
                out.push_str(text);
                i = close + 1;
                continue;
            }
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    let cleaned = out.replace("**", "").replace("__", "");
    cleaned.trim().to_owned()
}

fn find_closing(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|b| *b == needle)
        .map(|p| start + p)
}

fn utf8_len(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
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

    #[test]
    fn crucial_psnprofiles_link_encodes_the_title() {
        assert_eq!(
            psnprofiles_guide_url("Portal 2"),
            "https://psnprofiles.com/search/guides?q=Portal%202"
        );
        assert_eq!(
            psnprofiles_guide_url("NieR: Automata"),
            "https://psnprofiles.com/search/guides?q=NieR%3A%20Automata"
        );
        assert_eq!(
            psnprofiles_guide_url("Ori & the Blind Forest"),
            "https://psnprofiles.com/search/guides?q=Ori%20%26%20the%20Blind%20Forest"
        );
        assert_eq!(
            psnprofiles_guide_url("Ōkami"),
            "https://psnprofiles.com/search/guides?q=%C5%8Ckami"
        );
    }

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
                    guide: String::new(),
                    ..Default::default()
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
    fn crucial_loaded_art_renders_at_the_pending_placeholder_size() {
        let source = egui::vec2(600.0, 900.0);
        let loaded = egui::Image::new("file://art.jpg")
            .fit_to_exact_size(egui::vec2(ART_WIDTH, ART_HEIGHT))
            .calc_size(egui::vec2(CELL_WIDTH, CELL_HEIGHT), Some(source));
        assert!(
            (loaded.x - ART_WIDTH).abs() < 0.001 && (loaded.y - ART_HEIGHT).abs() < 0.001,
            "600x900 art renders {loaded:?} but the placeholder is {ART_WIDTH}x{ART_HEIGHT}"
        );
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

        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(700.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.draw(ui),
        );
        let mut cells: Vec<(f32, f32)> = out
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - ART_WIDTH).abs() < 0.5
                        && (r.rect.height() - ART_HEIGHT).abs() < 0.5 =>
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
            "expected a wrapped grid at 700 px: {rows:?}"
        );

        let col_pitch = CELL_WIDTH + 12.0;
        let row_pitch = CELL_HEIGHT + 10.0;
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
        let art_size = egui::vec2(ART_WIDTH, ART_HEIGHT);

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
                    egui::Shape::Rect(r)
                        if (r.rect.width() - art_size.x).abs() < 0.5
                            && (r.rect.height() - art_size.y).abs() < 0.5 =>
                    {
                        Some(r.rect)
                    }
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
        if let Some(game) = app.store.game_mut(620)
            && let Some(first) = game.achievements.first_mut()
        {
            first.mission = "Chapter 1".to_owned();
            first.typical_minutes = 90;
            first.tags = vec![Tag::Missable, Tag::Grind];
        }
        app.detail_edit = Some("A".to_owned());
        app.store.settings.tag_filter = Some(Tag::Missable);
        app.page_open = Some((620, "A".to_owned(), "Awake".to_owned()));
        app.page_html = "<h1>Page</h1><p>plain <b>bold</b> <i>ital</i> <code>code</code> \
             <a href=\"https://example.com\">link</a></p><ul><li>item</li></ul>\
             <pre>  block</pre><hr><img src=\"missing.png\">"
            .to_owned();
        app.page_edit = Some((620, "A".to_owned()));
        app.page_draft = app.page_html.clone();
        for game_tab in GameTab::ALL {
            app.game_tab = game_tab;
            ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
                .drop_without_applying_deltas();
        }

        // The save-scan button is gated on this appid, so it renders nowhere else.
        app.store.games.push(crate::model::Game {
            appid: sotf::APPID,
            name: "Sons Of The Forest".to_owned(),
            ..Default::default()
        });
        app.selected = Some(sotf::APPID);
        app.game_tab = GameTab::Achievements;
        ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui))
            .drop_without_applying_deltas();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_refresh_delay_counts_down_and_respects_off() {
        use std::time::Duration;
        assert_eq!(refresh_delay(Duration::from_secs(1), 0), None);
        assert_eq!(
            refresh_delay(Duration::from_secs(60), 15),
            Some(Duration::from_secs(14 * 60))
        );
        assert_eq!(
            refresh_delay(Duration::from_secs(15 * 60), 15),
            Some(Duration::ZERO)
        );
        assert_eq!(
            refresh_delay(Duration::from_secs(9_999), 15),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn crucial_strip_markdown_links_removes_images_and_keeps_link_text() {
        assert_eq!(
            strip_markdown_links("![Gold](https://x/gold.png) [Souls Saved](https://y#a)"),
            "Souls Saved"
        );
        assert_eq!(
            strip_markdown_links("plain **bold** text"),
            "plain bold text"
        );
        assert_eq!(strip_markdown_links("[link](u) tail"), "link tail");
        assert_eq!(
            strip_markdown_links("Ōkami — diacritics"),
            "Ōkami — diacritics"
        );
    }

    #[test]
    fn crucial_guide_slice_keeps_the_whole_long_list() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-guide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("guides/psnprofiles")).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        // Far longer than the 12-line cap this used to have, one item per line.
        // Escaped: a raw newline inside a JSON string would not parse.
        let items: String = (0..20).map(|i| format!("\\nitem {i}")).collect();
        std::fs::write(
            dir.join("guides/psnprofiles/620.json"),
            format!(
                r#"{{"title":"T","markdown":"FOODIE 0.00%Common\nEat every kind of edible{items}\n"}}"#
            ),
        )
        .unwrap();

        let mut game = crate::model::Game {
            appid: 620,
            name: "Game".to_owned(),
            ..Default::default()
        };
        game.achievements = vec![Achievement {
            name: "FOODIE".to_owned(),
            ..Default::default()
        }];

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx);
        app.store.games = vec![game];
        app.attach_guide_slices(620);
        let guide = app.store.game(620).unwrap().achievements[0].guide.clone();

        assert!(guide.contains("item 19"), "guide was truncated: {guide:?}");
        assert!(
            guide.lines().any(|l| l.trim() == "item 0"),
            "line structure was flattened: {guide:?}"
        );

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_checklist_detects_item_lists_not_prose() {
        let list = "Eat every kind of edible in the game\nThis achievement includes:\n\n\
                    bacon bite\nbrain bite\nsteak bite\nMRE pack\ncanned food";
        let items = checklist_lines(list);
        assert!(items.contains("bacon bite"), "{items:?}");
        assert!(items.contains("canned food"), "{items:?}");
        // the intro sentence ends in a full stop, so it is prose, not an item
        assert!(!items.contains("This achievement includes:"), "{items:?}");

        let prose =
            "Same as the Tradesman achievement.\nJust build with logs until 100 have been used.";
        assert!(checklist_lines(prose).is_empty(), "{prose:?}");
    }

    #[test]
    fn crucial_save_scan_writes_the_key_the_checkbox_reads() {
        let found: std::collections::BTreeSet<String> = ["crunchie wunchie", "severed arm"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let guide = "FOODIE 0.00%Common\ncrunchie wunchies\nhuman arm\nturtle eggs";
        let keys: HashSet<String> = save_tick_lines(guide, &found).into_iter().collect();

        // written by the scan, read by achievement_list -- same string or the box stays empty
        assert!(keys.contains("crunchie wunchies"), "{keys:?}");
        assert!(keys.contains("human arm"), "{keys:?}");
        assert!(!keys.contains("turtle eggs"), "{keys:?}");
        for line in checklist_lines(guide) {
            assert_eq!(
                keys.contains(&normalize_match(line)),
                sotf::line_done(line, &found),
                "read/write disagree on {line:?}"
            );
        }
    }

    #[test]
    fn crucial_clean_guide_drops_decorative_rules() {
        let lines = [
            "Survive day 50",
            "------------------------------------------------------",
            "Thank you for the support",
            "---",
            "and good luck",
        ];
        let guide = clean_guide(&lines);
        assert!(!guide.contains("---"), "rule kept, would clip: {guide:?}");
        assert!(guide.contains("Survive day 50"));
        assert!(guide.contains("and good luck"));
    }

    #[test]
    fn crucial_achievement_rows_do_not_overlap() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-overlap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        // Long enough to wrap over several visual lines, which is what made the
        // row under-measure and paint over its neighbour.
        let sentence = "A sentence long enough to wrap over several visual lines when the \
                        row is narrow, repeated for effect. ";
        let wide = sentence.repeat(3);
        let mut game = crate::model::Game {
            appid: 1,
            name: "G".to_owned(),
            ..Default::default()
        };
        // A tall row: one wrapping prose line plus a long checklist, with every
        // guide expanded. This is the shape that painted over its neighbour.
        game.achievements = (0..4)
            .map(|i| {
                let guide = std::iter::once(wide.clone())
                    .chain((0..15).map(|j| format!("1x item {i}-{j}")))
                    .collect::<Vec<_>>()
                    .join("\n");
                Achievement {
                    api_name: format!("a{i}"),
                    name: format!("ACH {i}"),
                    description: wide.clone(),
                    guide,
                    ..Default::default()
                }
            })
            .collect();

        // Icons must exist on disk: drawing one used to move the row cursor backwards,
        // which pulled the next row up into a tall one.
        for i in 0..4 {
            let icon = crate::paths::icon_path(1, &format!("a{i}"), true);
            std::fs::create_dir_all(icon.parent().unwrap()).unwrap();
            std::fs::write(&icon, b"x").unwrap();
        }

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        app.store.games = vec![game];
        app.store.settings.filter_locked = true;
        app.store.settings.filter_unlocked = true;
        app.tab = Tab::Games;
        app.selected = Some(1);
        app.game_tab = GameTab::Achievements;
        for i in 0..4 {
            app.expanded.insert(format!("a{i}"));
        }

        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(947.0, 510.0),
        ));
        let out = ctx.run_ui(input, |ui| app.achievement_list(ui, 1));
        let rects = text_rects(&out);
        assert!(rects.len() > 20, "expected text, got {}", rects.len());
        for (index, a) in rects.iter().enumerate() {
            for b in &rects[index + 1..] {
                let overlap = a.intersect(*b);
                assert!(
                    overlap.width() <= 1.0 || overlap.height() <= 1.0,
                    "text overlaps, rows under-measured: {a:?} vs {b:?}"
                );
            }
        }

        out.drop_without_applying_deltas();
        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_guide_button_toggles_the_guide() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-fold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        let mut game = crate::model::Game {
            appid: 1,
            name: "G".to_owned(),
            ..Default::default()
        };
        game.achievements = vec![Achievement {
            api_name: "a0".to_owned(),
            name: "ACH".to_owned(),
            description: "desc".to_owned(),
            guide: (0..20)
                .map(|i| format!("1x item {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ..Default::default()
        }];

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        app.store.games = vec![game];
        app.store.settings.filter_locked = true;
        app.store.settings.filter_unlocked = true;
        app.tab = Tab::Games;
        app.selected = Some(1);
        app.game_tab = GameTab::Achievements;

        let screen = egui::vec2(1000.0, 800.0);
        let raw = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
            events,
            ..Default::default()
        };

        // Folded by default: the guide body is hidden behind a "Show guide" button.
        let out = ctx.run_ui(raw(Vec::new()), |ui| app.achievement_list(ui, 1));
        let button = text_runs(&out)
            .into_iter()
            .find(|(_, text)| text.starts_with("Show guide (20 lines)"))
            .expect("folded guide should offer a Show guide button")
            .0;
        out.drop_without_applying_deltas();
        assert!(!app.expanded.contains("a0"), "guide starts folded");

        let press = |pressed: bool| {
            raw(vec![
                egui::Event::PointerMoved(button.center()),
                egui::Event::PointerButton {
                    pos: button.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
        };
        ctx.run_ui(press(true), |ui| app.achievement_list(ui, 1))
            .drop_without_applying_deltas();
        ctx.run_ui(press(false), |ui| app.achievement_list(ui, 1))
            .drop_without_applying_deltas();
        assert!(
            app.expanded.contains("a0"),
            "clicking Show guide expands it"
        );

        let out = ctx.run_ui(raw(Vec::new()), |ui| app.achievement_list(ui, 1));
        assert!(
            text_runs(&out)
                .iter()
                .any(|(_, text)| text.starts_with("Hide guide (20 lines)")),
            "expanded guide should offer Hide guide"
        );
        out.drop_without_applying_deltas();

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_achievement_sort_control_reorders_the_visible_rows() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-sort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        let mut game = crate::model::Game {
            appid: 1,
            name: "G".to_owned(),
            ..Default::default()
        };
        game.achievements = ["Zeta", "alpha", "Mid"]
            .into_iter()
            .map(|name| Achievement {
                api_name: name.to_owned(),
                name: name.to_owned(),
                ..Default::default()
            })
            .collect();
        let stored: Vec<String> = game.achievements.iter().map(|a| a.name.clone()).collect();

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        app.store.games = vec![game];
        app.tab = Tab::Games;
        app.selected = Some(1);
        app.game_tab = GameTab::Achievements;

        let raw = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let top_of = |out: &egui::FullOutput, name: &str| -> f32 {
            text_runs(out)
                .into_iter()
                .find(|(_, text)| text == name)
                .unwrap_or_else(|| panic!("{name} not drawn"))
                .0
                .min
                .y
        };

        let out = ctx.run_ui(raw(Vec::new()), |ui| app.achievement_list(ui, 1));
        assert!(
            top_of(&out, "Zeta") < top_of(&out, "alpha"),
            "stored order first"
        );
        let name_label = text_runs(&out)
            .into_iter()
            .find(|(_, text)| text == AchievementSort::Name.label())
            .expect("the Sort row must offer a Name option")
            .0;
        out.drop_without_applying_deltas();

        let press = |pressed: bool| {
            raw(vec![
                egui::Event::PointerMoved(name_label.center()),
                egui::Event::PointerButton {
                    pos: name_label.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
        };
        ctx.run_ui(press(true), |ui| app.achievement_list(ui, 1))
            .drop_without_applying_deltas();
        let out = ctx.run_ui(press(false), |ui| app.achievement_list(ui, 1));
        assert_eq!(app.store.settings.achievement_sort, AchievementSort::Name);
        assert!(app.dirty.settings, "the choice has to be saved");
        assert!(
            top_of(&out, "alpha") < top_of(&out, "Mid")
                && top_of(&out, "Mid") < top_of(&out, "Zeta"),
            "name sort applies to the rows"
        );
        out.drop_without_applying_deltas();
        assert_eq!(
            app.store.games[0]
                .achievements
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>(),
            stored,
            "sorting must not rewrite the stored order"
        );

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn text_runs(output: &egui::FullOutput) -> Vec<(egui::Rect, String)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, String)>) {
            match shape {
                egui::Shape::Text(text) => out.push((
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    text.galley.job.text.clone(),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// Every laid-out text run, so a test can prove rows do not paint over each other.
    fn text_rects(output: &egui::FullOutput) -> Vec<egui::Rect> {
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push(egui::Rect::from_min_size(text.pos, text.galley.size()));
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    #[test]
    fn crucial_attach_guide_slices_fills_matching_achievements() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-slice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("guides/psnprofiles")).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        std::fs::write(
            dir.join("guides/psnprofiles/620.json"),
            r###"{"title":"T","markdown":" Taking NamesDefeat a boss 92.81%Common\n\nStory\nYou will get this after the first boss.\n"}"###,
        )
        .unwrap();

        let mut game = crate::model::Game {
            appid: 620,
            name: "Game".to_owned(),
            ..Default::default()
        };
        game.achievements = vec![
            Achievement {
                name: "Taking Names".to_owned(),
                description: "Defeat a boss".to_owned(),
                ..Default::default()
            },
            Achievement {
                name: "Unrelated".to_owned(),
                description: "Nope".to_owned(),
                ..Default::default()
            },
        ];

        let ctx = egui::Context::default();
        let mut app = App::headless(ctx);
        app.store.games = vec![game];
        app.attach_guide_slices(620);
        let achievements = &app.store.game(620).unwrap().achievements;
        assert!(
            achievements[0].guide.contains("after the first boss"),
            "{:?}",
            achievements[0].guide
        );
        assert!(achievements[1].guide.is_empty());

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_active_tab_accent_rounds_the_top_corners_and_skips_the_bottom() {
        let ctx = egui::Context::default();
        let mut app = App::headless(ctx.clone());
        app.tab = Tab::Games;
        let out = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));

        let accent = out.shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Path(path) if path.points.len() > 4 && path.stroke.width == 2.0 => {
                Some(path.points.clone())
            }
            _ => None,
        });
        out.drop_without_applying_deltas();
        let points = accent.expect("active tab accent path");

        let bottom = points.first().unwrap().y;
        let curve = &points[1..points.len() - 1];
        let left = curve
            .iter()
            .copied()
            .min_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        let right = curve
            .iter()
            .copied()
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        let top = curve
            .iter()
            .copied()
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap();

        assert!(top.x > left.x + 1.0, "top-left corner curves inward");
        assert!(top.x < right.x - 1.0, "top-right corner curves inward");
        assert!(
            left.y > top.y + 1.0,
            "left side starts below the rounded corner"
        );
        assert!(
            right.y > top.y + 1.0,
            "right side starts below the rounded corner"
        );
        assert!(points[0].y > top.y + 1.0, "left side runs down");
        assert!(
            (points.last().unwrap().y - bottom).abs() < 0.5,
            "both side ends level"
        );
        assert_eq!(
            points.iter().filter(|p| (p.y - bottom).abs() < 0.5).count(),
            2,
            "only the two side ends reach the bottom, no bottom edge"
        );
    }

    #[test]
    fn crucial_import_psnprofiles_maps_titles_and_skips_unmatched() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        let collector = dir.join("psn-guides.json");
        std::fs::write(
            &collector,
            r###"{"results":[
                {"title":"Portal 2","pickedTitle":"Portal 2 Guide","pickedUrl":"/guide/1-x","markdown":"## Stage\nbody"},
                {"title":"Nonexistent Game","pickedTitle":"n","pickedUrl":"/guide/2","markdown":"nope"},
                {"title":"Hades","pickedTitle":"h","pickedUrl":"/guide/3","markdown":"   "}
            ]}"###,
        )
        .unwrap();

        let games = vec![sample_game()];
        let (written, skipped) = providers::import_psnprofiles(&collector, &games).unwrap();
        assert_eq!((written, skipped), (1, 2));
        let (title, markdown) = providers::local_psnprofiles_guide(620).expect("imported");
        assert_eq!(title, "Portal 2 Guide");
        assert!(markdown.contains("Stage"));

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crucial_hit_node_uses_the_card_bounds() {
        let card = egui::Rect::from_center_size(
            egui::pos2(100.0, 100.0),
            egui::vec2(FLOW_CARD_WIDTH, FLOW_CARD_MIN_HEIGHT),
        );
        let cards = vec![card];
        assert_eq!(hit_node(&cards, egui::pos2(100.0, 100.0)), Some(0));
        assert_eq!(
            hit_node(
                &cards,
                egui::pos2(100.0 + FLOW_CARD_WIDTH / 2.0 - 1.0, 100.0)
            ),
            Some(0)
        );
        assert_eq!(
            hit_node(
                &cards,
                egui::pos2(100.0 + FLOW_CARD_WIDTH / 2.0 + 5.0, 100.0)
            ),
            None
        );
        assert_eq!(
            hit_node(
                &cards,
                egui::pos2(100.0, 100.0 + FLOW_CARD_MIN_HEIGHT / 2.0 + 5.0)
            ),
            None
        );
    }

    #[test]
    fn crucial_hit_node_grabs_the_topmost_of_overlapping_cards() {
        let lower = egui::Rect::from_center_size(
            egui::pos2(100.0, 100.0),
            egui::vec2(FLOW_CARD_WIDTH, FLOW_CARD_MIN_HEIGHT),
        );
        let upper = egui::Rect::from_center_size(
            egui::pos2(110.0, 100.0),
            egui::vec2(FLOW_CARD_WIDTH, FLOW_CARD_MIN_HEIGHT),
        );
        let cards = vec![lower, upper];
        assert_eq!(hit_node(&cards, egui::pos2(100.0, 100.0)), Some(1));
    }

    #[test]
    fn crucial_file_uri_image_loader_is_installed() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        assert!(
            ctx.is_loader_installed(egui_extras::loaders::file_loader::FileLoader::ID),
            "egui_extras `file` feature is required, or file:// art never loads"
        );
        assert!(
            ctx.is_loader_installed(egui_extras::loaders::image_loader::ImageCrateLoader::ID),
            "egui_extras `image` feature is required, or images never decode"
        );
    }

    #[test]
    fn crucial_file_uri_image_loads_end_to_end() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let dir = std::env::temp_dir();

        let png = dir.join(format!("ah-loader-{}.png", std::process::id()));
        let mut png_bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([255, 0, 0, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        std::fs::write(&png, &png_bytes).unwrap();

        let jpg = dir.join(format!("ah-loader-{}.jpg", std::process::id()));
        let mut jpg_bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 3, image::Rgb([0, 255, 0])))
            .write_to(
                &mut std::io::Cursor::new(&mut jpg_bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        std::fs::write(&jpg, &jpg_bytes).unwrap();

        let loaded = |path: &std::path::Path| {
            let uri = format!("file://{}", path.display());
            for _ in 0..400 {
                if let Ok(egui::load::ImagePoll::Ready { image }) =
                    ctx.try_load_image(&uri, egui::load::SizeHint::default())
                {
                    return Some(image.size);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            None
        };
        let png_size = loaded(&png);
        let jpg_size = loaded(&jpg);
        let _ = std::fs::remove_file(&png);
        let _ = std::fs::remove_file(&jpg);
        assert_eq!(png_size, Some([2, 2]), "file:// PNG never decoded");
        assert_eq!(jpg_size, Some([3, 3]), "file:// JPEG never decoded");
    }

    #[test]
    fn crucial_guide_slices_match_each_achievement_block() {
        let markdown = "\
Summary Taking Names A Walk in the Park\n\
\n\
#### Tips\n\
\n\
 Taking NamesDefeat a boss 92.81%Common\n\
\n\
Story\n\
You will get this trophy after defeating your first boss.\n\
\n\
 A Walk in the ParkDefeat every boss 72.64%Common\n\
\n\
Story\n\
Defeat all five bosses in Inkwell Isle 1.\n";
        let achievements = vec![
            Achievement {
                name: "Taking Names".to_owned(),
                description: "Defeat a boss".to_owned(),
                ..Default::default()
            },
            Achievement {
                name: "A Walk in the Park".to_owned(),
                description: "Defeat every boss".to_owned(),
                ..Default::default()
            },
        ];
        let slices = guide_slices(markdown, &achievements);
        assert!(
            slices[0].contains("defeating your first boss"),
            "{:?}",
            slices[0]
        );
        assert!(
            slices[1].contains("five bosses in Inkwell Isle 1"),
            "{:?}",
            slices[1]
        );
        assert!(!slices[0].contains("five bosses"), "slices bled together");
        assert!(
            !slices[0].contains("Taking Names"),
            "header was not stripped"
        );
    }

    #[test]
    fn crucial_guide_slices_match_curly_apostrophes() {
        let markdown = "THIS CAN'T BE HEALTHY\n\nDrink 50 energy drinks.\n\nTHIS PLACE ISN'T SO BAD\n\nSurvive 25 days.";
        let achievements = vec![
            Achievement {
                name: "THIS CAN\u{2019}T BE HEALTHY".to_owned(),
                ..Default::default()
            },
            Achievement {
                name: "THIS PLACE ISN\u{2019}T SO BAD".to_owned(),
                ..Default::default()
            },
        ];
        let slices = guide_slices(markdown, &achievements);
        assert!(slices[0].contains("50 energy drinks"), "{:?}", slices[0]);
        assert!(slices[1].contains("25 days"), "{:?}", slices[1]);
    }

    #[test]
    fn crucial_pasted_page_slices_into_achievements() {
        let html = r#"<!doctype html><html><head><title>Guide</title>
            <script>junk()</script></head><body>
            <nav>PSNProfiles Games</nav>
            <h2>Pacifist</h2>
            <p><strong>Complete the Chamber 4-9 without knocking over any turrets</strong></p>
            <p>Step past the turrets slowly.</p>
            <h2>Sending a message</h2>
            <p><strong>Blow up a test chamber sign</strong></p>
            <p>Throw a cube at the sign.</p>
            </body></html>"#;
        let achievements = vec![
            Achievement {
                name: "Pacifist".to_owned(),
                description: "Complete the Chamber 4-9 without knocking over any turrets"
                    .to_owned(),
                ..Default::default()
            },
            Achievement {
                name: "Sending a message".to_owned(),
                description: "Blow up a test chamber sign".to_owned(),
                ..Default::default()
            },
        ];
        let markdown = crate::providers::page_to_guide(html);
        let slices = guide_slices(&markdown, &achievements);
        assert!(
            slices[0].contains("Step past the turrets"),
            "{:?}",
            slices[0]
        );
        assert!(slices[1].contains("Throw a cube"), "{:?}", slices[1]);
        assert!(!slices[0].contains("Throw a cube"), "slices bled together");
        assert!(!slices[0].contains("PSNProfiles"), "page chrome leaked in");
    }

    #[test]
    fn crucial_guide_slices_empty_when_achievement_absent() {
        let achievements = vec![Achievement {
            name: "Missing Trophy".to_owned(),
            ..Default::default()
        }];
        let slices = guide_slices("## Some Other Trophy\nDo the thing.\n", &achievements);
        assert!(slices[0].is_empty());
    }

    #[test]
    fn crucial_toggle_edge_adds_then_removes_either_direction() {
        let mut edges = Vec::new();
        toggle_edge(&mut edges, 0, 1);
        assert_eq!(edges, vec![(0, 1)]);
        toggle_edge(&mut edges, 1, 0);
        assert!(edges.is_empty());
        toggle_edge(&mut edges, 2, 3);
        toggle_edge(&mut edges, 2, 3);
        assert!(edges.is_empty());
    }

    #[test]
    fn crucial_flow_seed_positions_do_not_overlap() {
        let first = flow_node_position(0, 4);
        let next_column = flow_node_position(1, 4);
        let next_row = flow_node_position(4, 4);
        assert!(next_column.0 - first.0 >= FLOW_CARD_WIDTH);
        assert!(next_row.1 - first.1 >= FLOW_CARD_MIN_HEIGHT);
        assert_eq!(flow_node_position(0, 0), flow_node_position(0, 1));
    }

    #[test]
    fn crucial_local_psnprofiles_url_prefixes_relative_guides() {
        let _guard = crate::test_env_lock();
        let dir = std::env::temp_dir().join(format!("ah-psnurl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("ACHIEVEMENT_HUNTER_DATA", &dir) };

        assert_eq!(providers::local_psnprofiles_url(620), None);

        let guides = crate::paths::psnprofiles_guides_dir();
        std::fs::create_dir_all(&guides).unwrap();
        std::fs::write(
            guides.join("620.json"),
            r#"{"url":"/guide/1-portal-2","markdown":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            providers::local_psnprofiles_url(620).as_deref(),
            Some("https://psnprofiles.com/guide/1-portal-2")
        );

        std::fs::write(
            guides.join("621.json"),
            r#"{"url":"https://psnprofiles.com/guide/9-x","markdown":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            providers::local_psnprofiles_url(621).as_deref(),
            Some("https://psnprofiles.com/guide/9-x")
        );

        unsafe { std::env::remove_var("ACHIEVEMENT_HUNTER_DATA") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
