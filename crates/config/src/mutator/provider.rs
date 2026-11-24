//! Provider and plugin mutators (CONFIG-026, `edit.go:127-155,175-183,703-744`,
//! `provider_entry.go:175-203`, `edit.go:1105-1136`).
//!
//! Validation runs before the in-memory write; runtime credential resolution,
//! pricing backfill and source-locked installs stay wip.

use thiserror::Error;

use super::MutatorError;
use crate::model::Config;
use crate::plugin::PluginEntry;
use crate::provider::ProviderEntry;

/// Declared balance response shapes (`walletformat.All`).
const BALANCE_FORMATS: [&str; 3] = ["deepseek", "moonshot", "openrouter"];
const BALANCE_FORMAT_LIST: &str = "deepseek, moonshot, openrouter";
/// Provider idle-timeout bounds (`contract/provider/streaminterrupt.go:33-34`).
const MIN_IDLE_TIMEOUT_SECONDS: i64 = 1;
const MAX_IDLE_TIMEOUT_SECONDS: i64 = 32_767;

/// `[providers]` entry validation and lookup errors.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider: name is required")]
    NameRequired,

    #[error("provider {name:?}: kind is required")]
    KindRequired { name: String },

    #[error("provider {name:?}: base_url is required")]
    BaseURLRequired { name: String },

    #[error("provider {name:?}: model is required")]
    ModelRequired { name: String },

    #[error("provider {name:?}: api_key_env {env:?} is not a valid environment variable name")]
    APIKeyEnvInvalid { name: String, env: String },

    #[error("provider {name:?}: balance_format {format:?} is not one of {BALANCE_FORMAT_LIST}")]
    BalanceFormatInvalid { name: String, format: String },

    #[error(
        "provider {name:?}: idle_timeout_seconds must be 0 (use the default) or between {MIN_IDLE_TIMEOUT_SECONDS} and {MAX_IDLE_TIMEOUT_SECONDS}"
    )]
    IdleTimeoutOutOfRange { name: String },

    #[error("set provider effort: no provider {name:?}")]
    EffortNotFound { name: String },

    #[error("set provider display name {name:?}: no such provider")]
    DisplayNameNotFound { name: String },

    #[error("provider display name is too long")]
    DisplayNameTooLong,

    #[error("provider display name contains a control character")]
    DisplayNameInvalidControl,

    #[error("remove provider {name:?}: no such provider")]
    RemoveNotFound { name: String },
}

impl ProviderError {
    /// Stable Go sentinel/kind identifier.
    pub fn ident(&self) -> &'static str {
        match self {
            Self::NameRequired => "ProviderNameRequired",
            Self::KindRequired { .. } => "ProviderKindRequired",
            Self::BaseURLRequired { .. } => "ProviderBaseURLRequired",
            Self::ModelRequired { .. } => "ProviderModelRequired",
            Self::APIKeyEnvInvalid { .. } => "ProviderAPIKeyEnvInvalid",
            Self::BalanceFormatInvalid { .. } => "ProviderBalanceFormatInvalid",
            Self::IdleTimeoutOutOfRange { .. } => "ProviderIdleTimeoutOutOfRange",
            Self::EffortNotFound { .. } => "ErrProviderNotFound",
            Self::DisplayNameNotFound { .. } => "ErrProviderNotFound",
            Self::DisplayNameTooLong => "ErrProviderDisplayNameTooLong",
            Self::DisplayNameInvalidControl => "ErrProviderDisplayNameInvalid",
            Self::RemoveNotFound { .. } => "ErrProviderNotFound",
        }
    }
}

fn is_credential_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|r| r.is_ascii_alphanumeric() || r == '_')
}

fn provider_has_any_model(entry: &ProviderEntry) -> bool {
    !entry.model.trim().is_empty() || !entry.models.is_empty() || !entry.model_aliases.is_empty()
}

