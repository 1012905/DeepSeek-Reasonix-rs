//! Desktop and UI preference mutators (CONFIG-026, `edit.go:95-121,236-428`,
//! the `desktop_*.go` and `billing.go` small files).
//!
//! Validators mirror the reference normalization matrix; a few legacy-only
//! fields the Rust schema has not carried yet validate here but do not write a
//! target field (documented in the wip gap list).

use thiserror::Error;

use super::MutatorError;
use crate::desktop::DEFAULT_STATUS_BAR_ITEMS;
use crate::model::Config;
use crate::ui::normalize_theme_style;

/// Desktop/UI preference validation errors.
#[derive(Debug, Error)]
pub enum DesktopError {
    #[error("default_tool_approval_mode {mode:?}: must be ask|auto|yolo")]
    ApprovalModeInvalid { mode: String },

    #[error("shortcut_layout {layout:?}: must be classic|desktop")]
    ShortcutLayoutInvalid { layout: String },

    #[error("desktop theme {theme:?}: must be auto|dark|light")]
    ThemeInvalid { theme: String },

    #[error("desktop theme style {style:?}: must be graphite|aurora|slate|carbon|nocturne|amber")]
    ThemeStyleInvalid { style: String },

    #[error("desktop terminal theme {theme:?}: must be auto|dark|light")]
    TerminalThemeInvalid { theme: String },

    #[error("external opener {id:?}: invalid id")]
    ExternalOpenerInvalid { id: String },

    #[error("close behavior {mode:?}: must be quit|background")]
    CloseBehaviorInvalid { mode: String },

    #[error("display mode {mode:?}: must be standard|compact")]
    DisplayModeInvalid { mode: String },

    #[error("status bar style {style:?}: must be icon|text")]
    StatusBarStyleInvalid { style: String },

    #[error("status bar item {item:?}: unknown item")]
    StatusBarItemUnknown { item: String },

    #[error(
        "CLI update channel {channel:?} is unsupported; Reasonix now uses the official release channel"
    )]
    CLIUpdateChannelUnsupported { channel: String },

    #[error("cli_metrics {mode:?}: must be auto|on|off")]
    CLITelemetryModeInvalid { mode: String },

    #[error("conversation width {width:?}: must be standard|full|full-all|custom")]
    ConversationWidthInvalid { width: String },

    #[error("tray {mode:?}: must be auto|off")]
    TrayInvalid { mode: String },

    #[error("display currency {currency:?}: must be auto|CNY|USD")]
    DisplayCurrencyInvalid { currency: String },

    #[error("config: unknown update mode")]
    UpdateModeInvalid,

    #[error("reasoning display mode {mode:?}: must be hidden|summary|auto")]
    ReasoningDisplayModeInvalid { mode: String },

    #[error("desktop language {lang:?}: must be auto|en|zh")]
    DesktopLanguageInvalid { lang: String },
}

impl DesktopError {
    /// Stable Go sentinel/kind identifier.
    pub fn ident(&self) -> &'static str {
        match self {
            Self::ApprovalModeInvalid { .. } => "ApprovalModeInvalid",
            Self::ShortcutLayoutInvalid { .. } => "ShortcutLayoutInvalid",
            Self::ThemeInvalid { .. } => "DesktopThemeInvalid",
            Self::ThemeStyleInvalid { .. } => "DesktopThemeStyleInvalid",
            Self::TerminalThemeInvalid { .. } => "DesktopTerminalThemeInvalid",
            Self::ExternalOpenerInvalid { .. } => "ExternalOpenerInvalid",
            Self::CloseBehaviorInvalid { .. } => "CloseBehaviorInvalid",
            Self::DisplayModeInvalid { .. } => "DisplayModeInvalid",
            Self::StatusBarStyleInvalid { .. } => "StatusBarStyleInvalid",
            Self::StatusBarItemUnknown { .. } => "StatusBarItemUnknown",
            Self::CLIUpdateChannelUnsupported { .. } => "CLIUpdateChannelUnsupported",
            Self::CLITelemetryModeInvalid { .. } => "CLITelemetryModeInvalid",
            Self::ConversationWidthInvalid { .. } => "ConversationWidthInvalid",
            Self::TrayInvalid { .. } => "TrayInvalid",
            Self::DisplayCurrencyInvalid { .. } => "DisplayCurrencyInvalid",
            Self::UpdateModeInvalid => "ErrUpdateModeInvalid",
            Self::ReasoningDisplayModeInvalid { .. } => "ReasoningDisplayModeInvalid",
            Self::DesktopLanguageInvalid { .. } => "DesktopLanguageInvalid",
        }
    }
}

