//! The typed configuration surface for this slice, and the built-in defaults.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::desktop::DesktopSection;
use crate::secrets::SecretsSection;
use crate::serve::ServeSection;

/// The host-startup subset of the frozen configuration surface
/// (docs/contracts/config.md §1). Keys outside the slice, at the top level
/// and inside every table, decode through untouched — the reference decoder
/// ignores them the same way.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Top-level `language`: raw ui/model language tag; empty = auto-detect.
    pub language: String,
    pub serve: ServeSection,
    pub desktop: DesktopSection,
    /// [secrets]: the read-tool denylist toggles. User-global — a workspace
    /// file never gets to arm them.
    pub secrets: SecretsSection,
    /// [storage]: relocation targets keyed by root id. User-global — a
    /// workspace file never gets to set it (load_project.go userGlobals).
    pub storage: BTreeMap<String, String>,
    /// Problems recorded while layers were applied. A malformed file is
    /// skipped with a warning, never fatal (load.go: bad TOML never fails
    /// the load). Not part of the TOML surface.
    #[serde(skip)]
    pub load_warnings: Vec<String>,
}

impl Config {
    /// The built-in defaults for this slice's keys (config.go Default): no
    /// language, serve zero (auth_mode "" reads as none), desktop flags nil
    /// (= enabled), no relocation. Every layer merges over this.
    pub fn builtin_defaults() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use crate::serve::AuthMode;
    use serde::Deserialize;

    #[test]
    fn builtin_defaults_match_the_s1_surface() {
        // config.md §2 for the keys this slice models; everything else is
        // the zero value until its own slice lands.
        let defaults = Config::builtin_defaults();
        assert_eq!(defaults.language, "");
        assert_eq!(defaults.serve.auth_mode, AuthMode::None);
        assert_eq!(defaults.serve.token, "");
        assert_eq!(defaults.serve.password_hash, "");
        assert!(!defaults.serve.behind_proxy);
        assert_eq!(defaults.serve.share_port, 0);
        assert!(!defaults.serve.remember_paired_devices);
        assert!(!defaults.secrets.protect_sensitive_files);
        assert!(defaults.desktop.telemetry.is_none());
        assert!(defaults.desktop.metrics.is_none());
        assert_eq!(defaults.desktop.tray, "");
        assert!(defaults.storage.is_empty());
        assert!(defaults.load_warnings.is_empty());
    }

    #[test]
    fn a_secrets_section_decodes_from_toml() {
        let config: Config = toml::from_str(
            "[secrets]
protect_sensitive_files = true
",
        )
        .unwrap();
        assert!(config.secrets.protect_sensitive_files);
        // Keys the slice does not carry decode through untouched.
        let config: Config = toml::from_str(
            "[secrets]
filter_subprocess_env = true
",
        )
        .unwrap();
        assert!(!config.secrets.protect_sensitive_files);
    }

    #[test]
    fn serde_default_matches_builtin_defaults() {
        // An empty document decodes to the same defaults, so a load from
        // empty files behaves like a load from no files.
        assert_eq!(
            Config::deserialize(toml::Value::Table(toml::Table::new())).ok(),
            Some(Config::builtin_defaults())
        );
    }
}
