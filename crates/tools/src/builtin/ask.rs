//! The `ask` tool (Go `internal/runtime/agent/ask.go`): the way a turn asks
//! the user for something only they can supply. It reaches the user through
//! the [`crate::ask::Asker`] bound to the call context; a run with no asker
//! leaves the decision unresolved instead of inventing one, because a run
//! with nobody to ask is exactly a run that cannot settle this.

use std::any::Any;
use std::sync::Arc;

use reasonix_contract::eventwire::ask::{AskOption, AskQuestion};
use serde_json::Value;

use crate::abi::{CallContext, Capability, CapabilitySet, DecisionBarrier, Tool, ToolResult};
use crate::ask::{AskAnswer, asker_from};
use crate::registry::{REACH_HOST_CONTROL, ReachDeclarer};

/// `ask`, the tool name the model calls.
pub const ASK_TOOL_NAME: &str = "ask";

/// Why only the user can answer (Go `event.AskReason*`). Only the exact
/// `missing_value` spelling is kept; anything else reads as a decision,
/// which keeps a question in front of the user.
pub const ASK_REASON_USER_DECISION: &str = "user_decision";
pub const ASK_REASON_MISSING_VALUE: &str = "missing_value";

/// The literal a headless run answers with (Go: the no-asker arm). It tells
/// the model to leave the decision unresolved rather than choose.
pub const ASK_UNRESOLVED: &str = "unresolved: this run has no interactive user, and the answer is the user's to give — \
nothing here can supply it, including you. Do not choose on their behalf. Do whatever the task \
allows without this decision, then say in your reply which decision is missing.";

/// The user's dismissal (Go `formatAnswers`' no-selection arm): read as
/// "don't decide for me", never as license to proceed.
pub const ASK_DISMISSED: &str = "The user dismissed the question without choosing — read this as \"don't decide for me, \
let's just talk.\" Do not pick an option, run a tool, or take any further action toward this; \
stop and wait for the user's next message.";

/// The `ask` tool (Go `AskTool`).
#[derive(Debug, Default, Clone, Copy)]
pub struct AskTool;

/// The one instance a composition root registers.
pub fn ask_tool() -> Arc<dyn Tool> {
    Arc::new(AskTool)
}

/// Marks a call whose result is the user's answer: the round ends there, and
/// calls written alongside it were written without the answer (Go
/// `DecisionBarrier`).
struct AlwaysBarrier;

impl Capability for AlwaysBarrier {}
impl DecisionBarrier for AlwaysBarrier {
    fn decision_barrier(&self) -> bool {
        true
    }
}

struct HostControlReach;

impl Capability for HostControlReach {}
impl ReachDeclarer for HostControlReach {
    fn reach(&self) -> u8 {
        REACH_HOST_CONTROL
    }
}

/// Go `ask.Schema()`, verbatim.
const GO_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "questions":{
    "type":"array",
    "minItems":1,
    "maxItems":4,
    "description":"1-4 questions to ask together.",
    "items":{
      "type":"object",
      "properties":{
        "header":{"type":"string","description":"Very short label for the question (a tab title), e.g. \"Library\"."},
        "question":{"type":"string","description":"The full question to ask."},
        "options":{
          "type":"array","minItems":2,"maxItems":4,
          "description":"The choices. Put any recommended option first.",
          "items":{
            "type":"object",
            "properties":{
              "label":{"type":"string","description":"The choice text (concise)."},
              "description":{"type":"string","description":"Optional one-line explanation of the choice."}
            },
            "required":["label"]
          }
        },
        "reason":{"type":"string","enum":["user_decision","missing_value"],"description":"Why only the user can answer: user_decision when they must pick between real alternatives, missing_value when the request never supplied something you need. Nothing else belongs here."},
        "multiSelect":{"type":"boolean","description":"Allow selecting more than one option."}
      },
      "required":["question","header","options"]
    }
  }
},
"required":["questions"]
}"#;

