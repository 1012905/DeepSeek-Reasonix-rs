//! Permission decides, per tool call, whether to allow it, deny it, or ask
//! the user first. The core is a pure Policy (rule evaluation, no I/O); a
//! Gate wraps a Policy with an optional interactive Approver and is what the
//! agent consults at execute time.
//!
//! Parity target: `internal/safety/permission`. The bash-analysis machines
//! the reference reads from full ASTs sit behind [`bash::BashAnalysis`];
//! [`bash::StaticBashAnalysis`] answers from `reasonix-core::shellparse`'s
//! static port fail-closed until the AST slice lands.

pub mod bash;
#[cfg(test)]
mod bash_anchors;
pub mod browser;
pub mod computer;
pub mod decision;
pub mod gate;
pub mod inline_code;
pub mod network;
pub mod permrule;
pub mod policy;
pub mod rule_group;
pub mod session_grant_valid;
pub mod vocabulary;
pub mod write_fence;

pub(crate) mod shellparse {
    // The policy layer reads the static port through one alias so a later
    // engine swap never touches call sites.
    pub use reasonix_core::shellparse::{contains_shell_syntax, static_fields};
}

pub use bash::{
    BashAnalysis, BashApprovalBlocker, BashApprovalClass, StaticBashAnalysis, bash_analysis,
    set_bash_analysis,
};
pub use browser::{
    BROWSER_CREDENTIAL_PREFIX, BROWSER_SCRIPT_PREFIX, browser_subject_requires_explicit_approval,
};
pub use computer::{COMPUTER_POINTER_PREFIX, computer_subject_takes_pointer};
pub use decision::{Decision, match_glob, parse_decision, subject, subjects};
pub use gate::{
    Approver, Checker, Gate, REFUSAL_DECLINED, REFUSAL_DENY_RULE, REFUSAL_PARKED,
    REFUSAL_READ_ONLY, REFUSAL_UNATTENDED, REFUSAL_UNTRUSTED_FOLDER, Verdict,
    bash_command_is_read_only, is_refusal_code, posture_refusal, refusal_stands, verdict_of,
};
pub use inline_code::{argv_carries_inline_code, executable_base, is_inline_code_interpreter};
pub use permrule::Rule;
pub use policy::{
    Policy, bash_command_prefix, bash_danger_warning, remember_rule_for_scope, rule_covers_string,
    rule_matches_string, session_grant_rule_for_scope,
};
pub use session_grant_valid::valid_session_grant_rule;
pub use vocabulary::{UnknownToolError, Vocabulary};
pub use write_fence::{
    EXTEND_WRITE_PATHS, INSTALL_SOURCE_TOOL, MCP_SIGNIN, NETWORK_EGRESS, PAID_SEARCH,
    SELF_EXTEND_HUMAN_RISK, install_source_is_plan_only, session_grant_matches,
};

/// An error a caller must tell apart carries an identity, never a sentence.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GateError {
    /// The approver aborted (context cancelled while waiting).
    #[error("approval aborted")]
    Aborted,
}

impl GateError {
    pub fn aborted(_detail: &str) -> GateError {
        GateError::Aborted
    }
}
