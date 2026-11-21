//! The request composer (Go: `buildSamplingRequest`'s assembly minus the
//! context-window and extension machines).
//!
//! One composition is the cache-stable prefix — the system prompt and the
//! tool schemas — followed by the transcript's provider projection.
//! Determinism per turn is the only contract: retries replay the exact
//! request this seam returned, and DeepSeek's prefix cache keys on these
//! bytes, so wall-clock metadata never crosses the transport (Go strips
//! `CreatedAt` from the request copy).
//!
//! What the context-window wave adds (Go `contextManager().Prepare`, run
//! between the session snapshot and the transport copy): the fold/projection
//! pass deciding which messages a request carries once the visible
//! transcript nears the model's window, the one-shot overflow recovery
//! (`recoverContextOverflow`), and the output-budget clip
//! (`effectiveOutputBudget`).

use reasonix_contract::provider::{Message, Role, model_messages};
use reasonix_provider::ChatCompletionRequest;
use reasonix_tools::Registry;

use crate::error::TurnError;
use crate::seams::{RequestBase, RequestComposer};

/// The composition every request carries when no seam replaces it (Go: the
/// inline body of `buildSamplingRequest` minus the machines not yet ported).
#[derive(Debug, Clone, Default)]
pub struct DefaultComposer {
    /// Coalesces adjacent user messages on the request copy (Go:
    /// `strictAlternatingRoles` — set for providers that require strict role
    /// alternation).
    strict_alternating_roles: bool,
}

impl DefaultComposer {
    pub fn new(strict_alternating_roles: bool) -> Self {
        Self {
            strict_alternating_roles,
        }
    }
}

impl RequestComposer for DefaultComposer {
    fn compose(
        &self,
        system: &str,
        session_messages: &[Message],
        registry: &Registry,
        base: &RequestBase,
    ) -> Result<ChatCompletionRequest, TurnError> {
        // The provider-visible projection (Go: provider.ModelMessages over
        // the prepared view): local-only records and their derived tails
        // stay out of the transport copy.
        let mut messages = model_messages(session_messages).into_owned();
        // Wall-clock metadata never reaches the transport: two requests
        // carrying the same conversation may differ in appended messages
        // only, or every turn cold-starts the prefix cache (Go: the
        // CreatedAt strip).
        for m in &mut messages {
            m.created_at = 0;
        }
        if self.strict_alternating_roles {
            messages = coalesce_user_runs(messages);
        }
        // The system prompt leads the request. The transcript may already
        // carry one (Go stores the composed prompt as session message zero),
        // so the configured prompt fills the head only when it does not: a
        // request never carries two.
        if !system.is_empty() && messages.first().map(|m| m.role.clone()) != Some(Role::SYSTEM) {
            messages.insert(
                0,
                Message {
                    role: Role::SYSTEM,
                    content: system.to_owned(),
                    ..Message::default()
                },
            );
        }
        // Registry order is stable name order (crates/tools registry), so
        // the tool contract is byte-stable across turns — the prefix cache
        // keys on it as much as on the prompt.
        let tools = registry
            .schemas()
            .into_iter()
            .map(|schema| reasonix_provider::ToolSchema {
                name: schema.name,
                description: schema.description,
                // An absent canonical schema reaches the transport empty;
                // the request build canonicalizes it (Go:
                // CanonicalizeSchema(nil)).
                parameters: if schema.parameters.is_null() {
                    String::new()
                } else {
                    serde_json::to_string(&schema.parameters).unwrap_or_default()
                },
            })
            .collect();
        Ok(ChatCompletionRequest {
            messages,
            tools,
            temperature: base.temperature,
            max_tokens: base.max_tokens,
            // The session's mode/effort dials (Go: sess.mode/sess.effort)
            // belong to the request-mode machine, not yet ported; the
            // override stays unset rather than guessed.
            effort_override: String::new(),
        })
    }
}

/// The tag a fold digest opens with (Go: `SummaryTagOpen`); the rule lives
/// in [`crate::window`], where the fold installs the digest.
use crate::window::is_compaction_summary;

