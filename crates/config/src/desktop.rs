//! The `[desktop]` and `[telemetry]` keys and their normalization matrices
//! (docs/contracts/config.md §3, §8.3).
//!
//! [desktop] is deliberately separate from top-level `language` and [ui]: a
//! desktop preference never moves CLI language, terminal colours, or anything
//! a provider sees.

use serde::Deserialize;

use crate::model::Config;
use crate::ui::{normalize_close_behavior, normalize_theme_style};

/// The ordered default status-bar items (config.go defaultDesktopStatusBarItems).
pub const DEFAULT_STATUS_BAR_ITEMS: [&str; 16] = [
    "model",
    "workspace",
    "git_branch",
    "cache",
    "cache_avg",
    "session_tokens",
    "turn_tokens",
    "turn_tps",
    "turn_output_tokens",
    "turn_cache_tokens",
    "turn_cost",
    "session_turns",
    "context",
    "compact",
    "cost",
    "balance",
];

/// The only public native CLI update channel (config.go NormalizeCLIUpdateChannel).
pub const CLI_UPDATE_CHANNEL: &str = "stable";

/// The only public desktop update channel (config.go
/// NormalizeDesktopUpdateChannel).
pub const DESKTOP_UPDATE_CHANNEL: &str = "stable";

/// [desktop] keys. The surface is large; the keys carried here are the ones
/// with a normalization matrix plus the host-startup keys. Unrecognized
/// [desktop] keys pass through untouched.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct DesktopSection {
    /// Anonymous daily ping; nil keeps the default enabled.
    pub telemetry: Option<bool>,
    /// Aggregate anonymous counters; nil keeps the default enabled.
    pub metrics: Option<bool>,
    /// `auto`|`off` raw value; anything but "off" is auto.
    pub tray: String,
    /// auto|en|zh; empty auto-detects from the browser/OS locale.
    pub language: String,
    /// Legacy display currency; [billing].display_currency now owns the choice.
    pub currency: String,
    pub theme: String,
    pub theme_style: String,
    pub theme_pack: String,
    /// auto|dark|light; auto follows the resolved desktop theme.
    pub terminal_theme: String,
    /// quit|background; the legacy [ui].close_behavior is the fallback.
    pub close_behavior: String,
    /// standard|compact; legacy `minimal` maps to compact.
    pub display_mode: String,
    /// standard|full|full-all|custom; unknown reads standard.
    pub conversation_width: String,
    /// icon|text; default text.
    pub status_bar_style: String,
    pub status_bar_items: Vec<String>,
    /// ask|auto|yolo; empty follows the sandbox claim and folder trust.
    pub default_tool_approval_mode: String,
    /// Startup update checks; nil keeps the default enabled.
    pub check_updates: Option<bool>,
    /// Legacy channel, read for compatibility and normalized to stable.
    pub update_channel: String,
    /// manual (default) | notify | download.
    pub update_mode: String,
    /// The desktop-only access list for Settings > Model > Access; nil means
    /// undeclared (allow all), an explicit empty list means none.
    pub provider_access: Option<Vec<String>>,
    /// Rollback-pinned release; empty follows the channel.
    pub pinned_version: String,
    /// installed pack id; empty is the default appearance.
    pub theme_pack_id: String,
    pub welcomed: bool,
    pub expand_thinking: bool,
    pub reasoning_display_mode: String,
}

