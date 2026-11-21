//! The `ask` tool at the tools boundary (Go `internal/runtime/agent/ask.go`,
//! docs/contracts/tools.md §3.0's turn-exit family): its provider-visible
//! contract, its registration beside the other built-ins, and its routing to
//! the Asker seam bound on the call context.

use std::sync::Arc;

use reasonix_contract::eventwire::ask::AskQuestion;
use reasonix_tools::abi::{CallContext, Tool, is_decision_barrier};
use reasonix_tools::ask::{AskAnswer, AskOutcome, Asker};
use reasonix_tools::builtin::{ASK_TOOL_NAME, AskTool, ask_tool, turn_exit_tools};
use reasonix_tools::registry::{REACH_HOST_CONTROL, Registry, reach_of};
use serde_json::{Value, json};

fn args() -> Value {
    json!({
        "questions": [{
            "header": "Library",
            "question": "which library",
            "options": [{"label": "A"}, {"label": "B", "description": "the second"}],
            "multiSelect": true
        }]
    })
}

/// A recording asker: proves the tool routes the parsed questions out, and
/// lets the test decide what comes back.
struct RecordingAsker {
    seen: std::sync::Mutex<Vec<Vec<AskQuestion>>>,
    outcome: AskOutcome,
}

impl Asker for RecordingAsker {
    fn ask(&self, questions: &[AskQuestion]) -> AskOutcome {
        self.seen
            .lock()
            .expect("asker lock")
            .push(questions.to_vec());
        self.outcome.clone()
    }
}

fn ctx_with(asker: Arc<Arc<dyn Asker>>) -> CallContext {
    CallContext::new().with_value(asker)
}

#[test]
fn the_schema_and_flags_match_the_go_contract() {
    let t = AskTool;
    assert_eq!(t.name(), ASK_TOOL_NAME);
    assert_eq!(t.name(), "ask");
    assert!(
        t.description()
            .contains("Ask the user for something only they can supply"),
        "description: {}",
        t.description()
    );
    assert!(
        t.description()
            .contains("No tool-approval mode, YOLO included"),
        "the description keeps the posture rule"
    );
    let s = t.schema();
    assert_eq!(s["type"], "object");
    assert_eq!(s["required"], json!(["questions"]));
    assert_eq!(s["properties"]["questions"]["minItems"], 1);
    assert_eq!(s["properties"]["questions"]["maxItems"], 4);
    let item = &s["properties"]["questions"]["items"];
    assert_eq!(
        item["required"],
        json!(["question", "header", "options"]),
        "the three fields a question cannot omit"
    );
    assert_eq!(item["properties"]["options"]["minItems"], 2);
    assert_eq!(item["properties"]["options"]["maxItems"], 4);
    assert_eq!(
        item["properties"]["options"]["items"]["required"],
        json!(["label"])
    );
    assert_eq!(
        item["properties"]["reason"]["enum"],
        json!(["user_decision", "missing_value"])
    );
    assert_eq!(item["properties"]["multiSelect"]["type"], "boolean");

    assert!(t.read_only(), "asking has no host side effects");
    assert!(is_decision_barrier(&t), "the answer ends the round");
    assert_eq!(reach_of(&t), REACH_HOST_CONTROL);
}

#[test]
fn both_registration_entries_carry_the_one_ask_instance() {
    let exit = turn_exit_tools();
    assert_eq!(exit.len(), 1);
    assert_eq!(exit[0].name(), "ask");
    let direct = ask_tool();
    assert_eq!(direct.name(), exit[0].name());
    assert_eq!(direct.description(), exit[0].description());
    assert_eq!(direct.schema(), exit[0].schema());
}

#[test]
fn a_registered_ask_is_provider_visible_and_resolvable() {
    let registry = Registry::new();
    registry.add(ask_tool());
    let names = registry.names();
    assert_eq!(names, vec!["ask".to_owned()]);
    let contract = registry.contract_entries();
    assert_eq!(contract.len(), 1);
    assert_eq!(contract[0].name, "ask");
    assert!(contract[0].read_only);
    assert_eq!(
        contract[0].schema["properties"]["questions"]["type"],
        "array"
    );
    assert!(matches!(
        registry.resolve_call("ask"),
        reasonix_tools::registry::ResolveCall::Exact(_)
    ));
}

