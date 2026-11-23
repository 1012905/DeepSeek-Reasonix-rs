//! The `[ui]` section and its presentation normalization matrices
//! (docs/contracts/config.md §3, §8.1). Presentation only — desktop appearance
//! lives in [desktop] so a desktop preference cannot alter terminal output.

use serde::Deserialize;

/// [ui]: terminal presentation preferences.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct UiSection {
    pub theme: String,
    pub theme_style: String,
    /// Legacy shortcut layout; both layouts toggle Plan the same way.
    pub shortcut_layout: String,
    /// Legacy close behavior; desktop.close_behavior wins when it is set.
    pub close_behavior: String,
    pub show_reasoning: bool,
    pub show_turn_usage: bool,
    pub show_changed_files: bool,
    pub feedback_level_notices: bool,
    pub fold_turn_work: bool,
    pub show_receipt: bool,
    pub cursor_shape: String,
    /// Empty keeps insert-always editing; only "vi" enables a command mode.
    pub command_mode: String,
}

impl Default for UiSection {
    fn default() -> Self {
        Self {
            theme: "auto".to_owned(),
            theme_style: String::new(),
            shortcut_layout: String::new(),
            close_behavior: String::new(),
            show_reasoning: false,
            show_turn_usage: true,
            show_changed_files: true,
            feedback_level_notices: false,
            fold_turn_work: false,
            show_receipt: false,
            cursor_shape: String::new(),
            command_mode: String::new(),
        }
    }
}

/// The themes a light/dark shell knows: an unlisted style leaves the frontend
/// to resolve one.
const THEME_STYLES: [&str; 12] = [
    "graphite",
    "aurora",
    "slate",
    "carbon",
    "nocturne",
    "amber",
    "ember",
    "midnight",
    "sandstone",
    "porcelain",
    "linen",
    "glacier",
];

/// Normalizes ui.theme to dark|light|auto (config.go UITheme).
pub fn normalize_theme(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "dark" => "dark",
        "light" => "light",
        _ => "auto",
    }
}

/// Normalizes a theme style, empty when it is not one of the twelve
/// (config.go normalizeThemeStyle).
pub fn normalize_theme_style(raw: &str) -> &'static str {
    let lowered = raw.trim().to_lowercase();
    THEME_STYLES
        .into_iter()
        .find(|style| *style == lowered)
        .unwrap_or_default()
}

/// Normalizes the legacy shortcut layout: desktop|dual|dual-axis|dual_axis are
/// the one layout, everything else is classic (config.go UIShortcutLayout).
pub fn normalize_shortcut_layout(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "desktop" | "dual" | "dual-axis" | "dual_axis" => "desktop",
        _ => "classic",
    }
}

/// Normalizes ui.cursor_shape to block|underline|bar. The slim bar default
/// stays visible without covering CJK wide characters.
pub fn normalize_cursor_shape(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "block" => "block",
        "underline" => "underline",
        _ => "bar",
    }
}

/// Whether the composer should use a vi-style command mode: only "vi" enables
/// it (config.go UICommandMode).
pub fn command_mode_enabled(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("vi")
}

/// Normalizes a close-window preference: quit|exit quit, everything else
/// hides into the background (config.go normalizeCloseBehavior).
pub fn normalize_close_behavior(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "quit" | "exit" => "quit",
        _ => "background",
    }
}