/// Validates one provider entry (`provider_entry.go:175-193`).
pub fn validate_provider(entry: &ProviderEntry) -> Result<(), ProviderError> {
    let name = entry.name.trim().to_owned();
    let kind = entry.kind.trim().to_owned();
    let base_url = entry.base_url.trim().to_owned();
    match () {
        _ if name.is_empty() => Err(ProviderError::NameRequired),
        _ if kind.is_empty() => Err(ProviderError::KindRequired { name }),
        _ if base_url.is_empty() => Err(ProviderError::BaseURLRequired { name }),
        _ if !provider_has_any_model(entry) => Err(ProviderError::ModelRequired { name }),
        _ if !entry.api_key_env.trim().is_empty() && !is_credential_key(&entry.api_key_env) => {
            Err(ProviderError::APIKeyEnvInvalid {
                name,
                env: entry.api_key_env.clone(),
            })
        }
        _ if !entry.balance_format.is_empty()
            && !BALANCE_FORMATS.contains(&entry.balance_format.as_str()) =>
        {
            Err(ProviderError::BalanceFormatInvalid {
                name,
                format: entry.balance_format.clone(),
            })
        }
        _ if entry.idle_timeout_seconds != 0
            && !(MIN_IDLE_TIMEOUT_SECONDS..=MAX_IDLE_TIMEOUT_SECONDS)
                .contains(&entry.idle_timeout_seconds) =>
        {
            Err(ProviderError::IdleTimeoutOutOfRange { name })
        }
        _ => Ok(()),
    }
}

/// `[[plugins]]` entry validation errors (`edit.go:1105-1136`).
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin: name is required")]
    NameRequired,

    #[error("plugin {name:?}: startup_timeout_seconds must be >= 0")]
    StartupTimeoutNegative { name: String },

    #[error("plugin {name:?}: call_timeout_seconds must be >= 0")]
    CallTimeoutNegative { name: String },

    #[error("plugin {name:?}: tool_timeout_seconds contains an empty tool name")]
    ToolTimeoutEmptyName { name: String },

    #[error("plugin {name:?}: tool_timeout_seconds[{tool:?}] must be >= 0")]
    ToolTimeoutNegative { name: String, tool: String },

    #[error("plugin {name:?}: command is required for a stdio server")]
    CommandRequired { name: String },

    #[error("plugin {name:?}: url is required for a {kind} server")]
    URLRequired { name: String, kind: String },

    #[error("plugin {name:?}: unknown type {kind:?} (want stdio|http|sse)")]
    UnknownType { name: String, kind: String },

    #[error("plugin {name:?}: disabled_tools contains an empty tool name")]
    DisabledToolEmpty { name: String },

    #[error("clear plugin authentication: no plugin {name:?}")]
    ClearAuthNotFound { name: String },
}

impl PluginError {
    /// Stable Go sentinel/kind identifier.
    pub fn ident(&self) -> &'static str {
        match self {
            Self::NameRequired => "PluginNameRequired",
            Self::StartupTimeoutNegative { .. } => "PluginStartupTimeoutNegative",
            Self::CallTimeoutNegative { .. } => "PluginCallTimeoutNegative",
            Self::ToolTimeoutEmptyName { .. } => "PluginToolTimeoutEmptyName",
            Self::ToolTimeoutNegative { .. } => "PluginToolTimeoutNegative",
            Self::CommandRequired { .. } => "PluginCommandRequired",
            Self::URLRequired { .. } => "PluginURLRequired",
            Self::UnknownType { .. } => "PluginUnknownType",
            Self::DisabledToolEmpty { .. } => "PluginDisabledToolEmpty",
            Self::ClearAuthNotFound { .. } => "ClearAuthNotFound",
        }
    }
}

/// Validates one plugin entry (`edit.go:1105-1136`).
pub fn validate_plugin(entry: &PluginEntry) -> Result<(), PluginError> {
    let name = entry.name.trim().to_owned();
    if name.is_empty() {
        return Err(PluginError::NameRequired);
    }
    if entry.startup_timeout_seconds < 0 {
        return Err(PluginError::StartupTimeoutNegative { name });
    }
    if entry.call_timeout_seconds < 0 {
        return Err(PluginError::CallTimeoutNegative { name });
    }
    for (tool, seconds) in &entry.tool_timeout_seconds {
        if tool.trim().is_empty() {
            return Err(PluginError::ToolTimeoutEmptyName { name });
        }
        if *seconds < 0 {
            return Err(PluginError::ToolTimeoutNegative {
                name,
                tool: tool.clone(),
            });
        }
    }
    match entry.kind.trim().to_lowercase().as_str() {
        "" | "stdio" => {
            if entry.command.trim().is_empty() {
                return Err(PluginError::CommandRequired { name });
            }
        }
        "http" | "sse" | "streamable-http" => {
            if entry.url.trim().is_empty() {
                return Err(PluginError::URLRequired {
                    name,
                    kind: entry.kind.clone(),
                });
            }
        }
        other => {
            return Err(PluginError::UnknownType {
                name,
                kind: other.to_owned(),
            });
        }
    }
    if entry
        .disabled_tools
        .iter()
        .any(|tool| tool.trim().is_empty())
    {
        return Err(PluginError::DisabledToolEmpty { name });
    }
    Ok(())
}

