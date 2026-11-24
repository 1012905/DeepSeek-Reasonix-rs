//! The typed configuration surface and its built-in defaults.
//!
//! Keys outside this slice — at the top level and inside every table — decode
//! through untouched: the reference decoder ignores unknown keys the same way
//! (docs/contracts/config.md §1, §2).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::agent::AgentSection;
use crate::desktop::{DesktopSection, TelemetrySection};
use crate::plugin::PluginEntry;
use crate::provider::ProviderEntry;
use crate::sandbox::{PermissionsSection, SandboxSection};
use crate::secrets::SecretsSection;
use crate::serve::ServeSection;
use crate::tools::ToolsSection;
use crate::ui::UiSection;

/// The config schema marker a fresh file carries (pricing.go freshConfigVersion).
pub const FRESH_CONFIG_VERSION: i64 = 12;

/// The model a fresh install answers with (config.go Default).
pub const DEFAULT_DEFAULT_MODEL: &str = "deepseek-flash";

/// The credentials store a fresh install uses (credentials.go
/// CredentialsStoreAuto).
pub const CREDENTIALS_STORE_AUTO: &str = "auto";

/// [billing]: the display currency. A provider's frozen list-price currency is
/// independent of this, so switching display never rewrites a rate.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct BillingSection {
    /// Empty follows the desktop/CLI language.
    pub display_currency: String,
}

/// The runtime configuration. Every table is `#[serde(default)]` so an absent
/// table keeps its built-in default.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub config_version: i64,
    /// Provider name (→ its default model) or "provider/model".
    pub default_model: String,
    /// Top-level `language`: raw ui/model language tag; empty = auto-detect.
    pub language: String,
    pub credentials_store: String,
    pub ui: UiSection,
    pub desktop: DesktopSection,
    /// User-global: a project file never opts a user into reporting.
    pub telemetry: TelemetrySection,
    pub agent: AgentSection,
    pub providers: Vec<ProviderEntry>,
    pub tools: ToolsSection,
    pub permissions: PermissionsSection,
    pub sandbox: SandboxSection,
    pub plugins: Vec<PluginEntry>,
    pub billing: BillingSection,
    pub serve: ServeSection,
    /// [secrets]: the read-tool denylist toggles. User-global — a workspace
    /// file never gets to arm them.
    pub secrets: SecretsSection,
    /// [storage]: relocation targets keyed by root id. User-global — a
    /// workspace file never gets to set it (load_project.go userGlobals).
    pub storage: BTreeMap<String, String>,
    /// Problems recorded while layers were applied. A malformed file is
    /// skipped with a warning, never fatal (load.go: bad TOML never fails the
    /// load). Not part of the TOML surface.
    #[serde(skip)]
    pub load_warnings: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self::builtin_defaults()
    }
}

