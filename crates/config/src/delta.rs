//! One layer's declared keys, applied field-wise onto the accumulated config.
//!
//! Every field is optional so "declared" and "built-in default" stay distinct:
//! the direct translation of the reference decoding each file onto the live
//! config, where BurntSushi writes only the keys a file contains.
//!
//! The user-globals rule (load_project.go userGlobals) is applied here: a
//! workspace file never sets [serve], [secrets], [storage], [telemetry], the
//! regional display keys, the command mode, or the agent/tools/sandbox keys the
//! reference marks user-only.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::agent::AgentSection;
use crate::desktop::{DesktopSection, TelemetrySection};
use crate::model::{BillingSection, Config};
use crate::plugin::PluginEntry;
use crate::provider::ProviderEntry;
use crate::sandbox::{PermissionsSection, SandboxSection};
use crate::secrets::SecretsSection;
use crate::serve::{AuthMode, ServeSection};
use crate::tools::ToolsSection;
use crate::ui::UiSection;

/// One file's declared keys.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LayerDelta {
    pub config_version: Option<i64>,
    pub default_model: Option<String>,
    pub language: Option<String>,
    pub credentials_store: Option<String>,
    pub ui: Option<PartialUi>,
    pub desktop: Option<PartialDesktop>,
    pub telemetry: Option<PartialTelemetry>,
    pub agent: Option<PartialAgent>,
    pub tools: Option<PartialTools>,
    pub permissions: Option<PartialPermissions>,
    pub sandbox: Option<PartialSandbox>,
    pub billing: Option<PartialBilling>,
    pub serve: Option<PartialServe>,
    pub secrets: Option<PartialSecrets>,
    pub storage: Option<BTreeMap<String, String>>,
    /// The lists are declared in the file for the cross-source merge, which
    /// owns their identity; a plain layer application ignores them.
    pub providers: Option<Vec<ProviderEntry>>,
    pub plugins: Option<Vec<PluginEntry>>,
}

