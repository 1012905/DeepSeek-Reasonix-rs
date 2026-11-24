//! In-memory mutator face (CONFIG-026, config.md §15.1–§15.2; `edit.go` and the
//! per-domain small mutator files).
//!
//! Every mutator changes only the in-memory [`Config`]; disk writes, source
//! resolution and locking stay wip (CONFIG-027/029/031). Errors are typed: the
//! variant carries a stable `ident` mirroring the Go `Err*` sentinel, and the
//! Display string is the reference message verbatim, with `%q`/`%v`/`%d`
//! filled in.

pub mod desktop;
pub mod provider;

use thiserror::Error;

use crate::model::Config;

/// Typed mutator error. `ident` is the stable sentinel/kind for callers that
/// match on the Go `Err*` names; the Display message is the frozen wording.
#[derive(Debug, Error)]
pub enum MutatorError {
    #[error("set default: empty name")]
    EmptyDefaultName,

    #[error("set default: no such model {name:?} (configured: {configured})")]
    NoSuchModel { name: String, configured: String },

    #[error("automatic plan mode has been retired; use Plan Mode explicitly")]
    AutoPlanRetired,

    #[error("default_mode {mode:?}: want normal or plan")]
    DefaultModeInvalid { mode: String },

    #[error("permission mode {mode:?}: must be ask|allow|deny")]
    PermissionModeInvalid { mode: String },

    #[error("unknown permission list {list:?} (want allow|ask|deny)")]
    UnknownPermissionList { list: String },

    #[error("invalid permission rule {rule:?} (want \"ToolName\" or \"ToolName(glob)\")")]
    InvalidPermissionRule { rule: String },

    #[error("skill path: empty path")]
    SkillPathEmpty,

    #[error(
        "skill name {name:?}: use Unicode letters or numbers and '_', '-', '.', 1-64 characters, starting alphanumeric"
    )]
    SkillNameInvalid { name: String },

    #[error("reasoning language {lang:?}: must be auto|zh|en")]
    ReasoningLanguageInvalid { lang: String },

    #[error("language {lang:?}: must be auto|en|zh")]
    LanguageInvalid { lang: String },

    #[error("desktop language {lang:?}: must be auto|en|zh")]
    DesktopLanguageInvalid { lang: String },

    #[error(
        "compact ratio outside the open window fraction range: {value} must be above {lo} and below {hi}"
    )]
    CompactRatioRange { value: f64, lo: f64, hi: f64 },

    #[error("share port out of range: use 0 or a port between {lo} and {hi}")]
    SharePortOutOfRange { lo: i64, hi: i64 },

    #[error(
        "session reference limit out of range: tokens must be 0 (default) or between {lo} and {hi}, not {n}"
    )]
    SessionRefTokensOutOfRange { lo: i64, hi: i64, n: i64 },

    #[error(
        "ask timeout out of range: {raw:?} is not a whole number of seconds (0 or {lo} to {hi})"
    )]
    AskTimeoutNotWhole { raw: String, lo: i64, hi: i64 },

    #[error("ask timeout out of range: seconds must be 0 (off) or between {lo} and {hi}, not {n}")]
    AskTimeoutOutOfRange { lo: i64, hi: i64, n: i64 },

    #[error("memory.proactive {mode:?}: must be ask|auto|off")]
    MemoryProactiveInvalid { mode: String },

    #[error("shell prefer {prefer:?}: must be auto|bash|powershell|pwsh")]
    ShellPreferInvalid { prefer: String },

    #[error("mcp load: server name is required")]
    MCPLoadNameRequired,

    #[error("mcp load {name:?}: want {always:?} or {deferred:?}")]
    MCPLoadInvalid {
        name: String,
        always: String,
        deferred: String,
    },

    #[error("plan_mode_read_only_commands: command entries must be non-empty")]
    PlanModeReadOnlyCommandEmpty,

    #[error("subagent_models: entry {entry:?} must be non-empty")]
    SubagentModelsEmpty { entry: String },

    #[error("subagent_efforts: entry {entry:?} must be non-empty")]
    SubagentEffortsEmpty { entry: String },

    #[error("auto archive setting out of range: days must be between 1 and {max}")]
    AutoArchiveOutOfRange { max: i64 },

    #[error("progress watch setting out of range: rounds must be between 1 and {max}")]
    ProgressWatchRoundsOutOfRange { max: i64 },

    #[error("progress watch setting out of range: token_multiple must be between 1 and {max}")]
    ProgressWatchMultipleOutOfRange { max: i64 },

    #[error("persona is too large: prefix and suffix must each be at most 32 KiB")]
    PersonaTooLarge,

    #[error("{0}")]
    Provider(provider::ProviderError),

    #[error("{0}")]
    Plugin(provider::PluginError),

    #[error("{0}")]
    Desktop(desktop::DesktopError),
}

