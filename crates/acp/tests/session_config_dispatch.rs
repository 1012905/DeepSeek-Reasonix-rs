//! ACP-033: `session/set_config_option` alias normalization, category
//! derivation, ceiling precheck, membership, and five-axis dispatch
//! (`session.go:497-545`; `session_config.go:549-621`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_config_option::{
    ConfigAxis, ConfigBackend, apply_option, axis_for, find_option, option_category,
    option_has_value, unknown_session_error,
};
use reasonix_acp::wire::{SessionConfigOption, SessionConfigSelectOption};

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

#[test]
fn lookup_normalizes_request_and_option_ids_with_trim() {
    let options = vec![option("models", None, &["gpt"])];
    assert!(find_option(&options, " models ").is_some());
    assert!(find_option(&options, "model").is_some());
    assert!(find_option(&options, "effort").is_none());
}

#[test]
fn category_explicit_wins_otherwise_id_maps_to_axis() {
    assert_eq!(
        option_category(&option("effort", None, &["low"])),
        "thought_level"
    );
    assert_eq!(
        option_category(&option("x", Some("agent_composition"), &["a"])),
        "agent_composition"
    );
    assert_eq!(
        option_category(&option("token_mode", None, &["focused"])),
        "work_mode"
    );
}

#[test]
fn membership_check_is_exact_and_names_advertised_option_id() {
    let model = option("models", None, &["gpt-4"]);
    assert!(option_has_value(&model, "gpt-4"));
    assert!(!option_has_value(&model, "oops"));

    let mut backend = Recording::default();
    let err = apply_option(&mut backend, &model, "oops", "").unwrap_err();
    assert_eq!(
        err.message,
        "session/set_config_option: invalid value oops for models"
    );
}

#[test]
fn ceiling_precheck_only_covers_known_approval_postures() {
    let approval = option("approval_mode", None, &["ask", "yolo"]);
    let err = apply_option(&mut Recording::default(), &approval, "yolo", "ask").unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert!(err.message.contains("above the ceiling"));
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

#[test]
fn axis_for_maps_work_mode_and_agent_preset_together() {
    assert_eq!(
        axis_for(&option("p", Some("agent_preset"), &["a"])),
        ConfigAxis::WorkMode
    );
    assert_eq!(
        axis_for(&option("nope", None, &["a"])),
        ConfigAxis::Unsupported
    );
}

#[test]
fn handler_error_builders_are_verbatim() {
    let err = unknown_session_error("s9");
    assert_eq!(err.message, "session/set_config_option: unknown session s9");
}
