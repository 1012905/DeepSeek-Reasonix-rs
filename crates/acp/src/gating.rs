//! Permission gating order, the ACP round-trip fail-closed decision, and the
//! fail-closed argv meta decision (dispatch.go:410-488; verdict.go:82-138).
//!
//! Two short-circuit layers sit in order: the kernel policy verdict first, and
//! only an `Ask` reaches the ACP adapter round-trip. Every round-trip failure —
//! missing binder, transport error, unparsable reply, non-`selected` outcome,
//! unknown option id — stays closed: the model receives a blocked result rather
//! than a hang. The real shellparse analyzer and the approval binder are
//! runtime wiring; this module owns the decision tables and the identities.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::permission::{posture_rank, within_ceiling};

// ---- Layer 1: kernel posture admission ----------------------------------------

/// Where a gated tool call lands after the posture/ceiling decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Approved without asking the client.
    Grant,
    /// Must go through the `session/request_permission` round-trip.
    Prompt,
    /// Refused before any round-trip.
    Deny,
}

/// Admission order (verdict.go:98-121; dispatch.go:412-415):
/// 1. Above the startup ceiling refuses outright, before any prompt.
/// 2. `readOnly` / `dontAsk` refuse.
/// 3. `ask` routes to the round-trip.
/// 4. `auto` / `yolo` grant.
pub fn admit(posture: &str, ceiling: &str) -> Admission {
    if !within_ceiling(posture, ceiling) {
        return Admission::Deny;
    }
    match posture_rank(posture) {
        0 | 1 => Admission::Deny, // readOnly, dontAsk
        2 => Admission::Prompt,   // ask
        _ => Admission::Grant,    // auto, yolo
    }
}

// ---- Layer 2: ACP round-trip fail-closed decision -----------------------------

/// Resolved approval, after the option id is mapped back to the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Approval {
    pub allow: bool,
    pub session: bool,
    pub persist: bool,
}

impl Approval {
    fn denied() -> Self {
        Self {
            allow: false,
            session: false,
            persist: false,
        }
    }
}

/// Official option kinds stay on the wire's four-value enum
/// (protocol.go:750-755). `allow_persist` is a vendor option id whose wire
/// kind still encodes `allow_always` (dispatch.go:596, 608).
pub const KIND_ALLOW_ONCE: &str = "allow_once";
pub const KIND_ALLOW_ALWAYS: &str = "allow_always";
pub const KIND_REJECT_ONCE: &str = "reject_once";
pub const KIND_REJECT_ALWAYS: &str = "reject_always";

/// Maps a selected option id to the controller approval. Any unmatched id —
/// including a vendor persist id the adapter does not recognize — stays denied
/// (dispatch.go:440-447).
pub fn resolve_option_id(option_id: &str) -> Approval {
    match option_id {
        "allow_once" => Approval {
            allow: true,
            session: false,
            persist: false,
        },
        "allow_always" => Approval {
            allow: true,
            session: true,
            persist: false,
        },
        "allow_persist" => Approval {
            allow: true,
            session: true,
            persist: true,
        },
        _ => Approval::denied(),
    }
}

/// Round-trip inputs the adapter supplies after the binder call. Every `Err`
/// or `None` here is a fail-closed branch; there is no "pending" outcome.
pub enum RoundTripInput {
    /// Binder was absent before any request was sent.
    NoBinder,
    /// Transport layer returned an error.
    TransportFailed,
    /// Reply could not parse as a permission result.
    MalformedReply,
    /// Reply parsed, but outcome was not `selected`.
    OutcomeNotSelected,
    /// `selected` with a resolved option id.
    Selected { option_id: String },
}

/// Runs the fail-closed decision. Only `Selected` with a recognized id opens
/// anything; all other branches deny (dispatch.go:413-447, 570-571).
pub fn resolve_round_trip(input: RoundTripInput) -> Approval {
    match input {
        RoundTripInput::Selected { option_id } => resolve_option_id(&option_id),
        _ => Approval::denied(),
    }
}

// ---- argv fail-closed meta -----------------------------------------------------

/// Static parse rejection reason, carried on the parse side only. These reasons
/// never ride the ACP wire (bash.go:33-39; matrix A §2.2 note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StaticRejectReason {
    /// `"parse error"`: bash syntax could not be parsed.
    #[error("parse error")]
    ParseError,
    /// `"here document"`: a here-doc is present.
    #[error("here document")]
    HereDocument,
    /// `"shell control syntax"`: multi-statement / negation / background /
    /// goroutine / disown / non-call expression.
    #[error("shell control syntax")]
    ShellControlSyntax,
    /// `"shell redirection"`: a redirection the policy disallows.
    #[error("shell redirection")]
    ShellRedirection,
    /// `"shell assignment"`: environment assignment, or assignment-only command.
    #[error("shell assignment")]
    ShellAssignment,
    /// `"shell expansion"": non-static word (expansion / quoted variable).
    #[error("shell expansion")]
    ShellExpansion,
}