impl Config {
    /// Validates and inserts a provider; an existing name keeps its position
    /// (`edit.go:127-140`).
    pub fn upsert_provider(&mut self, entry: ProviderEntry) -> Result<(), MutatorError> {
        validate_provider(&entry)?;
        match self
            .providers
            .iter()
            .position(|existing| existing.name == entry.name)
        {
            Some(pos) => self.providers[pos] = entry,
            None => self.providers.push(entry),
        }
        Ok(())
    }

    /// Sets the stored effort on a named provider (`edit.go:175-183`).
    pub fn set_provider_effort(&mut self, name: &str, effort: &str) -> Result<(), MutatorError> {
        let provider = self
            .providers
            .iter_mut()
            .find(|existing| existing.name.as_str() == name)
            .ok_or_else(|| ProviderError::EffortNotFound {
                name: name.to_owned(),
            })?;
        provider.effort = effort.trim().to_owned();
        Ok(())
    }

    /// Renames providers in one all-or-none pass
    /// (`provider_display_name.go:43-60`).
    pub fn set_provider_display_name(
        &mut self,
        names: &[String],
        display_name: &str,
    ) -> Result<(), MutatorError> {
        if display_name.chars().count() > 64 {
            return Err(ProviderError::DisplayNameTooLong.into());
        }
        if display_name.chars().any(|r| r.is_control()) {
            return Err(ProviderError::DisplayNameInvalidControl.into());
        }
        let targets: Vec<usize> = names
            .iter()
            .map(|name| {
                self.providers
                    .iter()
                    .position(|existing| existing.name.as_str() == name)
                    .ok_or_else(|| ProviderError::DisplayNameNotFound { name: name.clone() })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for pos in targets {
            self.providers[pos].display_name = display_name.to_owned();
        }
        Ok(())
    }

    /// Removes a provider. Re-routing default/planner/subagent references and
    /// dropping it from `desktop.provider_access` is a wip (`provider_remove.go`).
    pub fn remove_provider(&mut self, name: &str) -> Result<(), MutatorError> {
        let pos = self
            .providers
            .iter()
            .position(|existing| existing.name.as_str() == name)
            .ok_or_else(|| ProviderError::RemoveNotFound {
                name: name.to_owned(),
            })?;
        self.providers.remove(pos);
        Ok(())
    }

    /// Validates and inserts a plugin; an existing name keeps its position
    /// (`edit.go:703-716`).
    pub fn upsert_plugin(&mut self, entry: PluginEntry) -> Result<(), MutatorError> {
        validate_plugin(&entry)?;
        match self
            .plugins
            .iter()
            .position(|existing| existing.name == entry.name)
        {
            Some(pos) => self.plugins[pos] = entry,
            None => self.plugins.push(entry),
        }
        Ok(())
    }

    /// Removes a plugin by name; reports whether one existed (`edit.go:719-727`).
    pub fn remove_plugin(&mut self, name: &str) -> bool {
        let Some(pos) = self
            .plugins
            .iter()
            .position(|existing| existing.name == name)
        else {
            return false;
        };
        self.plugins.remove(pos);
        true
    }

    /// Strips authentication material from a plugin's headers/env/url
    /// (`edit.go:732-744`). Reports whether anything changed.
    pub fn clear_plugin_authentication(&mut self, name: &str) -> Result<bool, MutatorError> {
        let plugin = self
            .plugins
            .iter_mut()
            .find(|existing| existing.name.as_str() == name)
            .ok_or_else(|| PluginError::ClearAuthNotFound {
                name: name.to_owned(),
            })?;
        let changed =
            !plugin.headers.is_empty() || !plugin.env.is_empty() || !plugin.url.is_empty();
        plugin.headers.clear();
        plugin.env.clear();
        plugin.url.clear();
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_provider() -> ProviderEntry {
        ProviderEntry {
            name: "gw".to_owned(),
            kind: "openai".to_owned(),
            base_url: "https://example.com".to_owned(),
            model: "m".to_owned(),
            ..ProviderEntry::default()
        }
    }

    #[test]
    fn provider_validation_reports_each_missing_field_in_order() {
        let err = validate_provider(&ProviderEntry::default()).unwrap_err();
        assert_eq!(err.to_string(), "provider: name is required");
        let mut p = valid_provider();
        p.kind = String::new();
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(err.to_string(), r#"provider "gw": kind is required"#);
        p = valid_provider();
        p.base_url = String::new();
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(err.to_string(), r#"provider "gw": base_url is required"#);
        p = valid_provider();
        p.model = String::new();
        p.models = Vec::new();
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(err.to_string(), r#"provider "gw": model is required"#);
    }

    #[test]
    fn provider_api_key_env_balance_format_and_idle_timeout() {
        let mut p = valid_provider();
        p.api_key_env = "1BAD".to_owned();
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"provider "gw": api_key_env "1BAD" is not a valid environment variable name"#
        );
        p = valid_provider();
        p.balance_format = "weird".to_owned();
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"provider "gw": balance_format "weird" is not one of deepseek, moonshot, openrouter"#
        );
        p = valid_provider();
        p.idle_timeout_seconds = 99_999;
        let err = validate_provider(&p).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"provider "gw": idle_timeout_seconds must be 0 (use the default) or between 1 and 32767"#
        );
    }

    #[test]
    fn provider_upsert_replaces_in_place_and_missing_effort_errors() {
        let mut config = Config::default();
        config.upsert_provider(valid_provider()).unwrap();
        let mut second = valid_provider();
        second.display_name = "Gateway".to_owned();
        config.upsert_provider(second).unwrap();
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].display_name, "Gateway");
        let err = config.set_provider_effort("nope", "high").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"set provider effort: no provider "nope""#
        );
    }