impl DesktopSection {
    /// The resolved desktop theme: dark|light|auto (config.go DesktopTheme).
    pub fn theme(&self) -> &'static str {
        match self.theme.trim().to_lowercase().as_str() {
            "dark" => "dark",
            "light" => "light",
            _ => "auto",
        }
    }

    /// The resolved desktop style, empty meaning the frontend picks one.
    pub fn theme_style(&self) -> &'static str {
        normalize_theme_style(&self.theme_style)
    }

    /// The resolved terminal theme: dark|light|auto.
    pub fn terminal_theme(&self) -> &'static str {
        match self.terminal_theme.trim().to_lowercase().as_str() {
            "dark" => "dark",
            "light" => "light",
            _ => "auto",
        }
    }

    /// The resolved desktop UI language: en|zh, empty for auto. It never reads
    /// top-level `language`, which the CLI/model runtime uses.
    pub fn language(&self) -> &'static str {
        match self.language.trim().to_lowercase().as_str() {
            "en" => "en",
            "zh" => "zh",
            _ => "",
        }
    }

    /// The explicit user-global pricing currency: CNY|USD, empty when the
    /// pricing region should follow the language.
    pub fn currency(&self) -> &'static str {
        match self.currency.trim().to_uppercase().as_str() {
            "CNY" | "RMB" | "CNH" => "CNY",
            "USD" => "USD",
            _ => "",
        }
    }

    /// The normalized transcript display mode: standard|compact.
    pub fn display_mode(&self) -> &'static str {
        match self.display_mode.trim().to_lowercase().as_str() {
            "standard" => "standard",
            "compact" | "minimal" => "compact",
            _ => "standard",
        }
    }

    /// The normalized conversation width: standard|full|full-all|custom.
    pub fn conversation_width(&self) -> &'static str {
        match self.conversation_width.trim().to_lowercase().as_str() {
            "full" => "full",
            "full-all" => "full-all",
            "custom" => "custom",
            _ => "standard",
        }
    }

    /// The normalized status-bar label style: icon|text.
    pub fn status_bar_style(&self) -> &'static str {
        match self.status_bar_style.trim().to_lowercase().as_str() {
            "icon" => "icon",
            _ => "text",
        }
    }

    /// The ordered visible status-bar items: explicit non-empty lists keep
    /// their order and drop unknown/duplicate ids; empty means the full set.
    pub fn status_bar_items(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for raw in &self.status_bar_items {
            let id = raw.trim();
            if !DEFAULT_STATUS_BAR_ITEMS.contains(&id) || out.iter().any(|seen| seen == id) {
                continue;
            }
            out.push(id.to_owned());
        }
        if out.is_empty() {
            return default_status_bar_items();
        }
        out
    }

    /// Whether the shell shows a status icon: auto|off.
    pub fn tray(&self) -> &'static str {
        if self.tray.trim().eq_ignore_ascii_case("off") {
            "off"
        } else {
            "auto"
        }
    }

    /// The channel whose latest pointer to check: always stable, so a legacy
    /// preview value cannot strand the installation on a retired channel.
    pub fn update_channel(&self) -> &'static str {
        DESKTOP_UPDATE_CHANNEL
    }
}

/// The default ordered visible status-bar items.
pub fn default_status_bar_items() -> Vec<String> {
    DEFAULT_STATUS_BAR_ITEMS
        .iter()
        .map(|item| (*item).to_owned())
        .collect()
}

/// Normalizes the desktop/session tool approval posture: auto|yolo|ask.
/// Unknown falls back to ask for safety (config.go NormalizeToolApprovalMode).
pub fn normalize_tool_approval_mode(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "auto" => "auto",
        "yolo" | "full" | "full-access" | "bypass" => "yolo",
        _ => "ask",
    }
}

/// The only public native CLI update channel, whatever the input says.
pub fn normalize_cli_update_channel(_raw: &str) -> &'static str {
    CLI_UPDATE_CHANNEL
}

/// The only public desktop update channel, whatever the input says.
pub fn normalize_desktop_update_channel(_raw: &str) -> &'static str {
    DESKTOP_UPDATE_CHANNEL
}

/// `[telemetry]`: the CLI metrics consent (`auto`|`on`|`off`, empty = never
/// asked). User-global: a cloned repository cannot opt a user into reporting.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct TelemetrySection {
    pub cli_metrics: String,
}

impl TelemetrySection {
    /// Whether the user made an explicit telemetry choice. Persistence keeps
    /// absence until the first eligible consent prompt.
    pub fn configured(&self) -> bool {
        matches!(
            self.cli_metrics.trim().to_lowercase().as_str(),
            "auto" | "on" | "off"
        )
    }

    /// The normalized CLI telemetry policy: auto|on|off.
    pub fn mode(&self) -> &'static str {
        match self.cli_metrics.trim().to_lowercase().as_str() {
            "on" => "on",
            "off" => "off",
            _ => "auto",
        }
    }
}

impl Config {
    /// Whether the desktop sends the anonymous daily ping; absent = on.
    pub fn desktop_telemetry(&self) -> bool {
        self.desktop.telemetry.unwrap_or(true)
    }

