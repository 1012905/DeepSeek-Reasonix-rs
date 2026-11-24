//! TOOL-014 elicitation seam (Go `internal/contract/tool/elicit.go`,
//! docs/contracts/tools.md §1.19): the field/request/reply shapes, the
//! [`Elicitor`] port, and the call-context binding. This slice owns no
//! model-facing tool and no length caps; the MCP inbound caps belong to
//! reasonix-ext (MCP-029 todo), the ACP outbound caps to reasonix-acp.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use reasonix_tools::abi::CallContext;
use reasonix_tools::builtin::{ElicitField, ElicitReply, ElicitRequest, Elicitor, elicit_from};

struct RecordingElicitor {
    seen: std::sync::Mutex<Vec<ElicitRequest>>,
    reply: ElicitReply,
}

impl Elicitor for RecordingElicitor {
    fn elicit(
        &self,
        req: &ElicitRequest,
    ) -> Result<ElicitReply, Box<dyn std::error::Error + Send + Sync>> {
        self.seen.lock().expect("seen lock").push(req.clone());
        Ok(self.reply.clone())
    }
}

fn bound(elicitor: Arc<RecordingElicitor>) -> CallContext {
    let port: Arc<dyn Elicitor> = elicitor;
    CallContext::new().with_value(Arc::new(port))
}

#[test]
fn form_and_url_request_shapes_match_the_go_contract() {
    let form = ElicitRequest {
        source: "deployer".to_owned(),
        message: "Who?".to_owned(),
        note: String::new(),
        fields: vec![
            ElicitField {
                name: "name".to_owned(),
                title: "Name".to_owned(),
                description: "full name".to_owned(),
                choices: Vec::new(),
                multi: false,
                default: Vec::new(),
            },
            ElicitField {
                name: "env".to_owned(),
                title: "Env".to_owned(),
                description: String::new(),
                choices: vec!["prod".to_owned(), "dev".to_owned()],
                multi: true,
                default: vec!["dev".to_owned()],
            },
        ],
        url: String::new(),
    };
    assert!(!form.is_url());
    assert_eq!(form.fields.len(), 2);
    assert!(form.fields[1].multi);
    assert_eq!(form.fields[1].default, vec!["dev"]);

    let url = ElicitRequest {
        url: "https://example.com/complete".to_owned(),
        ..form
    };
    assert!(
        url.is_url(),
        "a non-empty URL selects the action-only interaction"
    );
}

#[test]
fn a_refusal_is_a_reply_not_an_error_and_url_replies_carry_no_values() {
    let declined = ElicitReply {
        declined: true,
        values: BTreeMap::new(),
        action: "decline".to_owned(),
    };
    let stub = Arc::new(RecordingElicitor {
        seen: std::sync::Mutex::new(Vec::new()),
        reply: declined.clone(),
    });
    let ctx = bound(stub.clone());
    let reply = elicit_from(&ctx)
        .expect("elicitor installed")
        .elicit(&ElicitRequest::default())
        .expect("a refusal answers, it never errors");
    assert!(reply.declined);
    assert!(reply.values.is_empty());
    assert_eq!(reply.action, "decline");
}

#[test]
fn the_elicitor_binding_round_trips_and_a_headless_run_binds_nobody() {
    let stub = Arc::new(RecordingElicitor {
        seen: std::sync::Mutex::new(Vec::new()),
        reply: ElicitReply::default(),
    });
    let ctx = bound(stub.clone());
    let port = elicit_from(&ctx).expect("elicitor installed");
    let req = ElicitRequest {
        source: "mcp".to_owned(),
        message: "open?".to_owned(),
        ..ElicitRequest::default()
    };
    let reply = port.elicit(&req).expect("elicit answers");
    assert!(reply.declined || reply.values.is_empty());
    let seen = stub.seen.lock().expect("seen lock").clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].source, "mcp");

    assert!(elicit_from(&CallContext::new()).is_none());
}