impl Config {
    /// The built-in defaults (config.go Default): the fresh-install identity
    /// and the presentation defaults. Every layer merges over this.
    pub fn builtin_defaults() -> Self {
        Self {
            config_version: FRESH_CONFIG_VERSION,
            default_model: DEFAULT_DEFAULT_MODEL.to_owned(),
            language: String::new(),
            credentials_store: CREDENTIALS_STORE_AUTO.to_owned(),
            ui: UiSection::default(),
            desktop: DesktopSection::default(),
            telemetry: TelemetrySection::default(),
            agent: AgentSection::default(),
            providers: Vec::new(),
            tools: ToolsSection::default(),
            permissions: PermissionsSection::default(),
            sandbox: SandboxSection::default(),
            plugins: Vec::new(),
            billing: BillingSection::default(),
            serve: ServeSection::default(),
            secrets: SecretsSection::default(),
            storage: BTreeMap::new(),
            load_warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CREDENTIALS_STORE_AUTO, Config, DEFAULT_DEFAULT_MODEL, FRESH_CONFIG_VERSION};
    use crate::agent::DEFAULT_SYSTEM_PROMPT;
    use crate::serve::AuthMode;
    use serde::Deserialize;

    #[test]
    fn builtin_defaults_match_the_frozen_surface() {
        // config.md §2, value for value.
        let defaults = Config::builtin_defaults();
        assert_eq!(defaults.config_version, FRESH_CONFIG_VERSION);
        assert_eq!(defaults.config_version, 12);
        assert_eq!(defaults.default_model, DEFAULT_DEFAULT_MODEL);
        assert_eq!(defaults.default_model, "deepseek-flash");
        assert_eq!(defaults.credentials_store, CREDENTIALS_STORE_AUTO);
        assert_eq!(defaults.credentials_store, "auto");
        assert_eq!(defaults.language, "");
        assert_eq!(defaults.ui.theme, "auto");
        assert!(defaults.ui.show_turn_usage);
        assert!(defaults.ui.show_changed_files);
        assert_eq!(defaults.desktop.conversation_width, "");
        assert_eq!(defaults.agent.system_prompt, DEFAULT_SYSTEM_PROMPT);
        assert_eq!(defaults.agent.max_steps, 0);
        assert_eq!(defaults.agent.planner_max_steps, 0);
        assert_eq!(defaults.agent.auto_plan, "off");
        assert_eq!(defaults.agent.compact_ratio, 0.90);
        assert_eq!(defaults.agent.max_subagent_depth, 8);
        assert_eq!(defaults.agent.max_provider_retries, 10);
        assert_eq!(defaults.agent.max_subagent_concurrency, 0);
        assert_eq!(defaults.agent.max_parallel_writers, 0);
        assert_eq!(defaults.agent.subagent_claims, "optimistic");
        assert_eq!(defaults.agent.write_lease, "optimistic");
        assert_eq!(defaults.agent.plan_approval, "ask");
        assert_eq!(defaults.agent.default_mode, "normal");
        assert_eq!(defaults.permissions.mode, "ask");
        assert!(defaults.sandbox.network);
        assert_eq!(defaults.sandbox.host_authorities, vec!["ssh_agent"]);
        assert!(defaults.tools.present_files);
        assert_eq!(defaults.serve.auth_mode, AuthMode::None);
        assert!(defaults.storage.is_empty());
        assert!(defaults.load_warnings.is_empty());
        assert!(defaults.providers.is_empty());
        assert!(defaults.plugins.is_empty());
        assert!(defaults.billing.display_currency.is_empty());
    }

    #[test]
    fn serde_default_matches_builtin_defaults() {
        // An empty document decodes to the same defaults, so a load from empty
        // files behaves like a load from no files (default_test.go covers the
        // same claim one field at a time).
        assert_eq!(
            Config::deserialize(toml::Value::Table(toml::Table::new())).ok(),
            Some(Config::builtin_defaults())
        );
    }

    #[test]
    fn the_shipped_sections_decode_from_their_tables() {
        let config: Config = toml::from_str(
            r#"
config_version = 12
default_model = "gateway/my-model"
language = "zh"
credentials_store = "auto"

[ui]
theme = "dark"
cursor_shape = "block"

[desktop]
theme = "light"
conversation_width = "full"

[telemetry]
cli_metrics = "on"

[agent]
system_prompt = "be terse"
compact_ratio = 0.85

[tools]
bash_timeout_seconds = 30
present_files = false

[permissions]
mode = "deny"
deny = ["Bash(rm -rf *)"]

[sandbox]
network = false
host_authorities = ["ssh_agent", "docker"]

[billing]
display_currency = "USD"

[secrets]
protect_sensitive_files = true
"#,
        )
        .expect("document decodes");
        assert_eq!(config.default_model, "gateway/my-model");
        assert_eq!(config.ui.theme, "dark");
        assert_eq!(config.ui.cursor_shape(), "block");
        assert_eq!(config.desktop_theme(), "light");
        assert_eq!(config.desktop.conversation_width(), "full");
        assert_eq!(config.telemetry.mode(), "on");
        assert_eq!(config.agent.system_prompt, "be terse");
        assert_eq!(config.agent.compact_ratio, 0.85);
        assert_eq!(config.tools.bash_timeout_seconds(), 30);
        assert!(!config.tools.present_files);
        assert_eq!(config.permissions.mode(), "deny");
        assert_eq!(config.permissions.deny, vec!["Bash(rm -rf *)"]);
        assert!(!config.sandbox.network);
        assert_eq!(config.sandbox.host_authorities.len(), 2);
        assert_eq!(config.billing.display_currency, "USD");
        assert!(config.secrets.protect_sensitive_files);
    }

    #[test]
    fn a_secrets_section_decodes_from_toml() {
        // config.md §8: the carried key decodes; a key this slice does not
        // model passes through without disturbing it.
        let config: Config = toml::from_str("[secrets]\nprotect_sensitive_files = true\n")
            .expect("secrets document decodes");
        assert!(config.secrets.protect_sensitive_files);
        let config: Config = toml::from_str("[secrets]\nfilter_subprocess_env = true\n")
            .expect("unknown secrets key decodes");
        assert!(!config.secrets.protect_sensitive_files);
    }

    #[test]
    fn a_table_where_an_array_of_tables_belongs_is_a_type_error() {
        // The reference surfaces the same refusal: `providers` is a slice, so
        // a table cannot decode onto it.
        let config: Result<Config, _> = toml::from_str("[providers]\nname = \"x\"\n");
        assert!(config.is_err());
    }

    #[test]
    fn unknown_keys_pass_through_silently() {
        // config.md §1: an unknown key is ignored, never a warning.
        let config: Config = toml::from_str(
            r#"
totally_unknown = "x"
[serve]
also_unknown = 3
[agent]
unknown_agent_key = true
[desktop.appearance]
zoom = 1.2
"#,
        )
        .expect("unknown keys decode");
        assert!(config.load_warnings.is_empty());
        assert_eq!(config.agent.compact_ratio, 0.90);
    }
}
