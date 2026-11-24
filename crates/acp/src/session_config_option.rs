//! ACP-033 `session/set_config_option` alias normalization, category
//! derivation, ceiling precheck, value membership, and five-axis dispatch
//! (`session.go:497-545`; `session_config.go:549-621`).
//!
//! The request id and every advertised option id normalize through the same
//! four-group alias table before matching. The ceiling precheck applies only to
//! a recognized tool-approval posture value; the membership check compares the
//! request value for equality against advertised select options. Real config
//! state resolution, runtime profile/model/effort/approval backends, and the
//! busy-state refusals are runtime wiring; this module owns the lookup, the
//! category table, the precheck, and the dispatch mapping.

use crate::errors::{RpcError, RpcErrorCode};
use crate::lifecycle::{invalid_config_value, normalize_config_id, unsupported_config_option};
use crate::permission::POSTURE_ORDER;
use crate::wire::SessionConfigOption;

/// Agent composition axis (`session_config.go:525-529`).
pub const CATEGORY_AGENT_COMPOSITION: &str = "agent_composition";
/// Model axis (`session_config.go:530-531`).
pub const CATEGORY_MODEL: &str = "model";
/// Reasoning effort axis (`session_config.go:532-533`).
pub const CATEGORY_THOUGHT_LEVEL: &str = "thought_level";
/// Runtime work profile axis; `agent_preset` aliases it (`session_config.go:534-535`).
pub const CATEGORY_WORK_MODE: &str = "work_mode";
pub const CATEGORY_AGENT_PRESET: &str = "agent_preset";
/// Tool approval axis (`session_config.go:536-537`).
pub const CATEGORY_TOOL_APPROVAL: &str = "tool_approval";

/// Finds an advertised option by normalized id on both sides
/// (`session_config.go:549-557`).
pub fn find_option<'a>(
    options: &'a [SessionConfigOption],
    raw_id: &str,
) -> Option<&'a SessionConfigOption> {
    let wanted = normalize_config_id(raw_id);
    options
        .iter()
        .find(|option| normalize_config_id(&option.id) == wanted)
}

/// Derives the option category: an explicit `category` wins; otherwise the
/// normalized id maps onto the thought-level axis for effort
/// (`session_config.go:583-599`).
pub fn option_category(option: &SessionConfigOption) -> String {
    if let Some(category) = option.category.as_ref().filter(|c| !c.trim().is_empty()) {
        return category.clone();
    }
    match normalize_config_id(&option.id).as_str() {
        "model" => CATEGORY_MODEL.to_owned(),
        "effort" => CATEGORY_THOUGHT_LEVEL.to_owned(),
        "work_mode" => CATEGORY_WORK_MODE.to_owned(),
        "tool_approval" => CATEGORY_TOOL_APPROVAL.to_owned(),
        _ => String::new(),
    }
}

/// Exact membership check against advertised select options
/// (`session_config.go:574-581`).
pub fn option_has_value(option: &SessionConfigOption, value: &str) -> bool {
    option.options.iter().any(|opt| opt.value == value)
}

/// Config axis the option dispatches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAxis {
    AgentComposition,
    Model,
    ThoughtLevel,
    WorkMode,
    ToolApproval,
    /// Category the session has no backend axis for.
    Unsupported,
}

pub fn axis_for(option: &SessionConfigOption) -> ConfigAxis {
    match option_category(option).as_str() {
        CATEGORY_AGENT_COMPOSITION => ConfigAxis::AgentComposition,
        CATEGORY_MODEL => ConfigAxis::Model,
        CATEGORY_THOUGHT_LEVEL => ConfigAxis::ThoughtLevel,
        CATEGORY_WORK_MODE | CATEGORY_AGENT_PRESET => ConfigAxis::WorkMode,
        CATEGORY_TOOL_APPROVAL => ConfigAxis::ToolApproval,
        _ => ConfigAxis::Unsupported,
    }
}

/// Backend seam each axis switches through.
pub trait ConfigBackend {
    fn select_composition(&mut self, value: &str) -> Result<(), String>;
    fn switch_model(&mut self, value: &str) -> Result<(), String>;
    fn switch_effort(&mut self, value: &str) -> Result<(), String>;
    fn switch_profile(&mut self, value: &str) -> Result<(), String>;
    fn switch_tool_approval(&mut self, value: &str) -> Result<(), String>;
}

/// Unknown session refusal (`session.go:503-505`).
pub fn unknown_session_error(id: &str) -> RpcError {
    crate::extensions::unknown_session("session/set_config_option", id)
}

/// Config-state fetch failure (`session.go:506-509`).
pub fn config_state_error(message: &str) -> RpcError {
    RpcError::new(
        RpcErrorCode::Internal,
        format!("session/set_config_option: {message}"),
    )
}