impl From<provider::ProviderError> for MutatorError {
    fn from(value: provider::ProviderError) -> Self {
        Self::Provider(value)
    }
}

impl From<provider::PluginError> for MutatorError {
    fn from(value: provider::PluginError) -> Self {
        Self::Plugin(value)
    }
}

impl From<desktop::DesktopError> for MutatorError {
    fn from(value: desktop::DesktopError) -> Self {
        Self::Desktop(value)
    }
}

impl MutatorError {
    /// Stable Go sentinel/kind identifier for matching and reports.
    pub fn ident(&self) -> &'static str {
        match self {
            Self::EmptyDefaultName => "EmptyDefaultName",
            Self::NoSuchModel { .. } => "NoSuchModel",
            Self::AutoPlanRetired => "AutoPlanRetired",
            Self::DefaultModeInvalid { .. } => "DefaultModeInvalid",
            Self::PermissionModeInvalid { .. } => "PermissionModeInvalid",
            Self::UnknownPermissionList { .. } => "UnknownPermissionList",
            Self::InvalidPermissionRule { .. } => "InvalidPermissionRule",
            Self::SkillPathEmpty => "SkillPathEmpty",
            Self::SkillNameInvalid { .. } => "SkillNameInvalid",
            Self::ReasoningLanguageInvalid { .. } => "ReasoningLanguageInvalid",
            Self::LanguageInvalid { .. } => "LanguageInvalid",
            Self::DesktopLanguageInvalid { .. } => "DesktopLanguageInvalid",
            Self::CompactRatioRange { .. } => "ErrCompactRatioRange",
            Self::SharePortOutOfRange { .. } => "ErrSharePortOutOfRange",
            Self::SessionRefTokensOutOfRange { .. } => "ErrSessionRefTokensOutOfRange",
            Self::AskTimeoutNotWhole { .. } => "ErrAskTimeoutOutOfRange",
            Self::AskTimeoutOutOfRange { .. } => "ErrAskTimeoutOutOfRange",
            Self::MemoryProactiveInvalid { .. } => "MemoryProactiveInvalid",
            Self::ShellPreferInvalid { .. } => "ShellPreferInvalid",
            Self::MCPLoadNameRequired => "MCPLoadNameRequired",
            Self::MCPLoadInvalid { .. } => "MCPLoadInvalid",
            Self::PlanModeReadOnlyCommandEmpty => "PlanModeReadOnlyCommandEmpty",
            Self::SubagentModelsEmpty { .. } => "SubagentModelsEmpty",
            Self::SubagentEffortsEmpty { .. } => "SubagentEffortsEmpty",
            Self::AutoArchiveOutOfRange { .. } => "ErrAutoArchiveOutOfRange",
            Self::ProgressWatchRoundsOutOfRange { .. } => "ErrProgressWatchOutOfRange",
            Self::ProgressWatchMultipleOutOfRange { .. } => "ErrProgressWatchOutOfRange",
            Self::PersonaTooLarge => "ErrPersonaTooLarge",
            Self::Provider(e) => e.ident(),
            Self::Plugin(e) => e.ident(),
            Self::Desktop(e) => e.ident(),
        }
    }
}

/// Shared bounds from the reference (small mutator file headers).
pub mod bounds {
    pub const COMPACT_RATIO_MIN: f64 = 0.0;
    pub const COMPACT_RATIO_MAX: f64 = 1.0;
    pub const SHARE_PORT_MIN: i64 = 1024;
    pub const SHARE_PORT_MAX: i64 = 65535;
    pub const SESSION_REF_TOKENS_MIN: i64 = 500;
    pub const SESSION_REF_TOKENS_MAX: i64 = 200_000;
    pub const ASK_TIMEOUT_MIN: i64 = 10;
    pub const ASK_TIMEOUT_MAX: i64 = 86_400;
    pub const AUTO_ARCHIVE_MAX_DAYS: i64 = 3_650;
    pub const PROGRESS_WATCH_MAX: i64 = 1_000;
    pub const PERSONA_HALF_LIMIT_BYTES: usize = 32 * 1024;
}

