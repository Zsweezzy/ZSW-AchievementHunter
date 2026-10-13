use crate::model::SortMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub background: [u8; 3],
    pub panel: [u8; 3],
    pub field: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    pub accent2: [u8; 3],
    pub ok: [u8; 3],
    pub warning: [u8; 3],
    pub error: [u8; 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self::tokyo_night()
    }
}

impl Theme {
    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".to_owned(),
            background: [26, 27, 38],
            panel: [22, 22, 30],
            field: [36, 40, 59],
            text: [192, 202, 245],
            accent: [122, 162, 247],
            accent2: [187, 154, 247],
            ok: [158, 206, 106],
            warning: [224, 175, 104],
            error: [247, 118, 142],
        }
    }

    pub fn color(&self, rgb: [u8; 3]) -> egui::Color32 {
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FontSettings {
    pub proportional_path: String,
    pub monospace_path: String,
    pub proportional_size: f32,
    pub monospace_size: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            proportional_path: String::new(),
            monospace_path: String::new(),
            proportional_size: 14.0,
            monospace_size: 13.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub steam_api_key: String,
    pub steam_id: String,
    pub steamgriddb_key: String,
    pub use_steamgriddb: bool,
    pub guide_hltb: bool,
    pub guide_steamhunters: bool,
    pub filter_locked: bool,
    pub filter_unlocked: bool,
    pub installed_only: bool,
    pub hide_no_achievements: bool,
    pub hide_completed: bool,
    pub hide_unstarted: bool,
    pub sort_mode: SortMode,
    pub backups_kept: usize,
    pub ui_scale: f32,
    pub theme: Theme,
    pub fonts: FontSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            steam_api_key: String::new(),
            steam_id: String::new(),
            steamgriddb_key: String::new(),
            use_steamgriddb: true,
            guide_hltb: false,
            guide_steamhunters: true,
            filter_locked: true,
            filter_unlocked: true,
            installed_only: false,
            hide_no_achievements: true,
            hide_completed: false,
            hide_unstarted: false,
            sort_mode: SortMode::Playtime,
            backups_kept: 20,
            ui_scale: 1.0,
            theme: Theme::tokyo_night(),
            fonts: FontSettings::default(),
        }
    }
}

impl Settings {
    pub fn merge_env(&mut self) {
        if self.steam_api_key.is_empty()
            && let Ok(key) = std::env::var("STEAM_API_KEY")
        {
            self.steam_api_key = key;
        }
        if self.steam_id.is_empty()
            && let Ok(id) = std::env::var("STEAM_ID")
        {
            self.steam_id = id;
        }
        if self.steamgriddb_key.is_empty()
            && let Ok(key) = std::env::var("STEAMGRIDDB_API_KEY")
        {
            self.steamgriddb_key = key;
        }
    }

    pub fn has_steam_credentials(&self) -> bool {
        !self.steam_api_key.is_empty() && !self.steam_id.is_empty()
    }

    pub fn apply(&self, ctx: &egui::Context) {
        self.fonts.install(ctx);
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.set_zoom_factor(self.ui_scale.clamp(0.6, 2.5));
        let theme = self.theme.clone();
        let fonts = self.fonts.clone();
        ctx.all_styles_mut(move |style| {
            theme.apply_to(&mut style.visuals);
            fonts.apply_to_styles(&mut style.text_styles);
            style.spacing.item_spacing = egui::vec2(12.0, 10.0);
            style.spacing.button_padding = egui::vec2(14.0, 8.0);
            style.spacing.interact_size.y = 30.0;
            style.spacing.menu_margin = egui::Margin::same(10);
            style.visuals.window_corner_radius = 8.into();
        });
    }
}

