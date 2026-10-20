use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Achievement {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub unlocked: bool,
    pub unlock_time: Option<u64>,
    pub global_percent: Option<f32>,
    pub icon: String,
    pub icon_gray: String,
    pub hidden: bool,
    pub note: String,
    pub guide: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GuideInfo {
    pub source: String,
    pub url: String,
    pub summary: String,
    pub main_hours: Option<f32>,
    pub extra_hours: Option<f32>,
    pub completionist_hours: Option<f32>,
    pub difficulty: String,
    pub fetched_at: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub markdown: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub appid: u32,
    pub name: String,
    pub playtime_minutes: u64,
    pub last_played: u64,
    pub installed: bool,
    pub has_achievements: Option<bool>,
    pub achievements: Vec<Achievement>,
    pub guides: Vec<GuideInfo>,
    pub art: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FlowNode {
    pub api_name: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FlowGraph {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Games,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Games, Tab::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Games => "Games",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameTab {
    Achievements,
    Flow,
    Sort,
}

impl GameTab {
    pub const ALL: [GameTab; 3] = [GameTab::Achievements, GameTab::Flow, GameTab::Sort];

    pub fn label(self) -> &'static str {
        match self {
            GameTab::Achievements => "Achievements",
            GameTab::Flow => "Flowgraph",
            GameTab::Sort => "Custom Sort",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {
    #[default]
    Playtime,
    LastPlayed,
    Name,
}

impl SortMode {
    pub const ALL: [SortMode; 3] = [SortMode::Playtime, SortMode::LastPlayed, SortMode::Name];

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Playtime => "Most played",
            SortMode::LastPlayed => "Last played",
            SortMode::Name => "Name",
        }
    }
}

pub fn sort_games(games: &mut [Game], mode: SortMode) {
    match mode {
        SortMode::Playtime => games.sort_by_key(|g| std::cmp::Reverse(g.playtime_minutes)),
        SortMode::LastPlayed => games.sort_by_key(|g| std::cmp::Reverse(g.last_played)),
        SortMode::Name => games.sort_by_key(|g| g.name.to_lowercase()),
    }
}

#[derive(Clone, Copy, Default)]
pub struct Filters {
    pub installed_only: bool,
    pub hide_no_achievements: bool,
    pub hide_completed: bool,
    pub hide_unstarted: bool,
}

pub fn visible(game: &Game, filters: Filters) -> bool {
    if filters.installed_only && !game.installed {
        return false;
    }
    if filters.hide_no_achievements && game.has_achievements == Some(false) {
        return false;
    }
    let total = game.achievements.len();
    if total == 0 {
        return true;
    }
    let unlocked = game.unlocked_count();
    !((filters.hide_completed && unlocked == total) || (filters.hide_unstarted && unlocked == 0))
}

pub fn humanize_ago(seconds: u64) -> String {
    if seconds == 0 {
        return "never".to_owned();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = now.saturating_sub(seconds) / 86_400;
    match days {
        0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        d if d < 30 => format!("{d} days ago"),
        d if d < 365 => format!("{} months ago", d / 30),
        d => format!("{} years ago", d / 365),
    }
}

impl Game {
    pub fn unlocked_count(&self) -> usize {
        self.achievements.iter().filter(|a| a.unlocked).count()
    }

    pub fn completion_percent(&self) -> f32 {
        if self.achievements.is_empty() {
            return 0.0;
        }
        self.unlocked_count() as f32 / self.achievements.len() as f32 * 100.0
    }

    pub fn achievement(&self, api_name: &str) -> Option<&Achievement> {
        self.achievements.iter().find(|a| a.api_name == api_name)
    }
}

pub fn passes_filter(achievement: &Achievement, show_locked: bool, show_unlocked: bool) -> bool {
    if achievement.unlocked {
        show_unlocked
    } else {
        show_locked
    }
}

pub fn sort_order_for<'a>(
    achievements: &'a [Achievement],
    order: &[String],
) -> Vec<&'a Achievement> {
    let mut result: Vec<&Achievement> = order
        .iter()
        .filter_map(|name| achievements.iter().find(|a| &a.api_name == name))
        .collect();
    for achievement in achievements {
        if !order.contains(&achievement.api_name) {
            result.push(achievement);
        }
    }
    result
}

pub fn move_element<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to >= items.len() || from == to {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ach(name: &str, unlocked: bool) -> Achievement {
        Achievement {
            api_name: name.to_owned(),
            name: name.to_owned(),
            unlocked,
            ..Default::default()
        }
    }

    #[test]
    fn crucial_visible_hides_completed_and_unstarted() {
        let full = Game {
            appid: 1,
            achievements: vec![ach("a", true)],
            ..Default::default()
        };
        let unstarted = Game {
            appid: 2,
            achievements: vec![ach("a", false)],
            ..Default::default()
        };
        let partial = Game {
            appid: 3,
            achievements: vec![ach("a", true), ach("b", false)],
            ..Default::default()
        };
        let unsynced = Game {
            appid: 4,
            ..Default::default()
        };
        let filters = Filters {
            hide_completed: true,
            hide_unstarted: true,
            ..Default::default()
        };
        assert!(!visible(&full, filters));
        assert!(!visible(&unstarted, filters));
        assert!(visible(&partial, filters));
        assert!(visible(&unsynced, filters));
    }

    #[test]
    fn crucial_filter_shows_locked_only() {
        let locked = ach("a", false);
        let unlocked = ach("b", true);
        assert!(passes_filter(&locked, true, false));
        assert!(!passes_filter(&unlocked, true, false));
    }

    #[test]
    fn crucial_filter_shows_both_when_both_enabled() {
        assert!(passes_filter(&ach("a", false), true, true));
        assert!(passes_filter(&ach("b", true), true, true));
    }

    #[test]
    fn crucial_filter_shows_nothing_when_both_disabled() {
        assert!(!passes_filter(&ach("a", false), false, false));
        assert!(!passes_filter(&ach("b", true), false, false));
    }

    #[test]
    fn sort_order_puts_custom_first_then_unlisted() {
        let achievements = vec![ach("a", false), ach("b", false), ach("c", false)];
        let order = vec!["c".to_owned(), "a".to_owned()];
        let names: Vec<&str> = sort_order_for(&achievements, &order)
            .iter()
            .map(|a| a.api_name.as_str())
            .collect();
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    #[test]
    fn sort_order_ignores_stale_names() {
        let achievements = vec![ach("a", false)];
        let order = vec!["ghost".to_owned(), "a".to_owned()];
        assert_eq!(sort_order_for(&achievements, &order).len(), 1);
    }

    #[test]
    fn move_element_reorders_and_ignores_out_of_range() {
        let mut items = vec![1, 2, 3];
        move_element(&mut items, 0, 2);
        assert_eq!(items, vec![2, 3, 1]);
        move_element(&mut items, 9, 0);
        assert_eq!(items, vec![2, 3, 1]);
        move_element(&mut items, 1, 1);
        assert_eq!(items, vec![2, 3, 1]);
    }

    #[test]
    fn completion_percent_handles_empty() {
        let game = Game::default();
        assert_eq!(game.completion_percent(), 0.0);
    }

    #[test]
    fn completion_percent_counts_unlocked() {
        let game = Game {
            achievements: vec![
                ach("a", true),
                ach("b", false),
                ach("c", true),
                ach("d", false),
            ],
            ..Default::default()
        };
        assert_eq!(game.unlocked_count(), 2);
        assert_eq!(game.completion_percent(), 50.0);
    }

    #[test]
    fn crucial_visible_hides_only_known_achievementless_games() {
        let known_none = Game {
            has_achievements: Some(false),
            ..Default::default()
        };
        let known_some = Game {
            has_achievements: Some(true),
            ..Default::default()
        };
        let unknown = Game::default();
        let installed = Game {
            installed: true,
            ..Default::default()
        };
        let hide = Filters {
            hide_no_achievements: true,
            ..Default::default()
        };
        let installed_only = Filters {
            installed_only: true,
            ..Default::default()
        };
        assert!(!visible(&known_none, hide));
        assert!(visible(&known_none, Filters::default()));
        assert!(visible(&known_some, hide));
        assert!(visible(&unknown, hide));
        assert!(!visible(&unknown, installed_only));
        assert!(visible(&installed, installed_only));
    }

    #[test]
    fn crucial_sort_games_orders_by_each_mode() {
        let game = |name: &str, playtime: u64, last: u64| Game {
            name: name.to_owned(),
            playtime_minutes: playtime,
            last_played: last,
            ..Default::default()
        };
        let build = || {
            vec![
                game("zeta", 10, 500),
                game("Alpha", 90, 100),
                game("mid", 50, 900),
            ]
        };

        let mut games = build();
        sort_games(&mut games, SortMode::Playtime);
        assert_eq!(games[0].name, "Alpha");
        assert_eq!(games[2].name, "zeta");

        let mut games = build();
        sort_games(&mut games, SortMode::LastPlayed);
        assert_eq!(games[0].name, "mid");
        assert_eq!(games[2].name, "Alpha");

        let mut games = build();
        sort_games(&mut games, SortMode::Name);
        assert_eq!(games[0].name, "Alpha");
        assert_eq!(games[1].name, "mid");
        assert_eq!(games[2].name, "zeta");
    }

    #[test]
    fn crucial_humanize_ago_buckets() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(humanize_ago(0), "never");
        assert_eq!(humanize_ago(now), "today");
        assert_eq!(humanize_ago(now - 86_400 * 2), "2 days ago");
        assert_eq!(humanize_ago(now - 86_400 * 90), "3 months ago");
        assert_eq!(humanize_ago(now - 86_400 * 800), "2 years ago");
    }
}
