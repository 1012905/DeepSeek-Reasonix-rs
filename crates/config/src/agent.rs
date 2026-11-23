//! The `[agent]` section: prompt, model routing, delegation limits, and the
//! reasoning-language / default-mode normalizations.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Used when config provides none (config.go DefaultSystemPrompt).
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Reasonix, a coding agent.
Use the available tools when they help you complete the user's request.
Keep changes focused and responses concise.";

/// The built-in system prompt verbatim, three lines, no trailing newline.
pub fn default_system_prompt() -> &'static str {
    DEFAULT_SYSTEM_PROMPT
}

/// [agent]: how a turn thinks and how far delegation may go. Limits that are
/// user/global only in the reference are noted on their field; the project
/// merge holds them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct AgentSection {
    pub system_prompt: String,
    pub system_prompt_file: String,
    /// Retired step caps, normalized to 0 on load (the adaptive progress
    /// policy owns the limit now).
    pub max_steps: i64,
    pub planner_max_steps: i64,
    pub temperature: f64,
    pub planner_model: String,
    pub title_model: String,
    pub guardian_model: String,
    pub guardian_temperature: f64,
    pub recovery_model: String,
    pub subagent_model: String,
    pub subagent_models: BTreeMap<String, String>,
    pub vision_model: String,
    pub web_search_model: String,
    pub triage_model: String,
    pub decision_model: String,
    pub advisor_model: String,
    pub screen_external_content: bool,
    pub subagent_effort: String,
    pub subagent_efforts: BTreeMap<String, String>,
    /// 0 removes the stop; the built-in default is 8 (user file only).
    pub max_subagent_depth: i64,
    pub max_side_sessions: i64,
    pub max_queued_inputs: i64,
    /// 0 means until cancelled; the built-in default is 10 (user file only).
    pub max_provider_retries: i64,
    /// 0 leaves the goal budget off.
    pub goal_token_budget: i64,
    pub max_subagent_concurrency: i64,
    pub max_parallel_writers: i64,
    pub subagent_claims: String,
    pub write_lease: String,
    pub plan_approval: String,
    pub default_mode: String,
    pub goal_auto_retry: i64,
    pub write_lease_max_wait_minutes: i64,
    pub output_style: String,
    /// Retired (automatic plan mode); always reads "off" after a load.
    pub auto_plan: String,
    pub reasoning_language: String,
    pub turn_narration: String,
    /// Retired classifier selector, cleared on load.
    pub auto_plan_classifier: String,
    /// Fold at this share of the model window; 0.90 is the built-in default.
    pub compact_ratio: f64,
    /// 0 selects the built-in 0.50.
    pub checkpoint_ceiling_ratio: f64,
    pub embedded_diff_detection: Option<bool>,
    /// Legacy Plan-mode bash allow list; Plan bash reads [permissions] now.
    pub plan_mode_read_only_commands: Vec<String>,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            system_prompt: default_system_prompt().to_owned(),
            system_prompt_file: String::new(),
            max_steps: 0,
            planner_max_steps: 0,
            temperature: 0.0,
            planner_model: String::new(),
            title_model: String::new(),
            guardian_model: String::new(),
            guardian_temperature: 0.0,
            recovery_model: String::new(),
            subagent_model: String::new(),
            subagent_models: BTreeMap::new(),
            vision_model: String::new(),
            web_search_model: String::new(),
            triage_model: String::new(),
            decision_model: String::new(),
            advisor_model: String::new(),
            screen_external_content: false,
            subagent_effort: String::new(),
            subagent_efforts: BTreeMap::new(),
            max_subagent_depth: 8,
            max_side_sessions: 0,
            max_queued_inputs: 0,
            max_provider_retries: 10,
            goal_token_budget: 0,
            max_subagent_concurrency: 0,
            max_parallel_writers: 0,
            subagent_claims: "optimistic".to_owned(),
            write_lease: "optimistic".to_owned(),
            plan_approval: "ask".to_owned(),
            default_mode: "normal".to_owned(),
            goal_auto_retry: 0,
            write_lease_max_wait_minutes: 0,
            output_style: String::new(),
            auto_plan: "off".to_owned(),
            reasoning_language: String::new(),
            turn_narration: String::new(),
            auto_plan_classifier: String::new(),
            compact_ratio: 0.90,
            checkpoint_ceiling_ratio: 0.0,
            embedded_diff_detection: None,
            plan_mode_read_only_commands: Vec::new(),
        }
    }
}

/// Normalizes agent.reasoning_language to `auto`|`zh`|`en`
/// (config.go NormalizeReasoningLanguage). The model-following aliases the
/// reply language does not accept resolve to auto here.
pub fn normalize_reasoning_language(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "" | "auto" | "follow" | "conversation" | "detect" | "default" | "model"
        | "model-default" | "model_default" | "provider" => "auto",
        "zh" | "cn" | "chinese" | "中文" => "zh",
        "en" | "english" => "en",
        _ => "auto",
    }
}

/// The default session mode: Plan only when the value is exactly `plan`
/// (default_mode.go DefaultPlanMode).
pub fn default_plan_mode(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case("plan")
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SYSTEM_PROMPT, default_plan_mode, default_system_prompt,
        normalize_reasoning_language,
    };

    #[test]
    fn the_default_prompt_is_the_three_reference_lines() {
        // config.md §2: config.go:1307-1309 verbatim, no trailing newline.
        assert_eq!(default_system_prompt(), DEFAULT_SYSTEM_PROMPT);
        assert_eq!(
            DEFAULT_SYSTEM_PROMPT,
            "You are Reasonix, a coding agent.\n\
             Use the available tools when they help you complete the user's request.\n\
             Keep changes focused and responses concise."
        );
        assert!(!DEFAULT_SYSTEM_PROMPT.ends_with('\n'));
    }

    #[test]
    fn reasoning_language_accepts_the_model_following_aliases() {
        // config.md §3: the reasoning matrix is the reply matrix plus the
        // model-following spellings.
        for raw in [
            "",
            "auto",
            "follow",
            "conversation",
            "detect",
            "default",
            "model",
            "model-default",
            "model_default",
            "provider",
            "  MODEL  ",
        ] {
            assert_eq!(normalize_reasoning_language(raw), "auto", "raw = {raw:?}");
        }
        for raw in ["zh", "CN", "chinese", "中文"] {
            assert_eq!(normalize_reasoning_language(raw), "zh", "raw = {raw:?}");
        }
        for raw in ["en", "ENGLISH"] {
            assert_eq!(normalize_reasoning_language(raw), "en", "raw = {raw:?}");
        }
        assert_eq!(normalize_reasoning_language("fr"), "auto");
    }

    #[test]
    fn default_mode_is_plan_only_for_the_plan_spelling() {
        assert!(default_plan_mode("plan"));
        assert!(default_plan_mode(" PLAN "));
        assert!(!default_plan_mode("normal"));
        assert!(!default_plan_mode(""));
        assert!(!default_plan_mode("planner"));
    }
}
