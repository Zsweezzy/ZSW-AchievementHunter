use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Achievement {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub unlocked: bool,
    pub unlock_time: Option<u64>,
    pub global_percent: Option<f32>,
    pub icon: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuideInfo {
    pub source: String,
    pub url: String,
    pub summary: String,
    pub main_hours: Option<f32>,
    pub extra_hours: Option<f32>,
    pub completionist_hours: Option<f32>,
    pub difficulty: String,
    pub fetched_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Game {
    pub appid: u32,
    pub name: String,
    pub playtime_minutes: u64,
    pub installed: bool,
    #[serde(default)]
    pub achievements: Vec<Achievement>,
    #[serde(default)]
    pub guides: Vec<GuideInfo>,
    #[serde(default)]
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
    Flow,
    Sort,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Games, Tab::Flow, Tab::Sort, Tab::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Games => "Games",
            Tab::Flow => "Flowgraph",
            Tab::Sort => "Custom Sort",
            Tab::Settings => "Settings",
        }
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
}