// ---------------------------------------------------------------------------
// Agent section validators.
// ---------------------------------------------------------------------------

/// Normalizes `default_mode`: only `normal` and `plan` are accepted
/// (`default_mode.go:16-26`).
pub fn normalize_default_mode(raw: &str) -> Result<&'static str, MutatorError> {
    match raw.trim().to_lowercase().as_str() {
        "normal" => Ok("normal"),
        "plan" => Ok("plan"),
        other => Err(MutatorError::DefaultModeInvalid {
            mode: other.to_owned(),
        }),
    }
}

/// Validates the open-window compact ratio `(0.0, 1.0)` (`edit_compaction.go:27-33`).
pub fn validate_compact_ratio(ratio: f64) -> Result<(), MutatorError> {
    if ratio > bounds::COMPACT_RATIO_MIN && ratio < bounds::COMPACT_RATIO_MAX {
        Ok(())
    } else {
        Err(MutatorError::CompactRatioRange {
            value: ratio,
            lo: bounds::COMPACT_RATIO_MIN,
            hi: bounds::COMPACT_RATIO_MAX,
        })
    }
}

/// Validates `reasoning_language`: auto|zh|en, with the model-following
/// aliases resolving to auto (`edit.go:203-215`).
pub fn normalize_reasoning_language(raw: &str) -> Result<String, MutatorError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" | "follow" | "conversation" | "detect" | "default" | "model"
        | "model-default" | "model_default" | "provider" => Ok(String::new()),
        "zh" | "cn" | "chinese" | "中文" => Ok("zh".to_owned()),
        "en" | "english" => Ok("en".to_owned()),
        other => Err(MutatorError::ReasoningLanguageInvalid {
            lang: other.to_owned(),
        }),
    }
}

/// Validates the top-level `language`: auto|en|zh (`edit.go:186-199`).
pub fn normalize_language(raw: &str) -> Result<String, MutatorError> {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" => Ok(String::new()),
        "en" => Ok("en".to_owned()),
        "zh" => Ok("zh".to_owned()),
        other => Err(MutatorError::LanguageInvalid {
            lang: other.to_owned(),
        }),
    }
}

/// Validates a `subagent_models` entry: the model key must not be blank
/// (`edit.go:326-330`).
pub fn validate_subagent_models(
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), MutatorError> {
    map.keys()
        .find(|entry| entry.trim().is_empty())
        .map(|entry| {
            Err(MutatorError::SubagentModelsEmpty {
                entry: entry.clone(),
            })
        })
        .unwrap_or(Ok(()))
}

/// Validates a `subagent_efforts` entry: the role key must not be blank
/// (`edit.go:332-336`).
pub fn validate_subagent_efforts(
    map: &std::collections::BTreeMap<String, String>,
) -> Result<(), MutatorError> {
    map.keys()
        .find(|entry| entry.trim().is_empty())
        .map(|entry| {
            Err(MutatorError::SubagentEffortsEmpty {
                entry: entry.clone(),
            })
        })
        .unwrap_or(Ok(()))
}

/// Parses one permission rule: `ToolName`, `ToolName(glob)`, or the legacy
/// `ToolName=literal` (`permrule.go:26-48`). Only an empty tool name rejects.
pub fn parse_permission_rule(rule: &str) -> Result<(), MutatorError> {
    let trimmed = rule.trim();
    if trimmed.is_empty() {
        return Err(MutatorError::InvalidPermissionRule {
            rule: rule.to_owned(),
        });
    }
    let eq = trimmed.find('=');
    let paren = trimmed.find('(');
    if let Some(eq) = eq
        && eq > 0
        && match paren {
            None => true,
            Some(p) => eq < p,
        }
    {
        let tool = trimmed[..eq].trim();
        return (!tool.is_empty()).then_some(()).ok_or_else(|| {
            MutatorError::InvalidPermissionRule {
                rule: rule.to_owned(),
            }
        });
    }
    if let Some(open) = paren
        && trimmed.ends_with(')')
    {
        let tool = trimmed[..open].trim();
        return (!tool.is_empty()).then_some(()).ok_or_else(|| {
            MutatorError::InvalidPermissionRule {
                rule: rule.to_owned(),
            }
        });
    }
    Ok(())
}