impl Theme {
    pub fn apply_to(&self, visuals: &mut egui::Visuals) {
        visuals.dark_mode = true;
        visuals.panel_fill = self.color(self.background);
        visuals.window_fill = self.color(self.panel);
        visuals.faint_bg_color = self.color(self.panel);
        visuals.extreme_bg_color = self.color(self.field);
        visuals.code_bg_color = self.color(self.field);
        visuals.override_text_color = Some(self.color(self.text));
        visuals.hyperlink_color = self.color(self.accent);
        visuals.warn_fg_color = self.color(self.warning);
        visuals.error_fg_color = self.color(self.error);
        visuals.selection.bg_fill = self.color(self.accent2).linear_multiply(0.35);
        visuals.selection.stroke = egui::Stroke::new(1.0, self.color(self.accent2));

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = self.color(self.panel);
        widgets.noninteractive.weak_bg_fill = self.color(self.panel);
        widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.color(self.field));
        widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, self.color(self.text));

        widgets.inactive.bg_fill = self.color(self.field);
        widgets.inactive.weak_bg_fill = self.color(self.field);
        widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.color(self.field));
        widgets.inactive.fg_stroke = egui::Stroke::new(1.0, self.color(self.text));

        widgets.hovered.bg_fill = self.color(self.accent).linear_multiply(0.25);
        widgets.hovered.weak_bg_fill = self.color(self.accent).linear_multiply(0.2);
        widgets.hovered.bg_stroke = egui::Stroke::new(1.0, self.color(self.accent));
        widgets.hovered.fg_stroke = egui::Stroke::new(1.0, self.color(self.accent));

        widgets.active.bg_fill = self.color(self.accent).linear_multiply(0.4);
        widgets.active.weak_bg_fill = self.color(self.accent).linear_multiply(0.35);
        widgets.active.bg_stroke = egui::Stroke::new(1.0, self.color(self.accent2));
        widgets.active.fg_stroke = egui::Stroke::new(1.0, self.color(self.text));
    }
}

impl FontSettings {
    pub fn install(&self, ctx: &egui::Context) {
        let mut definitions = egui::FontDefinitions::default();
        let mut custom = false;
        if let Some(bytes) = load_font(&self.proportional_path) {
            let name = "user_proportional".to_owned();
            definitions.font_data.insert(
                name.clone(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            definitions
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, name);
            custom = true;
        }
        if let Some(bytes) = load_font(&self.monospace_path) {
            let name = "user_monospace".to_owned();
            definitions.font_data.insert(
                name.clone(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            definitions
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, name);
            custom = true;
        }
        if custom {
            ctx.set_fonts(definitions);
        }
    }

    pub fn apply_to_styles(
        &self,
        styles: &mut std::collections::BTreeMap<egui::TextStyle, egui::FontId>,
    ) {
        styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(self.proportional_size),
        );
        styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(self.proportional_size),
        );
        styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional((self.proportional_size - 2.0).max(8.0)),
        );
        styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(self.proportional_size + 6.0),
        );
        styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(self.monospace_size),
        );
    }
}

fn load_font(path: &str) -> Option<Vec<u8>> {
    if path.trim().is_empty() {
        return None;
    }
    std::fs::read(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crucial_defaults_show_all_achievements() {
        let s = Settings::default();
        assert!(s.filter_locked && s.filter_unlocked);
        assert_eq!(s.theme.name, "Tokyo Night");
    }

    #[test]
    fn crucial_settings_survive_toml_roundtrip() {
        let mut s = Settings {
            steam_id: "76561199105464144".to_owned(),
            ..Settings::default()
        };
        s.theme.accent = [1, 2, 3];
        s.fonts.proportional_size = 16.5;
        let encoded = toml::to_string_pretty(&s).unwrap();
        let decoded: Settings = toml::from_str(&encoded).unwrap();
        assert_eq!(s, decoded);
    }

    #[test]
    fn crucial_credentials_require_both_values() {
        let mut s = Settings::default();
        assert!(!s.has_steam_credentials());
        s.steam_api_key = "k".to_owned();
        assert!(!s.has_steam_credentials());
        s.steam_id = "1".to_owned();
        assert!(s.has_steam_credentials());
    }

    #[test]
    fn crucial_tokyo_night_matches_reference_palette() {
        let t = Theme::tokyo_night();
        assert_eq!(t.background, [26, 27, 38]);
        assert_eq!(t.accent, [122, 162, 247]);
    }
}