/// Features the runtime shellparse analyzer reports back. The pure decision
/// treats any flagged feature as non-exact (dispatch.go:477-488).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StaticFeatures {
    pub dynamic_command_name: bool,
    pub nested_execution: bool,
    pub expansion: bool,
    pub assignment: bool,
    pub redirection: bool,
    pub unquoted_glob: bool,
    pub tilde_args: bool,
    pub here_document: bool,
}

/// Decides whether the argv may be trusted as an exact static command. Trusted
/// requires an empty feature set, a non-empty argv, and no tilde-prefixed arg
/// (tilde expansion depends on the shell).
pub fn exact_argv(features: &StaticFeatures, argv: &[String]) -> Result<(), StaticRejectReason> {
    if argv.is_empty() {
        return Err(StaticRejectReason::ParseError);
    }
    if features.dynamic_command_name {
        return Err(StaticRejectReason::ShellControlSyntax);
    }
    if features.nested_execution {
        return Err(StaticRejectReason::ShellControlSyntax);
    }
    if features.expansion {
        return Err(StaticRejectReason::ShellExpansion);
    }
    if features.assignment {
        return Err(StaticRejectReason::ShellAssignment);
    }
    if features.redirection {
        return Err(StaticRejectReason::ShellRedirection);
    }
    if features.here_document {
        return Err(StaticRejectReason::HereDocument);
    }
    if features.unquoted_glob || features.tilde_args {
        return Err(StaticRejectReason::ShellExpansion);
    }
    Ok(())
}

/// Builds the `_meta.reasonix.io` argv block. On a non-exact decision the
/// argv, cwd, and schema version are all omitted and the call stays interactive
/// (dispatch.go:477-488); `rawInput` still carries the original command string.
pub fn argv_meta(trusted: bool, argv: &[String], cwd: &str) -> BTreeMap<String, Value> {
    let mut meta = BTreeMap::new();
    if !trusted || argv.is_empty() || cwd.is_empty() {
        return meta;
    }
    meta.insert("commandSchemaVersion".to_owned(), Value::from(1));
    meta.insert("argv".to_owned(), Value::from(argv));
    meta.insert("cwd".to_owned(), Value::from(cwd));
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Above-ceiling and read-only posture refuse before prompting.
    fn admission_short_circuits_in_order() {
        assert_eq!(admit("yolo", "ask"), Admission::Deny);
        assert_eq!(admit("readOnly", "yolo"), Admission::Deny);
        assert_eq!(admit("dontAsk", "yolo"), Admission::Deny);
        assert_eq!(admit("ask", "yolo"), Admission::Prompt);
        assert_eq!(admit("auto", "yolo"), Admission::Grant);
        assert_eq!(admit("yolo", "yolo"), Admission::Grant);
    }

    #[test]
    fn option_id_mapping_and_unknown_stay_closed() {
        assert_eq!(
            resolve_option_id("allow_once"),
            Approval {
                allow: true,
                session: false,
                persist: false,
            }
        );
        assert_eq!(
            resolve_option_id("allow_always"),
            Approval {
                allow: true,
                session: true,
                persist: false,
            }
        );
        assert_eq!(
            resolve_option_id("allow_persist"),
            Approval {
                allow: true,
                session: true,
                persist: true,
            }
        );
        assert!(!resolve_option_id("reject_once").allow);
        assert!(!resolve_option_id("bogus").allow);
    }

    #[test]
    fn every_round_trip_failure_stays_closed() {
        for input in [
            RoundTripInput::NoBinder,
            RoundTripInput::TransportFailed,
            RoundTripInput::MalformedReply,
            RoundTripInput::OutcomeNotSelected,
        ] {
            assert_eq!(resolve_round_trip(input), Approval::denied());
        }
        assert!(
            resolve_round_trip(RoundTripInput::Selected {
                option_id: "allow_once".to_owned(),
            })
            .allow
        );
    }

    #[test]
    fn exact_argv_requires_clean_features_and_args() {
        let clean = StaticFeatures::default();
        assert!(exact_argv(&clean, &["ls".to_owned(), "src".to_owned()]).is_ok());
        assert!(exact_argv(&clean, &[]).is_err());

        let expansion = StaticFeatures {
            expansion: true,
            ..Default::default()
        };
        assert_eq!(
            exact_argv(&expansion, &["echo".to_owned(), "$x".to_owned()]).unwrap_err(),
            StaticRejectReason::ShellExpansion
        );
    }

    #[test]
    fn argv_meta_omits_everything_when_untrusted() {
        let trusted = argv_meta(true, &["ls".to_owned()], "/proj");
        assert_eq!(trusted["commandSchemaVersion"], 1);
        assert_eq!(trusted["argv"][0], "ls");
        assert_eq!(trusted["cwd"], "/proj");

        let untrusted = argv_meta(false, &["ls".to_owned()], "/proj");
        assert!(untrusted.is_empty());
    }
}