/// Runs the prechecks and dispatches one option switch. Returns the axis that
/// was applied so the handler can re-read the refreshed config state.
pub fn apply_option(
    backend: &mut dyn ConfigBackend,
    option: &SessionConfigOption,
    value: &str,
    ceiling: &str,
) -> Result<ConfigAxis, RpcError> {
    let category = option_category(option);
    if category == CATEGORY_TOOL_APPROVAL
        && POSTURE_ORDER.contains(&value)
        && !crate::permission::within_ceiling(value, ceiling)
    {
        return Err(crate::permission::above_ceiling_error(
            "session/set_config_option",
            value,
            ceiling,
        ));
    }

    if !option_has_value(option, value) {
        return Err(invalid_config_value(value, &option.id));
    }

    let axis = axis_for(option);
    let result = match axis {
        ConfigAxis::AgentComposition => backend.select_composition(value),
        ConfigAxis::Model => backend.switch_model(value),
        ConfigAxis::ThoughtLevel => backend.switch_effort(value),
        ConfigAxis::WorkMode => backend.switch_profile(value),
        ConfigAxis::ToolApproval => backend.switch_tool_approval(value),
        ConfigAxis::Unsupported => {
            return Err(unsupported_config_option(&option.id));
        }
    };

    result.map_err(|message| {
        RpcError::new(
            RpcErrorCode::Internal,
            format!("session/set_config_option: {message}"),
        )
    })?;
    Ok(axis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;
    use crate::wire::SessionConfigSelectOption;

    fn option(id: &str, category: Option<&str>, values: &[&str]) -> SessionConfigOption {
        SessionConfigOption {
            id: id.to_owned(),
            name: id.to_owned(),
            description: None,
            category: category.map(str::to_owned),
            r#type: "select".to_owned(),
            current_value: String::new(),
            options: values
                .iter()
                .map(|v| SessionConfigSelectOption {
                    value: v.to_string(),
                    name: v.to_string(),
                    description: None,
                })
                .collect(),
        }
    }

    #[test]
    fn lookup_normalizes_both_sides_and_matches_aliases() {
        let options = vec![option("models", None, &["gpt"])];
        assert!(find_option(&options, " models ").is_some());
        assert!(find_option(&options, "model").is_some());
        assert!(find_option(&options, "effort").is_none());
    }

    #[test]
    fn category_explicit_wins_else_id_maps() {
        let effort = option("effort", None, &["low"]);
        assert_eq!(option_category(&effort), "thought_level");
        let explicit = option("x", Some("agent_composition"), &["a"]);
        assert_eq!(option_category(&explicit), "agent_composition");
        let bare = option("token_mode", None, &["focused"]);
        assert_eq!(option_category(&bare), "work_mode");
    }

    #[test]
    fn ceiling_precheck_applies_only_to_known_approval_values() {
        let approval = option("approval_mode", None, &["ask", "yolo"]);
        let err = apply_option(&mut Recording::default(), &approval, "yolo", "ask").unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert!(err.message.contains("above the ceiling"));
    }

    #[test]
    fn membership_check_uses_exact_option_id_in_message() {
        let model = option("models", None, &["gpt-4"]);
        let err = apply_option(&mut Recording::default(), &model, "oops", "").unwrap_err();
        assert_eq!(
            err.message,
            "session/set_config_option: invalid value oops for models"
        );
    }

    #[test]
    fn unsupported_category_refuses_with_option_id() {
        let odd = option("custom", Some("custom_axis"), &["x"]);
        let err = apply_option(&mut Recording::default(), &odd, "x", "").unwrap_err();
        assert_eq!(
            err.message,
            "session/set_config_option: unsupported config option custom"
        );
    }

    #[test]
    fn five_axes_dispatch_to_backend_methods() {
        let mut backend = Recording::default();
        let cases = [
            (
                option("agent", Some("agent_composition"), &["main"]),
                ConfigAxis::AgentComposition,
                "select_composition:main",
            ),
            (
                option("models", None, &["gpt"]),
                ConfigAxis::Model,
                "switch_model:gpt",
            ),
            (
                option("thought_level", None, &["low"]),
                ConfigAxis::ThoughtLevel,
                "switch_effort:low",
            ),
            (
                option("profile", None, &["focused"]),
                ConfigAxis::WorkMode,
                "switch_profile:focused",
            ),
            (
                option("approval", None, &["ask"]),
                ConfigAxis::ToolApproval,
                "switch_tool_approval:ask",
            ),
        ];
        for (opt, axis, event) in cases {
            let applied = apply_option(&mut backend, &opt, &opt.options[0].value, "").unwrap();
            assert_eq!(applied, axis);
            assert!(backend.events.contains(&event.to_owned()));
        }
    }

    #[derive(Default)]
    struct Recording {
        events: Vec<String>,
    }

    impl ConfigBackend for Recording {
        fn select_composition(&mut self, value: &str) -> Result<(), String> {
            self.events.push(format!("select_composition:{value}"));
            Ok(())
        }
        fn switch_model(&mut self, value: &str) -> Result<(), String> {
            self.events.push(format!("switch_model:{value}"));
            Ok(())
        }
        fn switch_effort(&mut self, value: &str) -> Result<(), String> {
            self.events.push(format!("switch_effort:{value}"));
            Ok(())
        }
        fn switch_profile(&mut self, value: &str) -> Result<(), String> {
            self.events.push(format!("switch_profile:{value}"));
            Ok(())
        }
        fn switch_tool_approval(&mut self, value: &str) -> Result<(), String> {
            self.events.push(format!("switch_tool_approval:{value}"));
            Ok(())
        }
    }
}