impl UiSection {
    /// The resolved theme: dark|light|auto.
    pub fn theme(&self) -> &'static str {
        normalize_theme(&self.theme)
    }

    /// The resolved style, empty meaning "the default for the shell".
    pub fn theme_style(&self) -> &'static str {
        normalize_theme_style(&self.theme_style)
    }

    /// The resolved shortcut layout: desktop|classic.
    pub fn shortcut_layout(&self) -> &'static str {
        normalize_shortcut_layout(&self.shortcut_layout)
    }

    /// The resolved cursor shape: block|underline|bar.
    pub fn cursor_shape(&self) -> &'static str {
        normalize_cursor_shape(&self.cursor_shape)
    }

    /// Whether vi-style command mode is on.
    pub fn command_mode(&self) -> bool {
        command_mode_enabled(&self.command_mode)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        UiSection, command_mode_enabled, normalize_close_behavior, normalize_cursor_shape,
        normalize_shortcut_layout, normalize_theme, normalize_theme_style,
    };

    #[test]
    fn the_theme_matrix_is_dark_light_auto() {
        // edit_test.go TestUIThemeNormalizes, case for case.
        for (raw, want) in [
            ("", "auto"),
            ("AUTO", "auto"),
            ("dark", "dark"),
            (" light ", "light"),
            ("unknown", "auto"),
            ("Dark", "dark"),
        ] {
            assert_eq!(normalize_theme(raw), want, "theme = {raw:?}");
        }
    }

    #[test]
    fn the_theme_style_matrix_keeps_the_twelve_names() {
        // edit_test.go TestUIThemeStyleNormalizes plus every declared style.
        for (raw, want) in [
            ("", ""),
            ("AURORA", "aurora"),
            (" nocturne ", "nocturne"),
            (" glacier ", "glacier"),
            ("unknown", ""),
        ] {
            assert_eq!(normalize_theme_style(raw), want, "style = {raw:?}");
        }
        for style in super::THEME_STYLES {
            assert_eq!(normalize_theme_style(style), style);
            assert_eq!(normalize_theme_style(&style.to_uppercase()), style);
        }
    }

    #[test]
    fn the_cursor_shape_matrix_defaults_to_bar() {
        // edit_test.go TestUICursorShapeNormalizes.
        for (raw, want) in [
            ("", "bar"),
            ("UNDERLINE", "underline"),
            (" block ", "block"),
            ("bar", "bar"),
            ("unknown", "bar"),
        ] {
            assert_eq!(normalize_cursor_shape(raw), want, "shape = {raw:?}");
        }
    }

    #[test]
    fn command_mode_needs_the_vi_spelling() {
        // edit_test.go TestUICommandMode.
        for (raw, want) in [
            ("", false),
            ("VI", true),
            (" vi ", true),
            ("emacs", false),
            ("unknown", false),
        ] {
            assert_eq!(command_mode_enabled(raw), want, "mode = {raw:?}");
        }
    }

    #[test]
    fn the_close_behavior_matrix_defaults_to_background() {
        // edit_test.go TestUICloseBehaviorNormalizes.
        for (raw, want) in [
            ("", "background"),
            ("QUIT", "quit"),
            ("exit", "quit"),
            (" background ", "background"),
            ("hide", "background"),
            ("unknown", "background"),
        ] {
            assert_eq!(normalize_close_behavior(raw), want, "close = {raw:?}");
        }
    }

    #[test]
    fn the_shortcut_layout_matrix_has_two_answers() {
        for (raw, want) in [
            ("", "classic"),
            ("DESKTOP", "desktop"),
            ("dual", "desktop"),
            ("dual-axis", "desktop"),
            ("dual_axis", "desktop"),
            ("classic", "classic"),
            ("surprise", "classic"),
        ] {
            assert_eq!(normalize_shortcut_layout(raw), want, "layout = {raw:?}");
        }
    }

    #[test]
    fn the_section_defaults_match_config_default() {
        // config.md §2: UI{Theme: "auto", ShowTurnUsage: true,
        // ShowChangedFiles: true}; every other key is the zero value.
        let ui = UiSection::default();
        assert_eq!(ui.theme, "auto");
        assert!(ui.show_turn_usage);
        assert!(ui.show_changed_files);
        assert!(!ui.show_reasoning);
        assert!(!ui.fold_turn_work);
        assert!(!ui.show_receipt);
        assert!(!ui.feedback_level_notices);
        assert_eq!(ui.theme_style(), "");
        assert_eq!(ui.shortcut_layout(), "classic");
        assert_eq!(ui.cursor_shape(), "bar");
        assert!(!ui.command_mode());
    }
}