/// Resolves which permission list a mutator targets (`edit.go:521-532`).
pub fn permission_list<'a>(
    config: &'a mut Config,
    list: &str,
) -> Result<&'a mut Vec<String>, MutatorError> {
    match list.trim().to_lowercase().as_str() {
        "allow" => Ok(&mut config.permissions.allow),
        "ask" => Ok(&mut config.permissions.ask),
        "deny" => Ok(&mut config.permissions.deny),
        other => Err(MutatorError::UnknownPermissionList {
            list: other.to_owned(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Config mutators wired to the modeled schema.
// ---------------------------------------------------------------------------

impl Config {
    /// Writes `agent.default_mode` (`default_mode.go`).
    pub fn set_default_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        self.agent.default_mode = normalize_default_mode(mode)?.to_owned();
        Ok(())
    }

    /// Writes `agent.compact_ratio` (`edit_compaction.go`).
    pub fn set_compact_ratio(&mut self, ratio: f64) -> Result<(), MutatorError> {
        validate_compact_ratio(ratio)?;
        self.agent.compact_ratio = ratio;
        Ok(())
    }

    /// Writes top-level `language` (`edit.go:186-199`). Pricing backfill after
    /// normalization is a platform-side wip.
    pub fn set_language(&mut self, lang: &str) -> Result<(), MutatorError> {
        self.language = normalize_language(lang)?;
        Ok(())
    }

    /// Writes `agent.reasoning_language` (`edit.go:203-215`).
    pub fn set_reasoning_language(&mut self, lang: &str) -> Result<(), MutatorError> {
        self.agent.reasoning_language = normalize_reasoning_language(lang)?;
        Ok(())
    }

    /// Writes `permissions.mode` (`edit.go:473-481`).
    pub fn set_permission_mode(&mut self, mode: &str) -> Result<(), MutatorError> {
        match mode.trim().to_lowercase().as_str() {
            "ask" | "allow" | "deny" => self.permissions.mode = mode.trim().to_lowercase(),
            other => {
                return Err(MutatorError::PermissionModeInvalid {
                    mode: other.to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Appends one rule to a permission list; duplicates are a no-op
    /// (`edit.go:486-500`).
    pub fn add_permission_rule(&mut self, list: &str, rule: &str) -> Result<(), MutatorError> {
        parse_permission_rule(rule)?;
        let rules = permission_list(self, list)?;
        let rule = rule.trim().to_owned();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
        Ok(())
    }

    /// Removes the first exact match from a list; reports whether something
    /// changed (`edit.go:504-517`).
    pub fn remove_permission_rule(&mut self, list: &str, rule: &str) -> Result<bool, MutatorError> {
        let rules = permission_list(self, list)?;
        let Some(pos) = rules.iter().position(|existing| existing == rule) else {
            return Ok(false);
        };
        rules.remove(pos);
        Ok(true)
    }

    /// Records a `[tools.mcp_load]` mode; an empty mode clears the override
    /// (`mcp_load.go:62-83`).
    pub fn set_mcp_load(&mut self, name: &str, mode: &str) -> Result<(), MutatorError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(MutatorError::MCPLoadNameRequired);
        }
        match mode.trim().to_lowercase().as_str() {
            "" => {
                self.tools.mcp_load.remove(name);
            }
            "deferred" => {
                self.tools
                    .mcp_load
                    .insert(name.to_owned(), "deferred".to_owned());
            }
            "always" => {
                self.tools
                    .mcp_load
                    .insert(name.to_owned(), "always".to_owned());
            }
            other => {
                return Err(MutatorError::MCPLoadInvalid {
                    name: other.to_owned(),
                    always: "always".to_owned(),
                    deferred: "deferred".to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Toggles a tool in the disabled list; blank names are a no-op
    /// (`tools_disabled.go:26-39`).
    pub fn set_tool_disabled(&mut self, name: &str, disabled: bool) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let present = self.tools.disabled.iter().any(|existing| existing == &name);
        match (disabled, present) {
            (true, false) => self.tools.disabled.push(name),
            (false, true) => self.tools.disabled.retain(|existing| existing != &name),
            _ => {}
        }
        self.tools.disabled.sort();
    }

    /// Writes `serve.share_port` (`serve_share_port.go:35-41`).
    pub fn set_share_port(&mut self, port: i64) -> Result<(), MutatorError> {
        if port == 0 || (bounds::SHARE_PORT_MIN..=bounds::SHARE_PORT_MAX).contains(&port) {
            self.serve.share_port = port;
            Ok(())
        } else {
            Err(MutatorError::SharePortOutOfRange {
                lo: bounds::SHARE_PORT_MIN,
                hi: bounds::SHARE_PORT_MAX,
            })
        }
    }

    /// Writes `serve.remember_paired_devices` (`serve_share_port.go:45-47`).
    pub fn set_remember_paired_devices(&mut self, on: bool) {
        self.serve.remember_paired_devices = on;
    }

    /// Writes `agent.subagent_claims` (`subagent_claims.go:15-21`).
    pub fn set_subagent_claims(&mut self, optimistic: bool) {
        self.agent.subagent_claims = if optimistic {
            "optimistic".to_owned()
        } else {
            "strict".to_owned()
        };
    }

    /// Writes `agent.write_lease` (`write_lease.go:17`).
    pub fn set_write_lease(&mut self, optimistic: bool) {
        self.agent.write_lease = if optimistic {
            "optimistic".to_owned()
        } else {
            "strict".to_owned()
        };
    }

    /// Writes `agent.plan_approval` (`plan_approval.go:12-18`).
    pub fn set_plan_approval(&mut self, auto: bool) {
        self.agent.plan_approval = if auto {
            "auto".to_owned()
        } else {
            "ask".to_owned()
        };
    }
}

/// Validates a skill name key (`skill_name.go`): Unicode letters/numbers plus
/// `_`, `-`, `.`, 1–64 chars, starting alphanumeric.
pub fn validate_skill_name(name: &str) -> Result<(), MutatorError> {
    let trimmed = name.trim();
    let ok = !trimmed.is_empty()
        && trimmed.chars().count() <= 64
        && trimmed
            .chars()
            .next()
            .is_some_and(|first| first.is_alphanumeric())
        && trimmed
            .chars()
            .all(|r| r.is_alphanumeric() || matches!(r, '_' | '-' | '.'));
    ok.then_some(())
        .ok_or_else(|| MutatorError::SkillNameInvalid {
            name: name.to_owned(),
        })
}

/// Validates the ask-timeout seconds; the non-whole branch mirrors the Go
/// string parsing before the numeric range check (`ask_timeout.go:70-76`).
pub fn validate_ask_timeout(raw: &str) -> Result<i64, MutatorError> {
    let seconds: i64 = raw
        .trim()
        .parse()
        .map_err(|_| MutatorError::AskTimeoutNotWhole {
            raw: raw.to_owned(),
            lo: bounds::ASK_TIMEOUT_MIN,
            hi: bounds::ASK_TIMEOUT_MAX,
        })?;
    if seconds == 0 || (bounds::ASK_TIMEOUT_MIN..=bounds::ASK_TIMEOUT_MAX).contains(&seconds) {
        Ok(seconds)
    } else {
        Err(MutatorError::AskTimeoutOutOfRange {
            lo: bounds::ASK_TIMEOUT_MIN,
            hi: bounds::ASK_TIMEOUT_MAX,
            n: seconds,
        })
    }
}

/// Validates session reference tokens: 0 clears to default, else the clamp
/// range (`session_ref_tokens.go:31-37`).
pub fn validate_session_ref_tokens(tokens: i64) -> Result<(), MutatorError> {
    if tokens == 0
        || (bounds::SESSION_REF_TOKENS_MIN..=bounds::SESSION_REF_TOKENS_MAX).contains(&tokens)
    {
        Ok(())
    } else {
        Err(MutatorError::SessionRefTokensOutOfRange {
            lo: bounds::SESSION_REF_TOKENS_MIN,
            hi: bounds::SESSION_REF_TOKENS_MAX,
            n: tokens,
        })
    }
}

/// Validates the memory proactive mode (`edit_memory.go:10-22`).
pub fn normalize_memory_proactive(mode: &str) -> Result<&'static str, MutatorError> {
    match mode.trim().to_lowercase().as_str() {
        "" | "ask" => Ok("ask"),
        "auto" => Ok("auto"),
        "off" => Ok("off"),
        other => Err(MutatorError::MemoryProactiveInvalid {
            mode: other.to_owned(),
        }),
    }
}

/// Validates the shell `prefer` (`edit_shell.go:13-29`).
pub fn normalize_shell_prefer(prefer: &str) -> Result<&'static str, MutatorError> {
    match prefer.trim().to_lowercase().as_str() {
        "" | "auto" => Ok("auto"),
        "bash" => Ok("bash"),
        "powershell" => Ok("powershell"),
        "pwsh" => Ok("pwsh"),
        other => Err(MutatorError::ShellPreferInvalid {
            prefer: other.to_owned(),
        }),
    }
}

/// Validates auto-archive days: 1..=`AUTO_ARCHIVE_MAX_DAYS`
/// (`auto_archive.go:46-52`).
pub fn validate_auto_archive_days(days: i64) -> Result<(), MutatorError> {
    if (1..=bounds::AUTO_ARCHIVE_MAX_DAYS).contains(&days) {
        Ok(())
    } else {
        Err(MutatorError::AutoArchiveOutOfRange {
            max: bounds::AUTO_ARCHIVE_MAX_DAYS,
        })
    }
}

/// Validates progress-watch rounds and token multiple (`progress_watch.go:49-55`).
pub fn validate_progress_watch(rounds: i64, token_multiple: i64) -> Result<(), MutatorError> {
    if !(1..=bounds::PROGRESS_WATCH_MAX).contains(&rounds) {
        return Err(MutatorError::ProgressWatchRoundsOutOfRange {
            max: bounds::PROGRESS_WATCH_MAX,
        });
    }
    if !(1..=bounds::PROGRESS_WATCH_MAX).contains(&token_multiple) {
        return Err(MutatorError::ProgressWatchMultipleOutOfRange {
            max: bounds::PROGRESS_WATCH_MAX,
        });
    }
    Ok(())
}

/// Validates the persona halves: prefix and suffix each ≤ 32 KiB
/// (`persona.go:66-73`).
pub fn validate_persona(prefix: &str, suffix: &str) -> Result<(), MutatorError> {
    if prefix.len() <= bounds::PERSONA_HALF_LIMIT_BYTES
        && suffix.len() <= bounds::PERSONA_HALF_LIMIT_BYTES
    {
        Ok(())
    } else {
        Err(MutatorError::PersonaTooLarge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_accepts_only_normal_and_plan() {
        assert_eq!(normalize_default_mode(" PLAN ").unwrap(), "plan");
        assert_eq!(normalize_default_mode("normal").unwrap(), "normal");
        let err = normalize_default_mode("think").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"default_mode "think": want normal or plan"#
        );
        assert_eq!(err.ident(), "DefaultModeInvalid");
    }

    #[test]
    fn compact_ratio_rejects_the_closed_window() {
        assert!(validate_compact_ratio(0.70).is_ok());
        let err = validate_compact_ratio(0.0).unwrap_err();
        assert!(err.to_string().contains("must be above 0 and below 1"));
        assert_eq!(err.ident(), "ErrCompactRatioRange");
        let err = validate_compact_ratio(1.0).unwrap_err();
        assert!(err.to_string().contains("must be above 0 and below 1"));
    }

    #[test]
    fn reasoning_language_aliases_resolve_to_auto() {
        assert_eq!(normalize_reasoning_language("follow").unwrap(), "");
        assert_eq!(normalize_reasoning_language("中文").unwrap(), "zh");
        let err = normalize_reasoning_language("fr").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"reasoning language "fr": must be auto|zh|en"#
        );
    }

    #[test]
    fn permission_mode_and_rule_matrix() {
        let mut config = Config::default();
        config.set_permission_mode("Deny").unwrap();
        assert_eq!(config.permissions.mode, "deny");
        let err = config.set_permission_mode("yolo").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"permission mode "yolo": must be ask|allow|deny"#
        );

        config.add_permission_rule("allow", "Read").unwrap();
        config.add_permission_rule("allow", "Read").unwrap();
        assert_eq!(config.permissions.allow, vec!["Read".to_owned()]);
        let err = config.add_permission_rule("deny", "()").unwrap_err();
        assert!(
            err.to_string()
                .starts_with(r#"invalid permission rule "()""#)
        );
        let err = config.add_permission_rule("nope", "Read").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"unknown permission list "nope" (want allow|ask|deny)"#
        );
        assert!(config.remove_permission_rule("allow", "Read").unwrap());
        assert!(!config.remove_permission_rule("allow", "Read").unwrap());
    }

    #[test]
    fn mcp_load_empty_name_and_bad_mode_are_rejected() {
        let mut config = Config::default();
        let err = config.set_mcp_load("  ", "always").unwrap_err();
        assert_eq!(err.to_string(), "mcp load: server name is required");
        let err = config.set_mcp_load("srv", "maybe").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"mcp load "maybe": want "always" or "deferred""#
        );
        config.set_mcp_load("srv", "ALWAYS").unwrap();
        assert_eq!(config.tools.mcp_load["srv"], "always");
        config.set_mcp_load("srv", "").unwrap();
        assert!(config.tools.mcp_load.is_empty());
    }

    #[test]
    fn tool_disabled_toggles_deduplicated_and_sorted() {
        let mut config = Config::default();
        config.set_tool_disabled("WebSearch", true);
        config.set_tool_disabled("Bash", true);
        config.set_tool_disabled("WebSearch", true);
        assert_eq!(config.tools.disabled, vec!["Bash", "WebSearch"]);
        config.set_tool_disabled("Bash", false);
        assert_eq!(config.tools.disabled, vec!["WebSearch"]);
    }

    #[test]
    fn share_port_accepts_zero_and_the_unprivileged_range() {
        let mut config = Config::default();
        config.set_share_port(0).unwrap();
        config.set_share_port(2049).unwrap();
        let err = config.set_share_port(80).unwrap_err();
        assert_eq!(
            err.to_string(),
            "share port out of range: use 0 or a port between 1024 and 65535"
        );
        assert_eq!(err.ident(), "ErrSharePortOutOfRange");
    }

    #[test]
    fn session_ref_tokens_and_ask_timeout_bounds() {
        assert!(validate_session_ref_tokens(0).is_ok());
        assert!(validate_session_ref_tokens(500).is_ok());
        let err = validate_session_ref_tokens(10).unwrap_err();
        assert!(err.to_string().contains("between 500 and 200000, not 10"));
        let err = validate_ask_timeout("abc").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"ask timeout out of range: "abc" is not a whole number of seconds (0 or 10 to 86400)"#
        );
        let err = validate_ask_timeout("5").unwrap_err();
        assert!(
            err.to_string()
                .contains("seconds must be 0 (off) or between 10 and 86400, not 5")
        );
        assert_eq!(validate_ask_timeout("0").unwrap(), 0);
        assert_eq!(validate_ask_timeout("30").unwrap(), 30);
    }

    #[test]
    fn progress_watch_and_persona_bounds() {
        assert!(validate_progress_watch(3, 2).is_ok());
        let err = validate_progress_watch(0, 2).unwrap_err();
        assert!(
            err.to_string()
                .contains("rounds must be between 1 and 1000")
        );
        let err = validate_progress_watch(3, 0).unwrap_err();
        assert!(
            err.to_string()
                .contains("token_multiple must be between 1 and 1000")
        );
        assert!(validate_persona("a", "b").is_ok());
        let big = "x".repeat(bounds::PERSONA_HALF_LIMIT_BYTES + 1);
        let err = validate_persona(&big, "").unwrap_err();
        assert_eq!(
            err.to_string(),
            "persona is too large: prefix and suffix must each be at most 32 KiB"
        );
    }

    #[test]
    fn skill_name_key_matrix() {
        assert!(validate_skill_name("web-search_1").is_ok());
        assert!(validate_skill_name("1digit").is_ok());
        let err = validate_skill_name("-bad").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("skill name \"-bad\": use Unicode")
        );
    }

    #[test]
    fn plan_approval_claims_and_lease_write_the_two_words() {
        let mut config = Config::default();
        config.set_plan_approval(true);
        assert_eq!(config.agent.plan_approval, "auto");
        config.set_subagent_claims(false);
        assert_eq!(config.agent.subagent_claims, "strict");
        config.set_write_lease(true);
        assert_eq!(config.agent.write_lease, "optimistic");
    }
}