fn normalize_approval_mode(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "ask" => Ok("ask"),
        "auto" => Ok("auto"),
        "yolo" | "full" | "full-access" | "bypass" => Ok("yolo"),
        other => Err(DesktopError::ApprovalModeInvalid {
            mode: other.to_owned(),
        }),
    }
}

fn normalize_shortcut_layout(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "classic" | "default" | "legacy" | "off" => Ok("classic"),
        "desktop" | "dual" | "dual-axis" | "dual_axis" => Ok("desktop"),
        other => Err(DesktopError::ShortcutLayoutInvalid {
            layout: other.to_owned(),
        }),
    }
}

fn normalize_desktop_theme(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" => Ok("auto"),
        "dark" => Ok("dark"),
        "light" => Ok("light"),
        other => Err(DesktopError::ThemeInvalid {
            theme: other.to_owned(),
        }),
    }
}

fn normalize_desktop_theme_style(raw: &str) -> Result<String, DesktopError> {
    let normalized = normalize_theme_style(raw);
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    if normalized.is_empty() {
        Err(DesktopError::ThemeStyleInvalid {
            style: raw.trim().to_owned(),
        })
    } else {
        Ok(normalized.to_owned())
    }
}

fn normalize_terminal_theme(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" => Ok("auto"),
        "dark" => Ok("dark"),
        "light" => Ok("light"),
        other => Err(DesktopError::TerminalThemeInvalid {
            theme: other.to_owned(),
        }),
    }
}

fn valid_external_opener(id: &str) -> bool {
    let id = id.trim().to_lowercase();
    if id.is_empty() {
        return true;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && id.len() <= 64
        && chars.all(|r| r.is_ascii_alphanumeric() || matches!(r, '.' | '_' | '-'))
}

fn normalize_close_behavior(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "quit" | "exit" => Ok("quit"),
        "" | "background" | "hide" => Ok("background"),
        other => Err(DesktopError::CloseBehaviorInvalid {
            mode: other.to_owned(),
        }),
    }
}

fn normalize_display_mode(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "compact" | "minimal" => Ok("compact"),
        "" | "standard" => Ok("standard"),
        other => Err(DesktopError::DisplayModeInvalid {
            mode: other.to_owned(),
        }),
    }
}

fn normalize_status_bar_style(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "icon" | "icons" => Ok("icon"),
        "" | "text" | "label" | "labels" => Ok("text"),
        other => Err(DesktopError::StatusBarStyleInvalid {
            style: other.to_owned(),
        }),
    }
}

fn normalize_conversation_width(raw: &str) -> Result<String, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "standard" => Ok("standard".to_owned()),
        "full" | "full-all" | "custom" => Ok(raw.trim().to_lowercase()),
        other => Err(DesktopError::ConversationWidthInvalid {
            width: other.to_owned(),
        }),
    }
}

fn normalize_tray(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" | "on" => Ok("auto"),
        "off" | "hidden" => Ok("off"),
        other => Err(DesktopError::TrayInvalid {
            mode: other.to_owned(),
        }),
    }
}

fn normalize_display_currency(raw: &str) -> Result<String, DesktopError> {
    match raw.trim().to_uppercase().as_str() {
        "" | "AUTO" => Ok(String::new()),
        "CNY" | "RMB" | "CNH" => Ok("CNY".to_owned()),
        "USD" => Ok("USD".to_owned()),
        other => Err(DesktopError::DisplayCurrencyInvalid {
            currency: other.to_owned(),
        }),
    }
}

fn normalize_update_mode(raw: &str) -> Result<&'static str, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "manual" => Ok("manual"),
        "notify" => Ok("notify"),
        "download" => Ok("download"),
        _ => Err(DesktopError::UpdateModeInvalid),
    }
}