/// Merges adjacent user messages into one run, in order (Go:
/// `coalesceProjectionUserRuns`). A fold digest rides ahead of the run it
/// folded into, because it describes what follows it. The merged run keeps
/// the head message's fields; its bytes belong to this request, not to what
/// the next one carries.
pub(crate) fn coalesce_user_runs(msgs: Vec<Message>) -> Vec<Message> {
    if msgs.len() < 2 {
        return msgs;
    }
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len());
    for msg in msgs {
        let joins = match out.last() {
            Some(prev) => msg.role == Role::USER && prev.role == Role::USER,
            None => false,
        };
        if !joins {
            out.push(msg);
            continue;
        }
        let prev = out.last_mut().expect("checked above");
        if is_compaction_summary(&msg) && !is_compaction_summary(prev) {
            let body = msg.content.trim_end_matches('\n').to_owned();
            prev.content = format!("{body}\n\n{}", prev.content);
        } else {
            let head = prev.content.trim_end_matches('\n').to_owned();
            prev.content = format!("{head}\n\n{}", msg.content);
        }
        prev.images.extend(msg.images);
        prev.tool_calls.extend(msg.tool_calls);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_tools::Registry;

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_owned(),
            ..Message::default()
        }
    }

    fn registry_with_one_tool() -> Registry {
        let registry = Registry::new();
        for tool in reasonix_tools::builtin::unconfined_readers() {
            registry.add(tool);
        }
        registry
    }

    #[test]
    fn the_configured_system_prompt_leads_a_transcript_without_one() {
        let composer = DefaultComposer::default();
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let req = composer
            .compose("the prefix", &[user_msg("hello")], &registry, &base)
            .expect("compose");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, Role::SYSTEM);
        assert_eq!(req.messages[0].content, "the prefix");
        assert_eq!(req.messages[1].role, Role::USER);
    }

    #[test]
    fn a_transcript_already_carrying_a_system_prompt_is_not_doubled() {
        let composer = DefaultComposer::default();
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let stored = Message {
            role: Role::SYSTEM,
            content: "stored prompt".to_owned(),
            ..Message::default()
        };
        let req = composer
            .compose(
                "configured prompt",
                &[stored, user_msg("hello")],
                &registry,
                &base,
            )
            .expect("compose");
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].content, "stored prompt");
    }

    #[test]
    fn wall_clock_metadata_never_reaches_the_request() {
        let composer = DefaultComposer::default();
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let mut m = user_msg("hello");
        m.created_at = 1_700_000_000_000;
        let req = composer
            .compose("", &[m], &registry, &base)
            .expect("compose");
        assert_eq!(req.messages[0].created_at, 0);
    }

    #[test]
    fn user_runs_coalesce_in_order_under_strict_alternation() {
        let composer = DefaultComposer::new(true);
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let msgs = vec![
            user_msg("first"),
            user_msg("second"),
            Message {
                role: Role::ASSISTANT,
                content: "reply".to_owned(),
                ..Message::default()
            },
            user_msg("third"),
            user_msg("fourth"),
        ];
        let req = composer
            .compose("", &msgs, &registry, &base)
            .expect("compose");
        let roles: Vec<&str> = req.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(req.messages[0].content, "first\n\nsecond");
        assert_eq!(req.messages[2].content, "third\n\nfourth");
    }

    #[test]
    fn a_fold_digest_rides_ahead_of_the_run_it_folded_into() {
        let composer = DefaultComposer::new(true);
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let digest = user_msg("<compaction-summary>\nearlier work\n</compaction-summary>");
        let msgs = vec![user_msg("older user text"), digest, user_msg("new text")];
        let req = composer
            .compose("", &msgs, &registry, &base)
            .expect("compose");
        assert_eq!(req.messages.len(), 1);
        let head = "<compaction-summary>\nearlier work\n</compaction-summary>";
        assert!(
            req.messages[0].content.starts_with(head),
            "{}",
            req.messages[0].content
        );
        assert!(req.messages[0].content.contains("older user text"));
        assert!(req.messages[0].content.ends_with("new text"));
    }

    #[test]
    fn the_tool_contract_is_stable_name_order() {
        let composer = DefaultComposer::default();
        let registry = registry_with_one_tool();
        let base = RequestBase::default();
        let a = composer
            .compose("", &[user_msg("hello")], &registry, &base)
            .expect("compose");
        let b = composer
            .compose("", &[user_msg("hello")], &registry, &base)
            .expect("compose");
        let names_a: Vec<String> = a.tools.iter().map(|t| t.name.clone()).collect();
        let names_b: Vec<String> = b.tools.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names_a, names_b);
        let mut sorted = names_a.clone();
        sorted.sort();
        assert_eq!(names_a, sorted);
    }
}
