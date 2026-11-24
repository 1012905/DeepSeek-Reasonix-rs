//! Elicitation seam (tools.md §1.19; Go `internal/contract/tool/elicit.go`).
//!
//! An external party (an MCP server, a plugin) asks the person to supply a
//! form's values or to accept/cancel an external interaction. The tool crate
//! owns only the request/reply shapes, the [`Elicitor`] port, and the
//! per-call context binding; the controller that turns a request into an
//! ask-card lives in the session slice, the MCP envelope parsing in the
//! plugin slice.
//!
//! A refusal is a reply, never an error: it goes back to the party and the
//! turn goes on. A [`None`] elicitor (headless runs, sub-agents with no
//! interactive parent) means nobody can be asked.
//!
//! Limits live in two other slices and are deliberately NOT mirrored here:
//! the MCP inbound caps (2000/200/32/64/3 runes) belong to `reasonix-ext`
//! (MCP-029 todo); the ACP outbound caps (1024/4096/120 runes) are already
//! landed in `reasonix-acp` (ACP-021). This contract layer carries shapes
//! only; it defines no character, rune, or field-count ceiling.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::abi::CallContext;

/// One value an external party asks the person to supply. With [`choices`]
/// it is picked from them (several when [`multi`]); without, it is typed.
/// [`default`] prefills: the party's default, or what was given last time.
///
/// [`choices`]: ElicitField::choices
/// [`multi`]: ElicitField::multi
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElicitField {
    pub name: String,
    pub title: String,
    pub description: String,
    pub choices: Vec<String>,
    pub multi: bool,
    pub default: Vec<String>,
}

/// Asks for an external party's form or URL interaction. A non-empty
/// [`url`] selects an action-only interaction; otherwise [`fields`] describes
/// the form.
///
/// [`url`]: ElicitRequest::url
/// [`fields`]: ElicitRequest::fields
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElicitRequest {
    pub source: String,
    pub message: String,
    pub note: String,
    pub fields: Vec<ElicitField>,
    pub url: String,
}

impl ElicitRequest {
    /// Whether this request selects the action-only URL interaction.
    pub fn is_url(&self) -> bool {
        !self.url.is_empty()
    }
}

/// Carries form values or a URL interaction's accept/decline/cancel
/// [`action`]. URL replies never carry [`values`]. The `action` spelling is
/// produced by the reply-mapping layer (MCP/ACP); this seam only stores it.
///
/// [`action`]: ElicitReply::action
/// [`values`]: ElicitReply::values
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElicitReply {
    pub declined: bool,
    pub values: BTreeMap<String, Vec<String>>,
    pub action: String,
}

/// Puts an external party's form in front of the person (Go `tool.Elicitor`).
/// A host with nobody to ask installs nothing; the caller fails closed rather
/// than inventing an answer.
pub trait Elicitor: Send + Sync {
    fn elicit(
        &self,
        req: &ElicitRequest,
    ) -> Result<ElicitReply, Box<dyn std::error::Error + Send + Sync>>;
}

/// The elicitor bound to one call, when the host installed one (Go
/// `tool.ElicitorFrom`).
pub fn elicit_from(ctx: &CallContext) -> Option<Arc<dyn Elicitor>> {
    ctx.value()?.downcast_ref::<Arc<dyn Elicitor>>().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::CallContext;

    struct RecordingElicitor {
        replies: std::sync::Mutex<Vec<ElicitRequest>>,
        reply: ElicitReply,
    }

    impl Elicitor for RecordingElicitor {
        fn elicit(
            &self,
            req: &ElicitRequest,
        ) -> Result<ElicitReply, Box<dyn std::error::Error + Send + Sync>> {
            self.replies.lock().expect("replies lock").push(req.clone());
            Ok(self.reply.clone())
        }
    }

    #[test]
    fn the_request_reply_shapes_match_the_go_contract() {
        let req = ElicitRequest {
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
        assert!(!req.is_url());
        assert_eq!(req.fields.len(), 2);
        assert!(req.fields[1].multi);
        assert_eq!(req.fields[1].default, vec!["dev"]);

        let url_req = ElicitRequest {
            url: "https://example.com/complete".to_owned(),
            ..req
        };
        assert!(url_req.is_url());
    }

    #[test]
    fn the_elicitor_binding_round_trips_through_the_call_context() {
        let stub = Arc::new(RecordingElicitor {
            replies: std::sync::Mutex::new(Vec::new()),
            reply: ElicitReply::default(),
        });
        let port: Arc<dyn Elicitor> = stub.clone();
        let ctx = CallContext::new().with_value(Arc::new(port));
        let bound = elicit_from(&ctx).expect("elicitor installed");
        let req = ElicitRequest {
            source: "mcp".to_owned(),
            message: "open?".to_owned(),
            ..ElicitRequest::default()
        };
        let reply = bound.elicit(&req).expect("elicit answers");
        assert!(reply.declined || reply.values.is_empty());
        let seen = stub.replies.lock().expect("replies lock").clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].source, "mcp");

        let empty = elicit_from(&CallContext::new());
        assert!(empty.is_none(), "a headless run binds nobody");
    }

    #[test]
    fn a_refusal_is_a_reply_and_url_replies_carry_no_values() {
        let declined = ElicitReply {
            declined: true,
            values: BTreeMap::new(),
            action: "decline".to_owned(),
        };
        assert!(declined.declined);
        assert!(declined.values.is_empty());
        assert_eq!(declined.action, "decline");
    }
}