/// Go `ask.Description()`, verbatim.
pub const ASK_DESCRIPTION: &str = "Ask the user for something only they can supply: a decision that is theirs to make, or a value the request never gave you. Use it when two or more genuinely reasonable options would change the architecture, public API, data model, dependencies, UX, compatibility, scope, or anything irreversible — having a recommendation does not make such a choice yours to make, so put the recommended option first and still ask. Everything else is not a question for the user: whether you may touch a file is permission, whether a plan may run is the plan card, and where something lives or why a test failed is yours to go find out. Purely local, low-risk, reversible implementation details take the sensible default without asking. Calling this ends the round — anything you write alongside it will not run, because it would be reasoning from an answer you do not have yet. No tool-approval mode, YOLO included, answers for the user. Each question has a short `header` (a tab label), the `question` text, 2-4 `options` (each a `label` and optional `description`), and `multiSelect` when more than one may apply.";

impl Tool for AskTool {
    fn name(&self) -> &str {
        ASK_TOOL_NAME
    }

    fn description(&self) -> &str {
        ASK_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(GO_SCHEMA).expect("the ask schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let questions = parse_questions(args)?;
        let Some(asker) = asker_from(ctx) else {
            return Ok(ASK_UNRESOLVED.to_owned());
        };
        let outcome = asker.ask(&questions);
        Ok(render_outcome(&questions, &outcome))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "decision_barrier",
                Arc::new(AlwaysBarrier) as Arc<dyn DecisionBarrier>,
            )
            .with(
                "reach",
                Arc::new(HostControlReach) as Arc<dyn ReachDeclarer>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Parses and validates the model's questions (Go `AskTool.Execute`'s
/// argument arm), assigning `q1..qn` ids the answers correlate back to.
fn parse_questions(
    args: &Value,
) -> Result<Vec<AskQuestion>, Box<dyn std::error::Error + Send + Sync>> {
    let raw = args
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if raw.is_empty() {
        return Err(invalid("at least one question is required"));
    }
    let mut out = Vec::with_capacity(raw.len());
    for (i, q) in raw.iter().enumerate() {
        let question = q
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let opts_raw = q
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if question.is_empty() || opts_raw.len() < 2 {
            return Err(invalid(format!(
                "question {}: a question and at least two options are required",
                i + 1
            )));
        }
        let mut options = Vec::with_capacity(opts_raw.len());
        let mut seen: Vec<String> = Vec::with_capacity(opts_raw.len());
        for (j, o) in opts_raw.iter().enumerate() {
            let label = o.get("label").and_then(Value::as_str).unwrap_or("").trim();
            if label.is_empty() {
                return Err(invalid(format!(
                    "question {} option {}: label is required",
                    i + 1,
                    j + 1
                )));
            }
            if let Some(prev) = seen.iter().position(|s| s == label) {
                return Err(invalid(format!(
                    "question {} option {}: duplicate label {:?} also used by option {}",
                    i + 1,
                    j + 1,
                    label,
                    prev + 1
                )));
            }
            seen.push(label.to_owned());
            options.push(AskOption {
                label: label.to_owned(),
                description: o
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned(),
            });
        }
        out.push(AskQuestion {
            id: format!("q{}", i + 1),
            header: q
                .get("header")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned(),
            prompt: question.to_owned(),
            reason: normalize_reason(q.get("reason").and_then(Value::as_str).unwrap_or("")),
            options,
            multi: q
                .get("multiSelect")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ..AskQuestion::default()
        });
    }
    Ok(out)
}

fn invalid(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

/// Normalizes the model's reason to the two the host recognises (Go
/// `askReason`): an unstated or unknown reason reads as a decision.
fn normalize_reason(raw: &str) -> String {
    if raw.trim() == ASK_REASON_MISSING_VALUE {
        ASK_REASON_MISSING_VALUE.to_owned()
    } else {
        ASK_REASON_USER_DECISION.to_owned()
    }
}

/// Renders what the model reads for one resolved ask (Go: `Execute`'s
/// timeout arm plus `formatAnswers`).
fn render_outcome(questions: &[AskQuestion], outcome: &crate::ask::AskOutcome) -> String {
    if let Some(after) = outcome.timed_out {
        return format!(
            "skipped: the user did not answer within {} seconds, so the host skipped this question under their ask-timeout setting. \
That is a timeout, not an answer: no option was chosen for them and none counts as their preference. \
Carry on with whatever the task allows without this decision and state the assumption you proceed under in your reply, \
and say in your reply which decision is missing.",
            after.as_secs()
        );
    }
    format_answers(questions, &outcome.answers)
}

/// Renders the selections keyed by question header (Go `formatAnswers`).
/// Nothing selected at all is the dismissal, said as a stop rather than as
/// a per-question "(no answer)" the model could read as license to proceed.
fn format_answers(questions: &[AskQuestion], answers: &[AskAnswer]) -> String {
    let pick = |id: &str| -> &[String] {
        answers
            .iter()
            .find(|a| a.question_id == id)
            .map(|a| a.selected.as_slice())
            .unwrap_or_default()
    };
    if !questions.iter().any(|q| !pick(&q.id).is_empty()) {
        return ASK_DISMISSED.to_owned();
    }
    let mut out = String::from("The user answered:");
    for q in questions {
        let selected = pick(&q.id);
        let label = if q.header.is_empty() {
            q.prompt.as_str()
        } else {
            q.header.as_str()
        };
        if selected.is_empty() {
            out.push_str(&format!(
                "\n- {label}: (left unanswered — don't assume a choice)"
            ));
            continue;
        }
        out.push_str(&format!("\n- {label}: {}", selected.join(", ")));
    }
    out
}

/// The ask tool's schema as one JSON value, for tests and contract reads.
pub fn schema_value() -> Value {
    serde_json::from_str(GO_SCHEMA).expect("the ask schema is valid JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_shape_matches_the_go_contract() {
        let s = schema_value();
        assert_eq!(s["type"], "object");
        assert_eq!(s["required"], json!(["questions"]));
        let item = &s["properties"]["questions"]["items"];
        assert_eq!(item["required"], json!(["question", "header", "options"]));
        assert_eq!(item["properties"]["options"]["minItems"], 2);
        assert_eq!(item["properties"]["options"]["maxItems"], 4);
        assert_eq!(
            item["properties"]["reason"]["enum"],
            json!(["user_decision", "missing_value"])
        );
    }

    #[test]
    fn description_and_flags_match_go() {
        let t = AskTool;
        assert_eq!(t.name(), "ask");
        assert_eq!(t.description(), ASK_DESCRIPTION);
        assert!(t.read_only(), "asking has no host side effects");
        assert!(
            crate::abi::is_decision_barrier(&t),
            "the answer ends the round"
        );
        assert_eq!(
            crate::registry::reach_of(&t),
            REACH_HOST_CONTROL,
            "an ask stays inside the host"
        );
    }

    #[test]
    fn validation_refuses_the_go_shapes() {
        let ctx = CallContext::new();
        let err = AskTool
            .execute(&ctx, &json!({"questions": []}))
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("at least one question is required")
        );
        let err = AskTool
            .execute(
                &ctx,
                &json!({"questions":[{"header":"h","question":"q","options":[{"label":"a"}]}]}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("at least two options"));
        let err = AskTool
            .execute(
                &ctx,
                &json!({"questions":[{"header":"h","question":"q","options":[{"label":"a"},{"label":"a"}]}]}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("duplicate label"));
    }

    #[test]
    fn a_run_without_an_asker_leaves_the_decision_unresolved() {
        let out = AskTool
            .execute(
                &CallContext::new(),
                &json!({"questions":[{"header":"h","question":"q","options":[{"label":"a"},{"label":"b"}]}]}),
            )
            .expect("the tool answers with a literal");
        assert!(out.starts_with("unresolved:"), "{out}");
        assert!(!out.contains("The user answered"));
    }

    #[test]
    fn reason_normalizes_to_the_two_the_host_recognises() {
        assert_eq!(
            normalize_reason(" missing_value "),
            ASK_REASON_MISSING_VALUE
        );
        assert_eq!(normalize_reason("user_decision"), ASK_REASON_USER_DECISION);
        assert_eq!(normalize_reason("something else"), ASK_REASON_USER_DECISION);
        assert_eq!(normalize_reason(""), ASK_REASON_USER_DECISION);
    }

    #[test]
    fn no_selections_reads_as_a_dismissal_not_as_empty_answers() {
        let q = [AskQuestion {
            id: "q1".to_owned(),
            header: "Library".to_owned(),
            prompt: "which".to_owned(),
            ..AskQuestion::default()
        }];
        assert_eq!(format_answers(&q, &[]), ASK_DISMISSED);
        let answered = [crate::ask::AskAnswer {
            question_id: "q1".to_owned(),
            selected: vec!["B".to_owned()],
            value: None,
        }];
        assert_eq!(
            format_answers(&q, &answered),
            "The user answered:\n- Library: B"
        );
    }
}