fn normalize_reasoning_display_mode(raw: &str) -> Result<(&'static str, bool), DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "hidden" => Ok(("hidden", false)),
        "summary" => Ok(("summary", false)),
        "auto" => Ok(("auto", true)),
        other => Err(DesktopError::ReasoningDisplayModeInvalid {
            mode: other.to_owned(),
        }),
    }
}

fn normalize_desktop_language(raw: &str) -> Result<String, DesktopError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" => Ok(String::new()),
        "en" => Ok("en".to_owned()),
        "zh" => Ok("zh".to_owned()),
        other => Err(DesktopError::DesktopLanguageInvalid {
            lang: other.to_owned(),
        }),
    }
}

impl Config {
    /// Writes the session approval posture (`edit.go:95-107`).
    pub fn set_desktop_default_tool_approval_mode(
        &mut self,
        mode: &str,
    ) -> Result<(), MutatorError> {
        self.desktop.default_tool_approval_mode = normalize_approval_mode(mode)?.to_owned();
        Ok(())
    }

    /// Writes the legacy shortcut layout (`edit.go:111-121`).
    pub fn set_ui_shortcut_layout(&mut self, layout: &str) -> Result<(), MutatorError> {
        self.ui.shortcut_layout = normalize_shortcut_layout(layout)?.to_owned();
        Ok(())
    }

    /// Writes the desktop theme and style together (`edit.go:236-257`).
    pub fn set_desktop_appearance(&mut self, theme: &str, style: &str) -> Result<(), MutatorError> {
        self.desktop.theme = normalize_desktop_theme(theme)?.to_owned();
        self.desktop.theme_style = normalize_desktop_theme_style(style)?;
        Ok(())
    }

    /// Writes the terminal theme (`edit.go:261-273`).
    pub fn set_desktop_terminal_theme(&mut self, theme: &str) -> Result<(), MutatorError> {
        self.desktop.terminal_theme = normalize_terminal_theme(theme)?.to_owned();
        Ok(())
    }

    /// Validates an external opener id. The schema has not carried the opener
    /// field yet, so this validates only; wiring is a wip.
    pub fn validate_desktop_external_opener(id: &str) -> Result<(), MutatorError> {
        if valid_external_opener(id) {
            Ok(())
        } else {
            Err(DesktopError::ExternalOpenerInvalid {
                id: id.trim().to_owned(),
            }
            .into())
        }
    }

    /// Writes the close behavior (`edit.go:294-304`).
    pub fn set_desktop_close_behavior(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.desktop.close_behavior = normalize_close_behavior(mode)?.to_owned();
        Ok(())
    }

    /// Legacy UI close behavior delegates to the desktop one (`edit.go:417-419`).
    pub fn set_ui_close_behavior(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.set_desktop_close_behavior(mode)
    }

