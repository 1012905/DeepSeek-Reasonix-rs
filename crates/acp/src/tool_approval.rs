//! ACP-035: the `tool_approval` config option — its five fixed choices, the
//! posture normalization, and the posture-ceiling filtering
//! (`session_config.go:489-547`; `tool_approval_ceiling.go:35-97`).
//!
//! The rank/ceiling helpers live in [`crate::permission`]; this module owns the
//! advertised option shape: the choice table with verbatim names and
//! descriptions, the case-insensitive mode normalization, and the builder that
//! drops choices the startup ceiling cannot admit. Real controller posture
//! application is runtime wiring; the option itself is pure.

use crate::permission::{POSTURE_ORDER, filter_posture_choices};
use crate::wire::{SessionConfigOption, SessionConfigSelectOption};

/// Option id advertised on the config surface (`session_config.go:519`).
pub const OPTION_ID: &str = "tool_approval";
/// Display name (`session_config.go:520`).
pub const OPTION_NAME: &str = "Tool Approval";
/// Category axis (`session_config.go:521`).
pub const OPTION_CATEGORY: &str = "tool_approval";

/// Five posture values in increasing permissiveness
/// (`tool_approval_ceiling.go:35-38`; `controller.go:251-254`;
/// `approval_orchestration.go:661`).
pub const POSTURE_READ_ONLY: &str = "readOnly";
pub const POSTURE_DONT_ASK: &str = "dontAsk";
pub const POSTURE_ASK: &str = "ask";
pub const POSTURE_AUTO: &str = "auto";
pub const POSTURE_YOLO: &str = "yolo";

/// One fixed choice in the `tool_approval` select. Names and descriptions are
/// the reference's verbatim strings (`session_config.go:526-530`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostureChoice {
    pub value: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// The five choices in advertised order.
pub const CHOICES: [PostureChoice; 5] = [
    PostureChoice {
        value: POSTURE_READ_ONLY,
        name: "Read only",
        description: "Refuse every call that is not a read",
    },
    PostureChoice {
        value: POSTURE_DONT_ASK,
        name: "Don't ask",
        description: "Refuse permission-gated calls instead of asking",
    },
    PostureChoice {
        value: POSTURE_ASK,
        name: "Ask",
        description: "Ask before permission-gated tool calls",
    },
    PostureChoice {
        value: POSTURE_AUTO,
        name: "Auto",
        description: "Follow configured permission rules without fallback prompts",
    },
    PostureChoice {
        value: POSTURE_YOLO,
        name: "Yolo",
        description: "Approve tool calls except protected decisions",
    },
];

/// Normalizes a client/posture string to the canonical wire value
/// (`session_config.go:489-502`): case- and space-insensitive, unknown values
/// default to `ask`.
pub fn normalize_mode(mode: &str) -> String {
    match mode.trim().to_ascii_lowercase().as_str() {
        "auto" => POSTURE_AUTO.to_owned(),
        "yolo" => POSTURE_YOLO.to_owned(),
        "readonly" => POSTURE_READ_ONLY.to_owned(),
        "dontask" => POSTURE_DONT_ASK.to_owned(),
        _ => POSTURE_ASK.to_owned(),
    }
}

/// Where a new session opens its posture: the fixed ceiling when one is set,
/// otherwise `ask` (`tool_approval_ceiling.go:51-57`).
pub fn starting_posture(ceiling: &str) -> String {
    if ceiling.trim().is_empty() {
        return POSTURE_ASK.to_owned();
    }
    normalize_mode(ceiling)
}