impl LayerDelta {
    /// Applies this layer onto `config`. `project_scope` holds the
    /// user-globals rule for every key the reference marks user-only.
    pub fn apply(&self, config: &mut Config, project_scope: bool) {
        if let Some(value) = &self.config_version {
            config.config_version = *value;
        }
        if let Some(value) = &self.default_model {
            config.default_model = value.clone();
        }
        if let Some(value) = &self.language {
            config.language = value.clone();
        }
        if let Some(value) = &self.credentials_store {
            config.credentials_store = value.clone();
        }
        if let Some(ui) = &self.ui {
            ui.apply(&mut config.ui, project_scope);
        }
        if let Some(desktop) = &self.desktop {
            desktop.apply(&mut config.desktop, project_scope);
        }
        if let Some(agent) = &self.agent {
            agent.apply(&mut config.agent, project_scope);
        }
        if let Some(tools) = &self.tools {
            tools.apply(&mut config.tools, project_scope);
        }
        if let Some(permissions) = &self.permissions {
            permissions.apply(&mut config.permissions, project_scope);
        }
        if let Some(sandbox) = &self.sandbox {
            sandbox.apply(&mut config.sandbox, project_scope);
        }
        if !project_scope {
            if let Some(telemetry) = &self.telemetry {
                telemetry.apply(&mut config.telemetry);
            }
            if let Some(billing) = &self.billing {
                billing.apply(&mut config.billing);
            }
            if let Some(serve) = &self.serve {
                serve.apply(&mut config.serve);
            }
            if let Some(secrets) = &self.secrets {
                secrets.apply(&mut config.secrets);
            }
            if let Some(storage) = &self.storage {
                config.storage = storage.clone();
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialUi {
    pub theme: Option<String>,
    pub theme_style: Option<String>,
    pub shortcut_layout: Option<String>,
    pub close_behavior: Option<String>,
    pub show_reasoning: Option<bool>,
    pub show_turn_usage: Option<bool>,
    pub show_changed_files: Option<bool>,
    pub feedback_level_notices: Option<bool>,
    pub fold_turn_work: Option<bool>,
    pub show_receipt: Option<bool>,
    pub cursor_shape: Option<String>,
    pub command_mode: Option<String>,
}

impl PartialUi {
    fn apply(&self, ui: &mut UiSection, project_scope: bool) {
        assign(&mut ui.theme, &self.theme);
        assign(&mut ui.theme_style, &self.theme_style);
        assign(&mut ui.shortcut_layout, &self.shortcut_layout);
        assign(&mut ui.close_behavior, &self.close_behavior);
        assign(&mut ui.show_reasoning, &self.show_reasoning);
        assign(&mut ui.show_turn_usage, &self.show_turn_usage);
        assign(&mut ui.show_changed_files, &self.show_changed_files);
        assign(&mut ui.feedback_level_notices, &self.feedback_level_notices);
        assign(&mut ui.fold_turn_work, &self.fold_turn_work);
        assign(&mut ui.show_receipt, &self.show_receipt);
        assign(&mut ui.cursor_shape, &self.cursor_shape);
        // Key bindings are the user's: a repository does not retrain them.
        if !project_scope {
            assign(&mut ui.command_mode, &self.command_mode);
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialDesktop {
    pub telemetry: Option<bool>,
    pub metrics: Option<bool>,
    pub tray: Option<String>,
    pub language: Option<String>,
    pub currency: Option<String>,
    pub theme: Option<String>,
    pub theme_style: Option<String>,
    pub theme_pack: Option<String>,
    pub terminal_theme: Option<String>,
    pub close_behavior: Option<String>,
    pub display_mode: Option<String>,
    pub conversation_width: Option<String>,
    pub status_bar_style: Option<String>,
    pub status_bar_items: Option<Vec<String>>,
    pub default_tool_approval_mode: Option<String>,
    pub check_updates: Option<bool>,
    pub update_channel: Option<String>,
    pub update_mode: Option<String>,
    pub provider_access: Option<Vec<String>>,
    pub pinned_version: Option<String>,
    pub welcomed: Option<bool>,
    pub expand_thinking: Option<bool>,
    pub reasoning_display_mode: Option<String>,
}

impl PartialDesktop {
    fn apply(&self, desktop: &mut DesktopSection, project_scope: bool) {
        // Regional display is user-global: a repo cannot move the currency or
        // the desktop language.
        if !project_scope {
            assign(&mut desktop.language, &self.language);
            assign(&mut desktop.currency, &self.currency);
        }
        assign_optional(&mut desktop.telemetry, &self.telemetry);
        assign_optional(&mut desktop.metrics, &self.metrics);
        assign(&mut desktop.tray, &self.tray);
        assign(&mut desktop.theme, &self.theme);
        assign(&mut desktop.theme_style, &self.theme_style);
        assign(&mut desktop.theme_pack, &self.theme_pack);
        assign(&mut desktop.terminal_theme, &self.terminal_theme);
        assign(&mut desktop.close_behavior, &self.close_behavior);
        assign(&mut desktop.display_mode, &self.display_mode);
        assign(&mut desktop.conversation_width, &self.conversation_width);
        assign(&mut desktop.status_bar_style, &self.status_bar_style);
        assign(&mut desktop.status_bar_items, &self.status_bar_items);
        assign_optional(&mut desktop.check_updates, &self.check_updates);
        assign(&mut desktop.update_channel, &self.update_channel);
        assign_optional(&mut desktop.provider_access, &self.provider_access);
        // The approval posture and the update mode are the user's alone: a
        // cloned repository must not decide how this machine runs updates, nor
        // how its tools are approved.
        if !project_scope {
            assign(
                &mut desktop.default_tool_approval_mode,
                &self.default_tool_approval_mode,
            );
            assign(&mut desktop.update_mode, &self.update_mode);
        }
        assign(&mut desktop.pinned_version, &self.pinned_version);
        assign(&mut desktop.welcomed, &self.welcomed);
        assign(&mut desktop.expand_thinking, &self.expand_thinking);
        assign(
            &mut desktop.reasoning_display_mode,
            &self.reasoning_display_mode,
        );
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialTelemetry {
    pub cli_metrics: Option<String>,
}

impl PartialTelemetry {
    fn apply(&self, telemetry: &mut TelemetrySection) {
        assign(&mut telemetry.cli_metrics, &self.cli_metrics);
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialAgent {
    pub system_prompt: Option<String>,
    pub system_prompt_file: Option<String>,
    pub max_steps: Option<i64>,
    pub planner_max_steps: Option<i64>,
    pub temperature: Option<f64>,
    pub planner_model: Option<String>,
    pub title_model: Option<String>,
    pub guardian_model: Option<String>,
    pub guardian_temperature: Option<f64>,
    pub recovery_model: Option<String>,
    pub subagent_model: Option<String>,
    pub subagent_models: Option<BTreeMap<String, String>>,
    pub vision_model: Option<String>,
    pub web_search_model: Option<String>,
    pub triage_model: Option<String>,
    pub decision_model: Option<String>,
    pub advisor_model: Option<String>,
    pub screen_external_content: Option<bool>,
    pub subagent_effort: Option<String>,
    pub subagent_efforts: Option<BTreeMap<String, String>>,
    pub max_subagent_depth: Option<i64>,
    pub max_side_sessions: Option<i64>,
    pub max_queued_inputs: Option<i64>,
    pub max_provider_retries: Option<i64>,
    pub goal_token_budget: Option<i64>,
    pub max_subagent_concurrency: Option<i64>,
    pub max_parallel_writers: Option<i64>,
    pub subagent_claims: Option<String>,
    pub write_lease: Option<String>,
    pub plan_approval: Option<String>,
    pub default_mode: Option<String>,
    pub goal_auto_retry: Option<i64>,
    pub write_lease_max_wait_minutes: Option<i64>,
    pub output_style: Option<String>,
    pub auto_plan: Option<String>,
    pub reasoning_language: Option<String>,
    pub turn_narration: Option<String>,
    pub auto_plan_classifier: Option<String>,
    pub compact_ratio: Option<f64>,
    pub checkpoint_ceiling_ratio: Option<f64>,
    pub embedded_diff_detection: Option<bool>,
    pub plan_mode_read_only_commands: Option<Vec<String>>,
}

impl PartialAgent {
    fn apply(&self, agent: &mut AgentSection, project_scope: bool) {
        assign(&mut agent.system_prompt, &self.system_prompt);
        assign(&mut agent.system_prompt_file, &self.system_prompt_file);
        assign(&mut agent.max_steps, &self.max_steps);
        assign(&mut agent.planner_max_steps, &self.planner_max_steps);
        assign(&mut agent.temperature, &self.temperature);
        assign(&mut agent.planner_model, &self.planner_model);
        assign(&mut agent.title_model, &self.title_model);
        assign(&mut agent.guardian_model, &self.guardian_model);
        assign(&mut agent.guardian_temperature, &self.guardian_temperature);
        assign(&mut agent.recovery_model, &self.recovery_model);
        assign(&mut agent.subagent_model, &self.subagent_model);
        assign(&mut agent.subagent_models, &self.subagent_models);
        assign(&mut agent.vision_model, &self.vision_model);
        assign(&mut agent.web_search_model, &self.web_search_model);
        assign(&mut agent.triage_model, &self.triage_model);
        assign(&mut agent.decision_model, &self.decision_model);
        assign(&mut agent.advisor_model, &self.advisor_model);
        assign(
            &mut agent.screen_external_content,
            &self.screen_external_content,
        );
        assign(&mut agent.subagent_effort, &self.subagent_effort);
        assign(&mut agent.subagent_efforts, &self.subagent_efforts);
        assign(&mut agent.goal_token_budget, &self.goal_token_budget);
        assign(&mut agent.output_style, &self.output_style);
        assign(&mut agent.auto_plan, &self.auto_plan);
        assign(&mut agent.reasoning_language, &self.reasoning_language);
        assign(&mut agent.auto_plan_classifier, &self.auto_plan_classifier);
        assign(&mut agent.compact_ratio, &self.compact_ratio);
        assign(
            &mut agent.checkpoint_ceiling_ratio,
            &self.checkpoint_ceiling_ratio,
        );
        assign_optional(
            &mut agent.embedded_diff_detection,
            &self.embedded_diff_detection,
        );
        assign(
            &mut agent.plan_mode_read_only_commands,
            &self.plan_mode_read_only_commands,
        );
        // Delegation limits, the session mode, and the turn's voice are the
        // user's: a cloned repository cannot decide how the agent delegates.
        if !project_scope {
            assign(&mut agent.max_subagent_depth, &self.max_subagent_depth);
            assign(&mut agent.max_side_sessions, &self.max_side_sessions);
            assign(&mut agent.max_queued_inputs, &self.max_queued_inputs);
            assign(&mut agent.max_provider_retries, &self.max_provider_retries);
            assign(
                &mut agent.max_subagent_concurrency,
                &self.max_subagent_concurrency,
            );
            assign(&mut agent.max_parallel_writers, &self.max_parallel_writers);
            assign(&mut agent.subagent_claims, &self.subagent_claims);
            assign(&mut agent.write_lease, &self.write_lease);
            assign(&mut agent.plan_approval, &self.plan_approval);
            assign(&mut agent.default_mode, &self.default_mode);
            assign(&mut agent.goal_auto_retry, &self.goal_auto_retry);
            assign(
                &mut agent.write_lease_max_wait_minutes,
                &self.write_lease_max_wait_minutes,
            );
            assign(&mut agent.turn_narration, &self.turn_narration);
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialTools {
    pub enabled: Option<Vec<String>>,
    pub disabled: Option<Vec<String>>,
    pub bash_timeout_seconds: Option<i64>,
    pub mcp_startup_timeout_seconds: Option<i64>,
    pub mcp_call_timeout_seconds: Option<i64>,
    pub protect_changed_files: Option<bool>,
    pub browser_tools: Option<bool>,
    pub setting_tool: Option<bool>,
    pub mcp_resources: Option<bool>,
    pub present_files: Option<bool>,
    pub video_url_passthrough: Option<bool>,
    pub mcp_load: Option<BTreeMap<String, String>>,
}

impl PartialTools {
    fn apply(&self, tools: &mut ToolsSection, project_scope: bool) {
        assign(&mut tools.enabled, &self.enabled);
        if project_scope {
            // A project can remove tools for its own sessions and never take
            // one back (project_scope.go: disabled = user's ++ project's).
            extend_unique(&mut tools.disabled, &self.disabled);
        } else {
            assign(&mut tools.disabled, &self.disabled);
        }
        assign_optional(&mut tools.bash_timeout_seconds, &self.bash_timeout_seconds);
        assign_optional(
            &mut tools.mcp_startup_timeout_seconds,
            &self.mcp_startup_timeout_seconds,
        );
        assign_optional(
            &mut tools.mcp_call_timeout_seconds,
            &self.mcp_call_timeout_seconds,
        );
        assign_optional(
            &mut tools.protect_changed_files,
            &self.protect_changed_files,
        );
        assign_optional(&mut tools.browser_tools, &self.browser_tools);
        assign_optional(&mut tools.setting_tool, &self.setting_tool);
        assign(&mut tools.mcp_load, &self.mcp_load);
        if !project_scope {
            assign(&mut tools.mcp_resources, &self.mcp_resources);
            assign(&mut tools.present_files, &self.present_files);
            assign(
                &mut tools.video_url_passthrough,
                &self.video_url_passthrough,
            );
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialPermissions {
    pub mode: Option<String>,
    pub allow: Option<Vec<String>>,
    pub ask: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub allow_dynamic_bash: Option<bool>,
    pub ask_on_external_origin: Option<bool>,
}

impl PartialPermissions {
    fn apply(&self, permissions: &mut PermissionsSection, project_scope: bool) {
        if project_scope {
            // Narrowing only, without the held-scope machinery: a repository's
            // extra deny/ask rules join the user's, and it never grants.
            extend_unique(&mut permissions.deny, &self.deny);
            extend_unique(&mut permissions.ask, &self.ask);
            return;
        }
        assign(&mut permissions.mode, &self.mode);
        assign(&mut permissions.allow, &self.allow);
        assign(&mut permissions.ask, &self.ask);
        assign(&mut permissions.deny, &self.deny);
        assign(
            &mut permissions.allow_dynamic_bash,
            &self.allow_dynamic_bash,
        );
        assign(
            &mut permissions.ask_on_external_origin,
            &self.ask_on_external_origin,
        );
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialSandbox {
    pub workspace_root: Option<String>,
    pub allow_write: Option<Vec<String>>,
    pub forbid_read: Option<Vec<String>>,
    pub bash: Option<String>,
    pub windows_integrity: Option<bool>,
    pub network: Option<bool>,
    pub host_authorities: Option<Vec<String>>,
    pub allow_devices: Option<Vec<String>>,
    pub allowed_domains: Option<Vec<String>>,
    pub denied_domains: Option<Vec<String>>,
    pub egress_unlisted: Option<String>,
}

impl PartialSandbox {
    fn apply(&self, sandbox: &mut SandboxSection, project_scope: bool) {
        if project_scope {
            // Narrowing only: a repository may add read denials and egress
            // denials, and may not widen egress or move the workspace root.
            extend_unique(&mut sandbox.forbid_read, &self.forbid_read);
            extend_unique(&mut sandbox.denied_domains, &self.denied_domains);
            return;
        }
        assign(&mut sandbox.workspace_root, &self.workspace_root);
        assign(&mut sandbox.allow_write, &self.allow_write);
        assign(&mut sandbox.forbid_read, &self.forbid_read);
        assign(&mut sandbox.bash, &self.bash);
        assign(&mut sandbox.windows_integrity, &self.windows_integrity);
        assign(&mut sandbox.network, &self.network);
        assign(&mut sandbox.host_authorities, &self.host_authorities);
        assign(&mut sandbox.allow_devices, &self.allow_devices);
        assign(&mut sandbox.allowed_domains, &self.allowed_domains);
        assign(&mut sandbox.denied_domains, &self.denied_domains);
        assign(&mut sandbox.egress_unlisted, &self.egress_unlisted);
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialBilling {
    pub display_currency: Option<String>,
}

impl PartialBilling {
    fn apply(&self, billing: &mut BillingSection) {
        assign(&mut billing.display_currency, &self.display_currency);
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialServe {
    pub auth_mode: Option<String>,
    pub token: Option<String>,
    pub password_hash: Option<String>,
    pub behind_proxy: Option<bool>,
    pub share_port: Option<i64>,
    pub remember_paired_devices: Option<bool>,
}

impl PartialServe {
    fn apply(&self, serve: &mut ServeSection) {
        if let Some(raw) = &self.auth_mode {
            serve.auth_mode = AuthMode::normalize(raw);
        }
        assign(&mut serve.token, &self.token);
        assign(&mut serve.password_hash, &self.password_hash);
        assign(&mut serve.behind_proxy, &self.behind_proxy);
        assign(&mut serve.share_port, &self.share_port);
        assign(
            &mut serve.remember_paired_devices,
            &self.remember_paired_devices,
        );
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialSecrets {
    pub protect_sensitive_files: Option<bool>,
}

impl PartialSecrets {
    fn apply(&self, secrets: &mut SecretsSection) {
        assign(
            &mut secrets.protect_sensitive_files,
            &self.protect_sensitive_files,
        );
    }
}

/// Copies a declared value over the accumulated one, leaving the accumulated
/// value alone when the layer said nothing.
fn assign<T: Clone>(target: &mut T, declared: &Option<T>) {
    if let Some(value) = declared {
        *target = value.clone();
    }
}

/// The same for a target that is itself optional: an undeclared key leaves it
/// untouched, while a declared `true`/`false` replaces it.
fn assign_optional<T: Clone>(target: &mut Option<T>, declared: &Option<T>) {
    if declared.is_some() {
        *target = declared.clone();
    }
}

/// Appends the entries `declared` names that are not already present. Used for
/// the narrowing-only project keys, where an addition can restrict but never
/// grant.
fn extend_unique(target: &mut Vec<String>, declared: &Option<Vec<String>>) {
    let Some(declared) = declared else {
        return;
    };
    for name in declared {
        if !target.iter().any(|existing| existing == name) {
            target.push(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LayerDelta;
    use crate::model::Config;
    use crate::serve::AuthMode;

    fn delta(text: &str) -> LayerDelta {
        toml::from_str(text).expect("layer decodes")
    }

    #[test]
    fn an_empty_layer_changes_nothing() {
        let mut config = Config::builtin_defaults();
        let before = config.clone();
        delta("").apply(&mut config, false);
        assert_eq!(config, before);
    }

    #[test]
    fn a_layer_sets_only_the_keys_it_names() {
        let mut config = Config::builtin_defaults();
        delta("[desktop]\ntray = \"off\"\n").apply(&mut config, false);
        assert_eq!(config.desktop.tray, "off");
        // Untouched neighbours keep the built-in defaults.
        assert!(config.desktop.telemetry.is_none());
        assert_eq!(config.agent.compact_ratio, 0.90);
        assert_eq!(config.ui.theme, "auto");
    }

    #[test]
    fn later_layers_merge_field_wise_not_whole_section() {
        let mut config = Config::builtin_defaults();
        delta("[agent]\ncompact_ratio = 0.80\ntemperature = 0.2\n").apply(&mut config, false);
        delta("[agent]\ntemperature = 0.4\n").apply(&mut config, false);
        assert_eq!(config.agent.compact_ratio, 0.80);
        assert_eq!(config.agent.temperature, 0.4);
    }

    #[test]
    fn the_project_layer_cannot_set_user_global_keys() {
        // load_project.go userGlobals, one representative per table.
        let mut config = Config::builtin_defaults();
        config.serve.auth_mode = AuthMode::Token;
        config.serve.token = "user-token".to_owned();
        config.secrets.protect_sensitive_files = true;
        config
            .storage
            .insert("state".to_owned(), "C:/state".to_owned());
        config.telemetry.cli_metrics = "off".to_owned();
        config.ui.command_mode = "vi".to_owned();
        config.desktop.language = "en".to_owned();
        config.desktop.currency = "USD".to_owned();
        config.billing.display_currency = "USD".to_owned();
        config.agent.default_mode = "normal".to_owned();
        config.tools.present_files = true;
        config.sandbox.network = true;

        let project = delta(
            r#"
[serve]
auth_mode = "none"
token = "project-token"
behind_proxy = true
[secrets]
protect_sensitive_files = false
[storage]
state = "D:/elsewhere"
[telemetry]
cli_metrics = "on"
[ui]
command_mode = ""
[desktop]
language = "zh"
currency = "CNY"
theme = "dark"
[billing]
display_currency = "CNY"
[agent]
default_mode = "plan"
compact_ratio = 0.5
[tools]
present_files = false
[sandbox]
network = false
"#,
        );
        project.apply(&mut config, true);

        assert_eq!(config.serve.auth_mode, AuthMode::Token);
        assert_eq!(config.serve.token, "user-token");
        assert!(!config.serve.behind_proxy);
        assert!(config.secrets.protect_sensitive_files);
        assert_eq!(
            config.storage.get("state").map(String::as_str),
            Some("C:/state")
        );
        assert_eq!(config.telemetry.mode(), "off");
        assert_eq!(config.ui.command_mode, "vi");
        assert_eq!(config.desktop.language, "en");
        assert_eq!(config.desktop.currency, "USD");
        assert_eq!(config.billing.display_currency, "USD");
        assert_eq!(config.agent.default_mode, "normal");
        assert!(config.tools.present_files);
        assert!(config.sandbox.network);
        // Keys the reference does not hold are still the project's.
        assert_eq!(config.desktop.theme, "dark");
        assert_eq!(config.agent.compact_ratio, 0.5);
    }

    #[test]
    fn the_project_layer_only_narrows_permissions_and_sandbox() {
        // heldScope.narrow: a repository's additions can restrict, never grant.
        let mut config = Config::builtin_defaults();
        let project = delta(
            r#"
[permissions]
mode = "allow"
allow = ["Bash(*)"]
deny = ["Bash(rm -rf *)"]
ask = ["Write(*)"]
[sandbox]
network = true
forbid_read = ["/etc"]
denied_domains = ["evil.example"]
allow_write = ["/"]
allowed_domains = ["anything.example"]
"#,
        );
        project.apply(&mut config, true);
        assert_eq!(config.permissions.mode, "ask");
        assert!(config.permissions.allow.is_empty());
        assert_eq!(config.permissions.deny, vec!["Bash(rm -rf *)"]);
        assert_eq!(config.permissions.ask, vec!["Write(*)"]);
        assert!(config.sandbox.allow_write.is_empty());
        assert!(config.sandbox.allowed_domains.is_empty());
        assert_eq!(config.sandbox.forbid_read, vec!["/etc"]);
        assert_eq!(config.sandbox.denied_domains, vec!["evil.example"]);
    }

    #[test]
    fn a_project_can_add_to_the_disabled_tool_list_and_never_replace_it() {
        // project_scope.go narrow: disabled = user's ++ project's, so a
        // checkout cannot re-enable a tool the user removed.
        let mut config = Config::builtin_defaults();
        delta("[tools]\ndisabled = [\"Bash\"]\n").apply(&mut config, false);
        delta("[tools]\ndisabled = [\"Write\", \"Bash\"]\n").apply(&mut config, true);
        assert_eq!(
            config.tools.disabled,
            vec!["Bash".to_owned(), "Write".to_owned()]
        );
    }

    #[test]
    fn the_approval_posture_and_update_mode_are_the_users_alone() {
        // project_scope.go narrow: default_tool_approval_mode and update_mode
        // are restored from the user's held scope.
        let mut config = Config::builtin_defaults();
        config.desktop.default_tool_approval_mode = "ask".to_owned();
        config.desktop.update_mode = "notify".to_owned();
        delta("[desktop]\ndefault_tool_approval_mode = \"yolo\"\nupdate_mode = \"download\"\n")
            .apply(&mut config, true);
        assert_eq!(config.desktop.default_tool_approval_mode, "ask");
        assert_eq!(config.desktop.update_mode, "notify");
    }

    #[test]
    fn the_user_layer_applies_every_key_including_the_globals() {
        let mut config = Config::builtin_defaults();
        delta(
            r#"
default_model = "gateway/m"
language = "zh"
[serve]
auth_mode = "  TOKEN  "
share_port = 8090
[secrets]
protect_sensitive_files = true
[storage]
state = "~/state"
[telemetry]
cli_metrics = "on"
[ui]
command_mode = "vi"
[desktop]
language = "zh"
currency = "cny"
[billing]
display_currency = "CNY"
[agent]
default_mode = "plan"
max_provider_retries = 3
[tools]
present_files = false
[sandbox]
network = false
"#,
        )
        .apply(&mut config, false);
        assert_eq!(config.default_model, "gateway/m");
        assert_eq!(config.language, "zh");
        assert_eq!(config.serve.auth_mode, AuthMode::Token);
        assert_eq!(config.serve.share_port, 8090);
        assert!(config.secrets.protect_sensitive_files);
        assert_eq!(
            config.storage.get("state").map(String::as_str),
            Some("~/state")
        );
        assert_eq!(config.telemetry.mode(), "on");
        assert_eq!(config.ui.command_mode, "vi");
        assert_eq!(config.desktop.language(), "zh");
        assert_eq!(config.desktop.currency(), "CNY");
        assert_eq!(config.billing.display_currency, "CNY");
        assert_eq!(config.agent.default_mode, "plan");
        assert_eq!(config.agent.max_provider_retries, 3);
        assert!(!config.tools.present_files);
        assert!(!config.sandbox.network);
    }

    #[test]
    fn an_explicit_empty_value_overrides_an_earlier_one() {
        // Decoding onto the live config: a declared "" wins over "zh".
        let mut config = Config::builtin_defaults();
        delta("language = \"zh\"\n").apply(&mut config, false);
        delta("language = \"\"\n").apply(&mut config, false);
        assert_eq!(config.language, "");
    }
}