    /// Writes the transcript display mode (`edit.go:307-317`).
    pub fn set_desktop_display_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.desktop.display_mode = normalize_display_mode(mode)?.to_owned();
        Ok(())
    }

    /// Writes the status-bar label style (`edit.go:321-331`).
    pub fn set_desktop_status_bar_style(&mut self, style: &str) -> Result<(), MutatorError> {
        self.desktop.status_bar_style = normalize_status_bar_style(style)?.to_owned();
        Ok(())
    }

    /// Writes the visible status-bar items, rejecting unknown ids
    /// (`edit.go:335-354`).
    pub fn set_desktop_status_bar_items(&mut self, items: &[String]) -> Result<(), MutatorError> {
        let mut normalized: Vec<String> = Vec::new();
        for raw in items {
            let id = raw.trim();
            if !DEFAULT_STATUS_BAR_ITEMS.contains(&id) {
                return Err(DesktopError::StatusBarItemUnknown { item: raw.clone() }.into());
            }
            if !normalized.iter().any(|seen| seen == id) {
                normalized.push(id.to_owned());
            }
        }
        self.desktop.status_bar_items = normalized;
        Ok(())
    }

    /// Records the startup update-check preference (`edit.go:358-361`).
    pub fn set_desktop_check_updates(&mut self, enabled: bool) {
        self.desktop.check_updates = Some(enabled);
    }

    /// Retired CLI channel: every accepted spelling migrates to empty
    /// (`edit.go:365-373`).
    pub fn set_cli_update_channel(&mut self, channel: &str) -> Result<(), MutatorError> {
        match channel.trim().to_lowercase().as_str() {
            "" | "stable" | "preview" | "canary" | "beta" | "next" => {
                self.desktop.update_channel = String::new();
                Ok(())
            }
            other => Err(DesktopError::CLIUpdateChannelUnsupported {
                channel: other.to_owned(),
            }
            .into()),
        }
    }

    /// Records the desktop daily ping preference (`edit.go:376-379`).
    pub fn set_desktop_telemetry(&mut self, enabled: bool) {
        self.desktop.telemetry = Some(enabled);
    }

    /// Records the aggregate metrics preference (`edit.go:382-385`).
    pub fn set_desktop_metrics(&mut self, enabled: bool) {
        self.desktop.metrics = Some(enabled);
    }

    /// Writes the CLI metrics consent (`edit.go:388-400`).
    pub fn set_cli_telemetry_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        match mode.trim().to_lowercase().as_str() {
            "" | "auto" => self.telemetry.cli_metrics = "auto".to_owned(),
            "on" => self.telemetry.cli_metrics = "on".to_owned(),
            "off" => self.telemetry.cli_metrics = "off".to_owned(),
            other => {
                return Err(DesktopError::CLITelemetryModeInvalid {
                    mode: other.to_owned(),
                }
                .into());
            }
        }
        Ok(())
    }

    /// Writes the conversation width (`edit.go:404-414`).
    pub fn set_desktop_conversation_width(&mut self, width: &str) -> Result<(), MutatorError> {
        self.desktop.conversation_width = normalize_conversation_width(width)?;
        Ok(())
    }

    /// Writes the status-icon preference (`desktop_config.go:42-51`).
    pub fn set_desktop_tray(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.desktop.tray = normalize_tray(mode)?.to_owned();
        Ok(())
    }

    /// Writes the display currency (`billing.go:59-77`).
    pub fn set_display_currency(&mut self, currency: &str) -> Result<(), MutatorError> {
        self.billing.display_currency = normalize_display_currency(currency)?;
        Ok(())
    }

    /// Legacy desktop currency delegates to the display currency (`billing.go:81-83`).
    pub fn set_desktop_currency(&mut self, currency: &str) -> Result<(), MutatorError> {
        let normalized = normalize_display_currency(currency)?;
        self.desktop.currency = normalized.clone();
        self.billing.display_currency = normalized;
        Ok(())
    }

    /// Writes the desktop update mode (`desktop_update_mode.go:49-56`).
    pub fn set_desktop_update_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.desktop.update_mode = normalize_update_mode(mode)?.to_owned();
        Ok(())
    }

    /// Clears the rollback pin; an empty version unpins (`desktop_pin.go:19-22`).
    pub fn set_desktop_pinned_version(&mut self, version: &str) {
        self.desktop.pinned_version = version.trim().to_owned();
    }

    /// Writes the reasoning display mode and its expand-thinking flag
    /// (`reasoning_display.go:31-43`).
    pub fn set_desktop_reasoning_display_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        let (mode, expand) = normalize_reasoning_display_mode(mode)?;
        self.desktop.reasoning_display_mode = mode.to_owned();
        self.desktop.expand_thinking = expand;
        Ok(())
    }

    /// Writes the desktop UI language (`edit.go:219-232`); top-level
    /// `language` is untouched.
    pub fn set_desktop_language(&mut self, lang: &str) -> Result<(), MutatorError> {
        self.desktop.language = normalize_desktop_language(lang)?;
        Ok(())
    }

    /// Writes `UI.ShowReasoning` (`edit.go:425-428`).
    pub fn set_show_reasoning(&mut self, on: bool) {
        self.ui.show_reasoning = on;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_mode_aliases_fall_to_yolo() {
        let mut config = Config::default();
        config
            .set_desktop_default_tool_approval_mode("full-access")
            .unwrap();
        assert_eq!(config.desktop.default_tool_approval_mode, "yolo");
        let err = config
            .set_desktop_default_tool_approval_mode("danger")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"default_tool_approval_mode "danger": must be ask|auto|yolo"#
        );
    }

    #[test]
    fn shortcut_layout_matrix() {
        let mut config = Config::default();
        config.set_ui_shortcut_layout("dual_axis").unwrap();
        assert_eq!(config.ui.shortcut_layout, "desktop");
        let err = config.set_ui_shortcut_layout("mouse").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"shortcut_layout "mouse": must be classic|desktop"#
        );
    }

    #[test]
    fn desktop_theme_and_style_matrix() {
        let mut config = Config::default();
        config.set_desktop_appearance("dark", "aurora").unwrap();
        assert_eq!(config.desktop.theme, "dark");
        assert_eq!(config.desktop.theme_style, "aurora");
        let err = config.set_desktop_appearance("neon", "").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"desktop theme "neon": must be auto|dark|light"#
        );
        let err = config.set_desktop_appearance("dark", "neon").unwrap_err();
        assert!(err.to_string().starts_with("desktop theme style \"neon\""));
    }

    #[test]
    fn close_behavior_display_mode_and_status_style() {
        let mut config = Config::default();
        config.set_desktop_close_behavior("exit").unwrap();
        assert_eq!(config.desktop.close_behavior, "quit");
        let err = config.set_desktop_close_behavior("min").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"close behavior "min": must be quit|background"#
        );
        config.set_desktop_display_mode("minimal").unwrap();
        assert_eq!(config.desktop.display_mode, "compact");
        let err = config.set_desktop_display_mode("wide").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"display mode "wide": must be standard|compact"#
        );
        config.set_desktop_status_bar_style("icons").unwrap();
        assert_eq!(config.desktop.status_bar_style, "icon");
        let err = config.set_desktop_status_bar_style("fancy").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"status bar style "fancy": must be icon|text"#
        );
    }

    #[test]
    fn status_bar_items_reject_unknown_ids() {
        let mut config = Config::default();
        config
            .set_desktop_status_bar_items(&["model".to_owned(), "cost".to_owned()])
            .unwrap();
        assert_eq!(config.desktop.status_bar_items, vec!["model", "cost"]);
        let err = config
            .set_desktop_status_bar_items(&["nope".to_owned()])
            .unwrap_err();
        assert_eq!(err.to_string(), r#"status bar item "nope": unknown item"#);
    }

    #[test]
    fn currency_tray_width_and_telemetry_mode() {
        let mut config = Config::default();
        config.set_display_currency("RMB").unwrap();
        assert_eq!(config.billing.display_currency, "CNY");
        let err = config.set_display_currency("EUR").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"display currency "EUR": must be auto|CNY|USD"#
        );
        config.set_desktop_tray("hidden").unwrap();
        assert_eq!(config.desktop.tray, "off");
        let err = config.set_desktop_tray("float").unwrap_err();
        assert_eq!(err.to_string(), r#"tray "float": must be auto|off"#);
        config.set_desktop_conversation_width("FULL-ALL").unwrap();
        assert_eq!(config.desktop.conversation_width, "full-all");
        let err = config.set_desktop_conversation_width("wide").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"conversation width "wide": must be standard|full|full-all|custom"#
        );
        config.set_cli_telemetry_mode("ON").unwrap();
        assert_eq!(config.telemetry.cli_metrics, "on");
        let err = config.set_cli_telemetry_mode("maybe").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"cli_metrics "maybe": must be auto|on|off"#
        );
    }

    #[test]
    fn update_mode_and_reasoning_display_mode() {
        let mut config = Config::default();
        config.set_desktop_update_mode("notify").unwrap();
        assert_eq!(config.desktop.update_mode, "notify");
        let err = config.set_desktop_update_mode("auto").unwrap_err();
        assert_eq!(err.to_string(), "config: unknown update mode");
        config.set_desktop_reasoning_display_mode("auto").unwrap();
        assert!(config.desktop.expand_thinking);
        assert_eq!(config.desktop.reasoning_display_mode, "auto");
        let err = config
            .set_desktop_reasoning_display_mode("raw")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"reasoning display mode "raw": must be hidden|summary|auto"#
        );
    }

    #[test]
    fn external_opener_id_shape() {
        assert!(Config::validate_desktop_external_opener("dev.tools").is_ok());
        let err = Config::validate_desktop_external_opener("has space").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"external opener "has space": invalid id"#
        );
    }
}
