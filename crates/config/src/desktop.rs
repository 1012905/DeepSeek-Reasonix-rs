//! The [desktop] keys the host reads at startup.

use serde::Deserialize;

use crate::model::Config;

/// [desktop] keys in this slice's scope. The rest of the desktop surface
/// lands with its own slice; unrecognized [desktop] keys pass through.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct DesktopSection {
    /// Anonymous daily ping; nil keeps the default enabled
    /// (config.go DesktopTelemetry).
    pub telemetry: Option<bool>,
    /// Aggregate anonymous counters; nil keeps the default enabled
    /// (config.go DesktopMetrics).
    pub metrics: Option<bool>,
    /// auto|off raw value; anything but "off" is auto.
    pub tray: String,
    /// ask|auto|yolo; empty follows the sandbox claim and folder trust
    /// (desktop_config.go `default_tool_approval_mode`).
    pub default_tool_approval_mode: String,
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
    /// off keeps the taskbar entry as the only surface
    /// (desktop_config.go DesktopTray).
    pub fn desktop_tray(&self) -> &'static str {
        if self.desktop.tray.trim().to_lowercase() == "off" {
            "off"
        } else {
            "auto"
        }
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
    use crate::model::Config;

    #[test]
    fn desktop_telemetry_and_metrics_default_on_when_nil() {
        // config.md §3: DesktopTelemetry/Metrics nil → true.
        let config = Config::default();
        assert!(config.desktop_telemetry());
        assert!(config.desktop_metrics());
    }

    #[test]
    fn desktop_telemetry_and_metrics_follow_explicit_values() {
        let mut config = Config::default();
        config.desktop.telemetry = Some(false);
        config.desktop.metrics = Some(true);
        assert!(!config.desktop_telemetry());
        assert!(config.desktop_metrics());
    }

    #[test]
    fn desktop_tray_is_off_only_for_the_off_spelling() {
        // desktop_config.go: EqualFold(TrimSpace(tray), "off") → off, else auto.
        let mut config = Config::default();
        assert_eq!(config.desktop_tray(), "auto");
        config.desktop.tray = "  OFF ".to_owned();
        assert_eq!(config.desktop_tray(), "off");
        config.desktop.tray = "off".to_owned();
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
}