/// Builds the advertised `tool_approval` option for a session. `current` is
/// the session's current posture; `ceiling` is the factory's startup posture
/// (`""` = uncapped). Choices below the ceiling are filtered out; with no
/// ceiling the two most restrictive choices are hidden
/// (`session_config.go:525-538`).
pub fn build_option(current: &str, ceiling: &str) -> SessionConfigOption {
    let current = normalize_mode(current);
    let ceiling = if ceiling.trim().is_empty() {
        String::new()
    } else {
        normalize_mode(ceiling)
    };
    let all: Vec<String> = POSTURE_ORDER.iter().map(|s| s.to_string()).collect();
    let allowed = filter_posture_choices(&all, &ceiling);
    let options = CHOICES
        .iter()
        .filter(|choice| allowed.iter().any(|v| v.as_str() == choice.value))
        .map(|choice| SessionConfigSelectOption {
            value: choice.value.to_owned(),
            name: choice.name.to_owned(),
            description: Some(choice.description.to_owned()),
        })
        .collect();

    SessionConfigOption {
        id: OPTION_ID.to_owned(),
        name: OPTION_NAME.to_owned(),
        description: None,
        category: Some(OPTION_CATEGORY.to_owned()),
        r#type: "select".to_owned(),
        current_value: current,
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_keep_the_five_frozen_values_and_labels_in_order() {
        let values: Vec<&str> = CHOICES.iter().map(|c| c.value).collect();
        assert_eq!(
            values,
            [
                POSTURE_READ_ONLY,
                POSTURE_DONT_ASK,
                POSTURE_ASK,
                POSTURE_AUTO,
                POSTURE_YOLO
            ]
        );
        let names: Vec<&str> = CHOICES.iter().map(|c| c.name).collect();
        assert_eq!(names, ["Read only", "Don't ask", "Ask", "Auto", "Yolo"]);
        let descriptions: Vec<&str> = CHOICES.iter().map(|c| c.description).collect();
        assert_eq!(
            descriptions,
            [
                "Refuse every call that is not a read",
                "Refuse permission-gated calls instead of asking",
                "Ask before permission-gated tool calls",
                "Follow configured permission rules without fallback prompts",
                "Approve tool calls except protected decisions",
            ]
        );
    }

    #[test]
    fn normalization_is_case_insensitive_and_defaults_to_ask() {
        assert_eq!(normalize_mode("AUTO"), POSTURE_AUTO);
        assert_eq!(normalize_mode(" yolo "), POSTURE_YOLO);
        assert_eq!(normalize_mode("readonly"), POSTURE_READ_ONLY);
        assert_eq!(normalize_mode("dontask"), POSTURE_DONT_ASK);
        assert_eq!(normalize_mode(""), POSTURE_ASK);
        assert_eq!(normalize_mode("banana"), POSTURE_ASK);
    }

    #[test]
    fn uncapped_option_offers_ask_auto_yolo_and_starts_at_ask() {
        let option = build_option(POSTURE_ASK, "");
        assert_eq!(option.id, OPTION_ID);
        assert_eq!(option.name, OPTION_NAME);
        assert_eq!(option.category.as_deref(), Some(OPTION_CATEGORY));
        assert_eq!(option.r#type, "select");
        assert_eq!(option.current_value, POSTURE_ASK);
        let values: Vec<String> = option.options.iter().map(|o| o.value.clone()).collect();
        assert_eq!(values, [POSTURE_ASK, POSTURE_AUTO, POSTURE_YOLO]);
        assert_eq!(starting_posture(""), POSTURE_ASK);
    }

    #[test]
    fn ceiling_hides_choices_above_it_while_the_current_value_is_preserved() {
        let option = build_option(POSTURE_YOLO, POSTURE_AUTO);
        assert_eq!(option.current_value, POSTURE_YOLO);
        let values: Vec<String> = option.options.iter().map(|o| o.value.clone()).collect();
        assert_eq!(
            values,
            [
                POSTURE_READ_ONLY,
                POSTURE_DONT_ASK,
                POSTURE_ASK,
                POSTURE_AUTO
            ]
        );
        assert!(!values.contains(&POSTURE_YOLO.to_owned()));
        assert_eq!(starting_posture(POSTURE_AUTO), POSTURE_AUTO);
    }

    #[test]
    fn read_only_ceiling_keeps_only_the_read_only_choice() {
        let option = build_option(POSTURE_READ_ONLY, POSTURE_READ_ONLY);
        let values: Vec<String> = option.options.iter().map(|o| o.value.clone()).collect();
        assert_eq!(values, [POSTURE_READ_ONLY]);
    }
}