    /// Whether the desktop sends aggregate metrics; absent = on.
    pub fn desktop_metrics(&self) -> bool {
        self.desktop.metrics.unwrap_or(true)
    }

    /// Status-icon preference: auto shows one where the platform has one,
    /// off keeps the taskbar entry as the only surface.
    pub fn desktop_tray(&self) -> &'static str {
        self.desktop.tray()
    }

    /// Whether startup runs an update check; absent = on.
    pub fn desktop_check_updates(&self) -> bool {
        self.desktop.check_updates.unwrap_or(true)
    }

    /// The desktop theme: dark|light|auto.
    pub fn desktop_theme(&self) -> &'static str {
        self.desktop.theme()
    }

    /// The desktop close behavior, falling back to the legacy ui value for
    /// configs written before [desktop] existed.
    pub fn desktop_close_behavior(&self) -> &'static str {
        if !self.desktop.close_behavior.trim().is_empty() {
            return normalize_close_behavior(&self.desktop.close_behavior);
        }
        normalize_close_behavior(&self.ui.close_behavior)
    }

    /// The Ask/Auto/YOLO posture the person named for new sessions, or `None`
    /// when they never named one and the session follows the sandbox claim
    /// and folder trust instead (desktop_config.go
    /// `DesktopDefaultToolApprovalMode`). `workspace-write` reads as unnamed
    /// and `danger-full-access` as YOLO; an unrecognized value reads as ask,
    /// which keeps a call in front of the user rather than running it.
    pub fn desktop_default_tool_approval_mode(&self) -> Option<&'static str> {
        match self
            .desktop
            .default_tool_approval_mode
            .trim()
            .to_lowercase()
            .as_str()
        {
            "" | "workspace-write" => None,
            "danger-full-access" => Some("yolo"),
            "ask" => Some("ask"),
            "auto" => Some("auto"),
            "yolo" | "full" | "full-access" | "bypass" => Some("yolo"),
            _ => Some("ask"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_STATUS_BAR_ITEMS, DesktopSection, TelemetrySection, default_status_bar_items,
        normalize_cli_update_channel, normalize_desktop_update_channel,
        normalize_tool_approval_mode,
    };
    use crate::model::Config;

    #[test]
    fn desktop_telemetry_and_metrics_default_on_when_nil() {
        // config.md §3: DesktopTelemetry/Metrics nil → true.
        let config = Config::default();
        assert!(config.desktop_telemetry());
        assert!(config.desktop_metrics());
        assert!(config.desktop_check_updates());
    }

    #[test]
    fn desktop_telemetry_and_metrics_follow_explicit_values() {
        let mut config = Config::default();
        config.desktop.telemetry = Some(false);
        config.desktop.metrics = Some(true);
        config.desktop.check_updates = Some(false);
        assert!(!config.desktop_telemetry());
        assert!(config.desktop_metrics());
        assert!(!config.desktop_check_updates());
    }

    #[test]
    fn desktop_tray_is_off_only_for_the_off_spelling() {
        // desktop_config.go DesktopTray: EqualFold(TrimSpace(tray), "off").
        let mut config = Config::default();
        assert_eq!(config.desktop_tray(), "auto");
        config.desktop.tray = "  OFF ".to_owned();
        assert_eq!(config.desktop_tray(), "off");
        config.desktop.tray = "hidden".to_owned();
        assert_eq!(config.desktop_tray(), "auto");
    }

    #[test]
    fn the_approval_posture_is_none_unless_the_person_named_one() {
        // desktop_config.go DesktopDefaultToolApprovalMode: empty and the
        // legacy workspace-write spelling leave the posture to the sandbox
        // claim and folder trust.
        let mut config = Config::default();
        assert_eq!(config.desktop_default_tool_approval_mode(), None);
        config.desktop.default_tool_approval_mode = " workspace-write ".to_owned();
        assert_eq!(config.desktop_default_tool_approval_mode(), None);
        config.desktop.default_tool_approval_mode = "danger-full-access".to_owned();
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("yolo"));
        config.desktop.default_tool_approval_mode = "Auto".to_owned();
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("auto"));
        config.desktop.default_tool_approval_mode = "full".to_owned();
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("yolo"));
        // An unrecognized value normalizes to ask: the reading that keeps a
        // call in front of the user.
        config.desktop.default_tool_approval_mode = "surprise".to_owned();
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("ask"));
    }

    #[test]
    fn the_approval_posture_decodes_and_layers_through_the_loader() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&dir).expect("create test-tmp base");
        let tmp = tempfile::Builder::new()
            .prefix(".tmpconfig-posture")
            .tempdir_in(&dir)
            .expect("create test temp dir");
        let user = tmp.path().join("user.toml");
        let project = tmp.path().join("project.toml");
        std::fs::write(&user, "[desktop]\ndefault_tool_approval_mode = \"yolo\"\n").unwrap();
        std::fs::write(
            &project,
            "[desktop]\ndefault_tool_approval_mode = \"ask\"\n",
        )
        .unwrap();
        let config = Config::load(&[&user]).expect("load");
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("yolo"));
        // A project file never overrides the user-scope posture.
        let config = Config::load(&[&user, &project]).expect("load");
        assert_eq!(config.desktop_default_tool_approval_mode(), Some("yolo"));
    }

    #[test]
    fn the_desktop_theme_matrix_is_auto_light_dark() {
        // default_test.go TestDefaultDesktopAppearanceAutoGraphite plus the
        // normalization matrix.
        let mut config = Config::default();
        assert_eq!(config.desktop_theme(), "auto");
        assert_eq!(config.desktop.theme_style(), "");
        for (raw, want) in [
            ("AUTO", "auto"),
            (" light ", "light"),
            ("DARK", "dark"),
            ("sepia", "auto"),
            ("", "auto"),
        ] {
            config.desktop.theme = raw.to_owned();
            assert_eq!(config.desktop_theme(), want, "theme = {raw:?}");
        }
        config.desktop.theme_style = " AURORA ".to_owned();
        assert_eq!(config.desktop.theme_style(), "aurora");
        config.desktop.theme_style = "unknown".to_owned();
        assert_eq!(config.desktop.theme_style(), "");
    }

    #[test]
    fn the_desktop_language_matrix_never_reads_top_level_language() {
        // config.md §3: DesktopLanguage is en|zh, else "".
        let mut config = Config {
            language: "zh".to_owned(),
            ..Config::default()
        };
        assert_eq!(config.desktop.language(), "");
        for (raw, want) in [("EN", "en"), (" zh ", "zh"), ("auto", ""), ("fr", "")] {
            config.desktop.language = raw.to_owned();
            assert_eq!(config.desktop.language(), want, "language = {raw:?}");
        }
    }

    #[test]
    fn the_currency_matrix_is_cny_usd_or_empty() {
        // config.md §3: CNY|RMB|CNH → CNY; USD → USD; else "".
        let mut config = Config::default();
        assert_eq!(config.desktop.currency(), "");
        for (raw, want) in [
            ("CNY", "CNY"),
            (" rmb ", "CNY"),
            ("CNH", "CNY"),
            ("usd", "USD"),
            ("EUR", ""),
            ("", ""),
        ] {
            config.desktop.currency = raw.to_owned();
            assert_eq!(config.desktop.currency(), want, "currency = {raw:?}");
        }
    }

    #[test]
    fn the_display_and_width_matrices() {
        // config.md §3: display standard|compact (minimal maps to compact);
        // width full|full-all|custom kept, else standard.
        let mut desktop = DesktopSection::default();
        assert_eq!(desktop.display_mode(), "standard");
        assert_eq!(desktop.conversation_width(), "standard");
        for (raw, want) in [
            ("compact", "compact"),
            (" MINIMAL ", "compact"),
            ("standard", "standard"),
            ("wide", "standard"),
        ] {
            desktop.display_mode = raw.to_owned();
            assert_eq!(desktop.display_mode(), want, "display = {raw:?}");
        }
        for (raw, want) in [
            (" FULL ", "full"),
            ("full-all", "full-all"),
            ("custom", "custom"),
            ("wide", "standard"),
            ("", "standard"),
        ] {
            desktop.conversation_width = raw.to_owned();
            assert_eq!(desktop.conversation_width(), want, "width = {raw:?}");
        }
    }

    #[test]
    fn the_tool_approval_mode_matrix() {
        // config.md §3: auto→auto; yolo|full|full-access|bypass→yolo; else ask.
        for (raw, want) in [
            ("auto", "auto"),
            ("AUTO", "auto"),
            ("yolo", "yolo"),
            ("full", "yolo"),
            ("full-access", "yolo"),
            ("BYPass", "yolo"),
            ("ask", "ask"),
            ("maybe", "ask"),
            ("", "ask"),
        ] {
            assert_eq!(normalize_tool_approval_mode(raw), want, "mode = {raw:?}");
        }
    }

    #[test]
    fn the_legacy_posture_spellings_keep_their_meaning() {
        // desktop_config.go: bypass and full-access are the yolo aliases, and
        // workspace-write reads as the same "never named one" as empty.
        let mut config = Config::default();
        for (raw, want) in [
            ("bypass", Some("yolo")),
            ("full-access", Some("yolo")),
            ("workspace-write", None),
        ] {
            config.desktop.default_tool_approval_mode = raw.to_owned();
            assert_eq!(
                config.desktop_default_tool_approval_mode(),
                want,
                "posture = {raw:?}"
            );
        }
    }

    #[test]
    fn the_status_bar_style_and_item_matrices() {
        // config.md §3: style icon|text default text; items preserve order,
        // drop unknown/dupes, and an empty list yields the default set.
        let mut desktop = DesktopSection::default();
        assert_eq!(desktop.status_bar_style(), "text");
        desktop.status_bar_style = "ICON".to_owned();
        assert_eq!(desktop.status_bar_style(), "icon");
        desktop.status_bar_style = "label".to_owned();
        assert_eq!(desktop.status_bar_style(), "text");

        assert_eq!(desktop.status_bar_items(), default_status_bar_items());
        assert_eq!(
            default_status_bar_items().len(),
            DEFAULT_STATUS_BAR_ITEMS.len()
        );
        desktop.status_bar_items = vec![
            " cost ".to_owned(),
            "nonsense".to_owned(),
            "model".to_owned(),
            "cost".to_owned(),
        ];
        assert_eq!(desktop.status_bar_items(), vec!["cost", "model"]);
        desktop.status_bar_items = vec![" ".to_owned()];
        assert_eq!(desktop.status_bar_items(), default_status_bar_items());
    }

    #[test]
    fn the_close_behavior_falls_back_to_the_legacy_ui_key() {
        // config.md §3 DesktopCloseBehavior: [desktop] wins, else [ui].
        let mut config = Config::default();
        assert_eq!(config.desktop_close_behavior(), "background");
        config.ui.close_behavior = "QUIT".to_owned();
        assert_eq!(config.desktop_close_behavior(), "quit");
        config.desktop.close_behavior = "hide".to_owned();
        assert_eq!(config.desktop_close_behavior(), "background");
        config.desktop.close_behavior = " exit ".to_owned();
        assert_eq!(config.desktop_close_behavior(), "quit");
    }

    #[test]
    fn update_channels_are_always_stable() {
        // config.md §3: both normalizers ignore the input.
        for raw in ["", "stable", "preview", "canary", "beta", "next"] {
            assert_eq!(normalize_cli_update_channel(raw), "stable");
            assert_eq!(normalize_desktop_update_channel(raw), "stable");
        }
        let desktop = DesktopSection {
            update_channel: "preview".to_owned(),
            ..DesktopSection::default()
        };
        assert_eq!(desktop.update_channel(), "stable");
    }

    #[test]
    fn the_cli_telemetry_matrix() {
        // config.md §3: on→on; off→off; else auto. Configured only for the
        // three declared values.
        let mut telemetry = TelemetrySection::default();
        assert_eq!(telemetry.mode(), "auto");
        assert!(!telemetry.configured());
        for (raw, mode, configured) in [
            ("on", "on", true),
            (" OFF ", "off", true),
            ("auto", "auto", true),
            ("", "auto", false),
            ("verbose", "auto", false),
        ] {
            telemetry.cli_metrics = raw.to_owned();
            assert_eq!(telemetry.mode(), mode, "mode = {raw:?}");
            assert_eq!(telemetry.configured(), configured, "configured = {raw:?}");
        }
    }
}