#[test]
fn the_tool_routes_the_parsed_questions_to_the_asker() {
    let asker = Arc::new(RecordingAsker {
        seen: std::sync::Mutex::new(Vec::new()),
        outcome: AskOutcome {
            answers: vec![AskAnswer {
                question_id: "q1".to_owned(),
                selected: vec!["A".to_owned()],
                value: None,
            }],
            timed_out: None,
        },
    });
    let bound: Arc<Arc<dyn Asker>> = Arc::new(asker.clone() as Arc<dyn Asker>);
    let out = AskTool
        .execute(&ctx_with(bound), &args())
        .expect("the tool answers");

    let seen = asker.seen.lock().expect("asker lock");
    assert_eq!(seen.len(), 1, "one asker call per tool call");
    let q = &seen[0][0];
    assert_eq!(q.id, "q1", "ids are assigned by the tool, not the model");
    assert_eq!(q.header, "Library");
    assert_eq!(q.prompt, "which library");
    assert_eq!(q.options.len(), 2);
    assert_eq!(q.options[1].description, "the second");
    assert!(q.multi, "multiSelect rides through");
    assert_eq!(
        q.reason, "user_decision",
        "an unstated reason reads as a decision"
    );

    assert_eq!(out, "The user answered:\n- Library: A");
}

#[test]
fn a_timeout_is_rendered_as_a_skip_not_as_an_empty_answer() {
    let asker = Arc::new(RecordingAsker {
        seen: std::sync::Mutex::new(Vec::new()),
        outcome: AskOutcome {
            answers: Vec::new(),
            timed_out: Some(std::time::Duration::from_secs(30)),
        },
    });
    let bound: Arc<Arc<dyn Asker>> = Arc::new(asker.clone() as Arc<dyn Asker>);
    let out = AskTool
        .execute(&ctx_with(bound), &args())
        .expect("the tool answers");
    assert!(out.starts_with("skipped:"), "{out}");
    assert!(
        out.contains("within 30 seconds"),
        "the wait is named so the model can say what was skipped: {out}"
    );
    assert!(
        !out.contains("The user answered"),
        "a timeout is not an answer: {out}"
    );
}

#[test]
fn a_dismissal_stops_the_model_instead_of_reading_as_an_empty_answer() {
    let asker = Arc::new(RecordingAsker {
        seen: std::sync::Mutex::new(Vec::new()),
        outcome: AskOutcome::default(),
    });
    let bound: Arc<Arc<dyn Asker>> = Arc::new(asker.clone() as Arc<dyn Asker>);
    let out = AskTool
        .execute(&ctx_with(bound), &args())
        .expect("the tool answers");
    assert!(out.starts_with("The user dismissed"), "{out}");
    assert!(out.contains("stop and wait for the user's next message"));
}

#[test]
fn a_missing_value_reason_is_the_one_that_survives() {
    let asker = Arc::new(RecordingAsker {
        seen: std::sync::Mutex::new(Vec::new()),
        outcome: AskOutcome::default(),
    });
    let bound: Arc<Arc<dyn Asker>> = Arc::new(asker.clone() as Arc<dyn Asker>);
    let mut a = args();
    a["questions"][0]["reason"] = json!("missing_value");
    AskTool.execute(&ctx_with(bound), &a).expect("answers");
    let seen = asker.seen.lock().expect("asker lock");
    assert_eq!(seen[0][0].reason, "missing_value");
}

#[test]
fn an_unknown_reason_reads_as_a_decision() {
    let asker = Arc::new(RecordingAsker {
        seen: std::sync::Mutex::new(Vec::new()),
        outcome: AskOutcome::default(),
    });
    let bound: Arc<Arc<dyn Asker>> = Arc::new(asker.clone() as Arc<dyn Asker>);
    let mut a = args();
    a["questions"][0]["reason"] = json!("because");
    AskTool.execute(&ctx_with(bound), &a).expect("answers");
    let seen = asker.seen.lock().expect("asker lock");
    assert_eq!(
        seen[0][0].reason, "user_decision",
        "the reading that keeps the question in front of the user"
    );
}

#[test]
fn the_go_argument_errors_are_typed() {
    let ctx = CallContext::new();
    let cases = [
        (
            json!({"questions": []}),
            "at least one question is required",
        ),
        (
            json!({"questions": [{"header": "h", "question": "", "options": [{"label": "a"}, {"label": "b"}]}]}),
            "question 1: a question and at least two options are required",
        ),
        (
            json!({"questions": [{"header": "h", "question": "q", "options": [{"label": "a"}]}]}),
            "question 1: a question and at least two options are required",
        ),
        (
            json!({"questions": [{"header": "h", "question": "q", "options": [{"label": "a"}, {"label": " "}]}]}),
            "question 1 option 2: label is required",
        ),
        (
            json!({"questions": [{"header": "h", "question": "q", "options": [{"label": "a"}, {"label": "a"}]}]}),
            "question 1 option 2: duplicate label \"a\" also used by option 1",
        ),
    ];
    for (args, want) in cases {
        let err = AskTool.execute(&ctx, &args).unwrap_err();
        assert!(err.to_string().contains(want), "want {want:?}, got {err}");
    }
}

#[test]
fn no_asker_leaves_the_decision_unresolved() {
    let out = AskTool
        .execute(&CallContext::new(), &args())
        .expect("a literal, not an error");
    assert!(out.starts_with("unresolved:"), "{out}");
    assert!(out.contains("Do not choose on their behalf"));
}