    #[test]
    fn provider_display_name_is_all_or_none_and_length_checked() {
        let mut config = Config::default();
        config.upsert_provider(valid_provider()).unwrap();
        let err = config
            .set_provider_display_name(&["missing".to_owned()], "X")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"set provider display name "missing": no such provider"#
        );
        let long = "x".repeat(65);
        let err = config
            .set_provider_display_name(&["gw".to_owned()], &long)
            .unwrap_err();
        assert_eq!(err.to_string(), "provider display name is too long");
        config
            .set_provider_display_name(&["gw".to_owned()], "Gateway")
            .unwrap();
        assert_eq!(config.providers[0].display_name, "Gateway");
    }

    #[test]
    fn plugin_validation_covers_transport_and_timeouts() {
        let err = validate_plugin(&PluginEntry::default()).unwrap_err();
        assert_eq!(err.to_string(), "plugin: name is required");
        let mut pl = PluginEntry {
            name: "srv".to_owned(),
            command: "npx".to_owned(),
            ..PluginEntry::default()
        };
        pl.startup_timeout_seconds = -1;
        let err = validate_plugin(&pl).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"plugin "srv": startup_timeout_seconds must be >= 0"#
        );
        pl = PluginEntry {
            name: "srv".to_owned(),
            kind: "http".to_owned(),
            ..PluginEntry::default()
        };
        let err = validate_plugin(&pl).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"plugin "srv": url is required for a http server"#
        );
        pl.kind = "weird".to_owned();
        let err = validate_plugin(&pl).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"plugin "srv": unknown type "weird" (want stdio|http|sse)"#
        );
    }

    #[test]
    fn plugin_upsert_remove_and_auth_clear() {
        let mut config = Config::default();
        let pl = PluginEntry {
            name: "srv".to_owned(),
            command: "npx".to_owned(),
            url: "https://x".to_owned(),
            ..PluginEntry::default()
        };
        config.upsert_plugin(pl).unwrap();
        assert!(config.remove_plugin("srv"));
        assert!(!config.remove_plugin("srv"));
    }
}
