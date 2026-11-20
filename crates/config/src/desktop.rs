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
}
