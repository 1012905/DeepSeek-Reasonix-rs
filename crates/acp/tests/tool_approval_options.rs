//! ACP-035: the `tool_approval` select option — five frozen choices in source
//! order, case-insensitive normalization, and the two ceiling branches.

use reasonix_acp::tool_approval::{
    CHOICES, POSTURE_ASK, POSTURE_AUTO, POSTURE_DONT_ASK, POSTURE_READ_ONLY, POSTURE_YOLO,
    build_option, normalize_mode, starting_posture,
};

#[test]
fn five_choices_keep_verbatim_values_names_and_descriptions_in_order() {
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
    assert_eq!(
        CHOICES.iter().map(|c| c.name).collect::<Vec<_>>(),
        ["Read only", "Don't ask", "Ask", "Auto", "Yolo"]
    );
    assert_eq!(
        CHOICES.iter().map(|c| c.description).collect::<Vec<_>>(),
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
fn normalization_folds_case_and_space_and_defaults_to_ask() {
    assert_eq!(normalize_mode("AUTO"), POSTURE_AUTO);
    assert_eq!(normalize_mode(" yolo "), POSTURE_YOLO);
    assert_eq!(normalize_mode("readonly"), POSTURE_READ_ONLY);
    assert_eq!(normalize_mode("dontask"), POSTURE_DONT_ASK);
    assert_eq!(normalize_mode(""), POSTURE_ASK);
    assert_eq!(normalize_mode("banana"), POSTURE_ASK);
}

#[test]
fn uncapped_option_lists_only_ask_auto_yolo_and_starts_at_ask() {
    let option = build_option(POSTURE_ASK, "");
    assert_eq!(option.id, "tool_approval");
    assert_eq!(option.name, "Tool Approval");
    assert_eq!(option.category.as_deref(), Some("tool_approval"));
    assert_eq!(option.r#type, "select");
    assert_eq!(option.current_value, POSTURE_ASK);
    let values: Vec<String> = option.options.iter().map(|o| o.value.clone()).collect();
    assert_eq!(values, [POSTURE_ASK, POSTURE_AUTO, POSTURE_YOLO]);
    assert_eq!(starting_posture(""), POSTURE_ASK);
}

#[test]
fn ceiling_keeps_at_or_below_choices_and_preserves_the_current_value() {
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

    let read_only = build_option(POSTURE_READ_ONLY, POSTURE_READ_ONLY);
    let values: Vec<String> = read_only.options.iter().map(|o| o.value.clone()).collect();
    assert_eq!(values, [POSTURE_READ_ONLY]);
}
