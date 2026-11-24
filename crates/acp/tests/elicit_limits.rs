//! ACP-021: elicitation limits, schema builder, reply validation, and mode gating.

use std::collections::BTreeMap;

use reasonix_acp::elicitation::{
    AdvertisedModes, AskOrigin, AskQuestion, ELICIT_MESSAGE_LIMIT, ELICIT_TITLE_LIMIT,
    ELICIT_VALUE_LIMIT, build_schema, clip_plain, mode_eligible, url_answer, validate_form_answers,
};
use serde_json::json;

#[test]
fn clip_plain_respects_rune_limits_and_drops_control() {
    assert_eq!(clip_plain("plain", 1024), "plain");
    let long = "a".repeat(ELICIT_MESSAGE_LIMIT + 50);
    let clipped = clip_plain(&long, ELICIT_MESSAGE_LIMIT);
    assert!(clipped.ends_with("…"));
    assert_eq!(clipped.chars().count(), ELICIT_MESSAGE_LIMIT + 1);
    assert_eq!(clip_plain("a\nb", 10), "a\nb");
}

#[test]
fn schema_multi_with_options_emits_array_enum() {
    let questions = [AskQuestion {
        id: "q1".to_owned(),
        header: "Pick tools".to_owned(),
        prompt: "Which tools?".to_owned(),
        options: vec!["bash".to_owned(), "edit".to_owned()],
        multi: true,
        default: vec!["bash".to_owned()],
    }];
    let schema = build_schema(&questions);
    let field = &schema["properties"]["q1"];
    assert_eq!(field["type"], "array");
    assert_eq!(field["items"]["type"], "string");
    assert_eq!(field["items"]["enum"], json!(["bash", "edit"]));
    assert_eq!(field["default"], json!(["bash"]));
    assert_eq!(field["title"], "Pick tools");
    assert_eq!(field["description"], "Which tools?");
}

#[test]
fn schema_single_with_options_emits_string_enum() {
    let questions = [AskQuestion {
        id: "q2".to_owned(),
        header: "Mode".to_owned(),
        prompt: String::new(),
        options: vec!["plan".to_owned(), "goal".to_owned()],
        multi: false,
        default: vec!["plan".to_owned()],
    }];
    let schema = build_schema(&questions);
    let field = &schema["properties"]["q2"];
    assert_eq!(field["type"], "string");
    assert_eq!(field["enum"], json!(["plan", "goal"]));
    assert_eq!(field["default"], "plan");
    assert!(field.get("description").is_none());
}

#[test]
fn schema_plain_text_emits_max_length_default() {
    let questions = [AskQuestion {
        id: "q3".to_owned(),
        header: "Notes".to_owned(),
        prompt: "Free text".to_owned(),
        options: Vec::new(),
        multi: false,
        default: Vec::new(),
    }];
    let schema = build_schema(&questions);
    let field = &schema["properties"]["q3"];
    assert_eq!(field["type"], "string");
    assert_eq!(field["maxLength"], ELICIT_VALUE_LIMIT);
    assert!(field.get("default").is_none());
}

#[test]
fn title_is_clamped_to_title_limit() {
    let questions = [AskQuestion {
        id: "q4".to_owned(),
        header: "z".repeat(ELICIT_TITLE_LIMIT + 30),
        prompt: String::new(),
        options: Vec::new(),
        multi: false,
        default: Vec::new(),
    }];
    let schema = build_schema(&questions);
    let title = schema["properties"]["q4"]["title"].as_str().unwrap();
    assert!(title.ends_with("…"));
    assert_eq!(title.chars().count(), ELICIT_TITLE_LIMIT + 1);
}

#[test]
fn validation_accepts_known_valid_answers() {
    let questions = [
        AskQuestion {
            id: "q1".to_owned(),
            header: "Multi".to_owned(),
            prompt: String::new(),
            options: vec!["a".to_owned(), "b".to_owned()],
            multi: true,
            default: Vec::new(),
        },
        AskQuestion {
            id: "q2".to_owned(),
            header: "Single".to_owned(),
            prompt: String::new(),
            options: vec!["x".to_owned()],
            multi: false,
            default: Vec::new(),
        },
    ];
    let mut content = BTreeMap::new();
    content.insert("q1".to_owned(), json!(["a", "b"]));
    content.insert("q2".to_owned(), json!("x"));
    let answers = validate_form_answers(&questions, &content).unwrap();
    assert_eq!(answers[0].selected, vec!["a", "b"]);
    assert_eq!(answers[1].selected, vec!["x"]);
}

#[test]
fn validation_rejects_whole_form_on_any_violation() {
    let questions = [AskQuestion {
        id: "q1".to_owned(),
        header: "Q".to_owned(),
        prompt: String::new(),
        options: vec!["a".to_owned()],
        multi: false,
        default: Vec::new(),
    }];

    let mut unknown = BTreeMap::new();
    unknown.insert("nope".to_owned(), json!("a"));
    assert!(validate_form_answers(&questions, &unknown).is_none());

    let mut bad_type = BTreeMap::new();
    bad_type.insert("q1".to_owned(), json!(42));
    assert!(validate_form_answers(&questions, &bad_type).is_none());

    let mut enum_violation = BTreeMap::new();
    enum_violation.insert("q1".to_owned(), json!("z"));
    assert!(validate_form_answers(&questions, &enum_violation).is_none());

    let mut multi_hit = BTreeMap::new();
    multi_hit.insert("q1".to_owned(), json!(["a", "b"]));
    assert!(validate_form_answers(&questions, &multi_hit).is_none());

    let mut too_long = BTreeMap::new();
    too_long.insert("q1".to_owned(), json!("y".repeat(ELICIT_VALUE_LIMIT + 1)));
    assert!(validate_form_answers(&questions, &too_long).is_none());
}

#[test]
fn url_answer_accepts_accept_and_cancel_only() {
    assert_eq!(url_answer("q1", "accept").unwrap().selected, vec!["accept"]);
    assert_eq!(url_answer("q1", "cancel").unwrap().selected, vec!["cancel"]);
    assert!(url_answer("q1", "deny").is_none());
    assert!(url_answer("q1", "").is_none());
}

#[test]
fn mode_gating_requires_advertised_origin_and_question_count() {
    let modes = AdvertisedModes {
        form: true,
        url: true,
    };
    assert!(mode_eligible(AskOrigin::Url, modes, 1));
    assert!(!mode_eligible(AskOrigin::Url, modes, 2));
    assert!(mode_eligible(AskOrigin::Form, modes, 1));
    assert!(!mode_eligible(AskOrigin::Form, modes, 0));
    assert!(!mode_eligible(AskOrigin::None, modes, 1));

    let no_url = AdvertisedModes {
        form: true,
        url: false,
    };
    assert!(!mode_eligible(AskOrigin::Url, no_url, 1));
}
