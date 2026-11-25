//! Workspace grants and trust states (docs/contracts/config.md §10.18-§10.19;
//! project_grants.go, workspace_trust.go, permrule/permrule.go).
//!
//! A grant records what the user already allowed one workspace folder:
//! permission rules answered "always", extra writable directories, and the
//! folder's trust decision. It lives under the user's home, keyed by the
//! folder's own path fingerprint — a grant has no digest to bind it, so it
//! must not follow a repository identity a checkout's `.git` could claim.
//!
//! This slice ports the rule syntax, the trust state machine, and the
//! in-memory grant map; the JSON record file, its lock, fingerprinting and
//! best-effort load are wiring left to the integration layer (CONFIG-010 wip).

use std::collections::BTreeMap;

/// Why a grant or trust decision was refused (project_grants.go Update,
/// workspace_trust.go SetTrust).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GrantError {
    /// A persisted allow rule does not match `ToolName` or `ToolName(glob)`.
    #[error("invalid permission rule {rule:?} (want \"ToolName\" or \"ToolName(glob)\")")]
    InvalidRule { rule: String },
    /// A trust value is neither of the two recorded answers.
    #[error("workspace trust {trust:?}: must be trusted or declined")]
    InvalidTrust { trust: String },
}

/// A permission rule: the tool name plus an optional subject constraint
/// (permrule.Rule). A non-empty subject matches as a glob unless `literal`
/// marks an exact-match `ToolName=literal` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: String,
    pub subject: String,
    pub literal: bool,
}

/// Parses `ToolName`, `ToolName(glob)`, or the legacy `ToolName=literal` form
/// (permrule.Parse). Surrounding whitespace is trimmed; `None` for a malformed
/// entry (empty tool name) so a caller can warn rather than install a rule
/// that matches nothing.
pub fn parse_rule(raw: &str) -> Option<Rule> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(eq) = s.find('=') {
        let paren = s.find('(');
        if eq > 0 && (paren.is_none() || eq < paren.unwrap()) {
            let tool = s[..eq].trim();
            if tool.is_empty() {
                return None;
            }
            return Some(Rule {
                tool: tool.to_owned(),
                subject: s[eq + 1..].to_owned(),
                literal: true,
            });
        }
    }
    if let Some(paren) = s.find('(')
        && s.ends_with(')')
    {
        let tool = s[..paren].trim();
        if tool.is_empty() {
            return None;
        }
        return Some(Rule {
            tool: tool.to_owned(),
            subject: s[paren + 1..s.len() - 1].to_owned(),
            literal: false,
        });
    }
    Some(Rule {
        tool: s.to_owned(),
        subject: String::new(),
        literal: false,
    })
}

/// The person's decision about one workspace folder (workspace_trust.go).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceTrust {
    /// No recorded answer: the folder asks again on the next gate.
    #[default]
    Undecided,
    /// The folder was trusted.
    Trusted,
    /// The folder was declined.
    Declined,
}

impl WorkspaceTrust {
    /// Empty string: undecided.
    pub const UNDECIDED: &'static str = "";
    /// Recorded trust answer.
    pub const TRUSTED: &'static str = "trusted";
    /// Recorded decline answer.
    pub const DECLINED: &'static str = "declined";

    /// The persisted wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Undecided => Self::UNDECIDED,
            Self::Trusted => Self::TRUSTED,
            Self::Declined => Self::DECLINED,
        }
    }

    /// Reads a recorded trust value. Anything but the two recorded answers
    /// reads as undecided (workspace_trust.go Trust).
    pub fn from_raw(raw: &str) -> Self {
        match raw {
            Self::TRUSTED => Self::Trusted,
            Self::DECLINED => Self::Declined,
            _ => Self::Undecided,
        }
    }

    /// Validates a value the user or wiring wants to record. Undecided forgets
    /// the decision; a third spelling is refused (workspace_trust.go SetTrust).
    pub fn validate(raw: &str) -> Result<Self, GrantError> {
        match raw {
            Self::UNDECIDED => Ok(Self::Undecided),
            Self::TRUSTED => Ok(Self::Trusted),
            Self::DECLINED => Ok(Self::Declined),
            other => Err(GrantError::InvalidTrust {
                trust: other.to_owned(),
            }),
        }
    }
}

impl std::fmt::Display for WorkspaceTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the user granted one workspace folder (project_grants.go ProjectGrant).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectGrant {
    /// Permission rules answered "always".
    pub allow: Vec<String>,
    /// Extra writable directories.
    pub allow_write: Vec<String>,
    /// The folder's trust decision.
    pub trust: WorkspaceTrust,
}

/// Validates every allow rule; the first malformed one is reported
/// (project_grants.go Update validation).
pub fn validate_allow_rules(rules: &[String]) -> Result<(), GrantError> {
    for rule in rules {
        if parse_rule(rule).is_none() {
            return Err(GrantError::InvalidRule {
                rule: rule.to_owned(),
            });
        }
    }
    Ok(())
}

/// Appends `extra` that are not already present, preserving base order
/// (project_scope.go appendMissing).
pub fn append_missing(base: &[String], extra: &[String]) -> Vec<String> {
    let mut out = base.to_vec();
    for value in extra {
        if !out.iter().any(|existing| existing == value) {
            out.push(value.clone());
        }
    }
    out
}

/// In-memory grant record, keyed by workspace fingerprint. The JSON store,
/// cross-process lock and file paths live in the wiring layer; this map is the
/// pure state the store mutates under lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantStore {
    workspaces: BTreeMap<String, ProjectGrant>,
}

impl GrantStore {
    /// Creates an empty record.
    pub fn new() -> Self {
        Self::default()
    }

    /// The grant recorded for `ws`; an unknown workspace gets the zero grant.
    pub fn grant(&self, ws: &str) -> ProjectGrant {
        self.workspaces.get(ws).cloned().unwrap_or_default()
    }

    /// The recorded trust for `ws`, tolerant of unknown spellings
    /// (workspace_trust.go Trust).
    pub fn trust(&self, ws: &str) -> WorkspaceTrust {
        self.grant(ws).trust
    }

    /// Replaces `ws`'s grant with what `edit` returns, validating the next
    /// allow rules before it is installed (project_grants.go Update).
    pub fn update(
        &mut self,
        ws: &str,
        edit: impl FnOnce(ProjectGrant) -> ProjectGrant,
    ) -> Result<(), GrantError> {
        let current = self.grant(ws);
        let next = edit(ProjectGrant {
            allow: current.allow.clone(),
            allow_write: current.allow_write.clone(),
            trust: current.trust,
        });
        validate_allow_rules(&next.allow)?;
        self.workspaces.insert(ws.to_owned(), next);
        Ok(())
    }

    /// Records a trust decision for `ws`; undecided forgets it
    /// (workspace_trust.go SetTrust).
    pub fn set_trust(&mut self, ws: &str, trust: WorkspaceTrust) -> Result<(), GrantError> {
        self.update(ws, |mut grant| {
            grant.trust = trust;
            grant
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_rule_shapes() {
        let rule = parse_rule("bash").expect("bare tool");
        assert_eq!(rule.tool, "bash");
        assert!(rule.subject.is_empty());
        assert!(!rule.literal);

        let rule = parse_rule("read_file(/tmp/**)").expect("glob subject");
        assert_eq!(rule.tool, "read_file");
        assert_eq!(rule.subject, "/tmp/**");
        assert!(!rule.literal);

        let rule = parse_rule("bash=ls -la").expect("legacy literal");
        assert_eq!(rule.tool, "bash");
        assert_eq!(rule.subject, "ls -la");
        assert!(rule.literal);
    }

    #[test]
    fn malformed_rules_are_rejected() {
        assert!(parse_rule("").is_none());
        assert!(parse_rule("   ").is_none());
        assert!(parse_rule("(glob)").is_none());
        // `=literal` has no tool name before the equals; permrule.Parse falls
        // through and keeps the whole spelling as a bare tool.
        let rule = parse_rule("=literal").expect("leading equals stays bare");
        assert_eq!(rule.tool, "=literal");
        // A paren without a closing brace stays a bare tool name.
        let rule = parse_rule("tool(no-close").expect("unclosed paren");
        assert_eq!(rule.tool, "tool(no-close");
    }

    #[test]
    fn equals_before_paren_wins_the_literal_form() {
        // permrule.Parse: the `=` decides literal only when it precedes any `(`.
        let rule = parse_rule("tool(a=b)").expect("equals after paren");
        assert_eq!(rule.tool, "tool");
        assert_eq!(rule.subject, "a=b");
        assert!(!rule.literal);
    }

    #[test]
    fn trust_values_round_trip_and_unknown_spellings_undecided() {
        assert_eq!(WorkspaceTrust::from_raw("trusted"), WorkspaceTrust::Trusted);
        assert_eq!(
            WorkspaceTrust::from_raw("declined"),
            WorkspaceTrust::Declined
        );
        assert_eq!(WorkspaceTrust::from_raw(""), WorkspaceTrust::Undecided);
        assert_eq!(WorkspaceTrust::from_raw("maybe"), WorkspaceTrust::Undecided);
        assert_eq!(WorkspaceTrust::Trusted.as_str(), "trusted");
        assert_eq!(WorkspaceTrust::Declined.as_str(), "declined");
        assert_eq!(WorkspaceTrust::Undecided.as_str(), "");
    }

    #[test]
    fn set_trust_validates_the_two_recorded_answers() {
        assert!(WorkspaceTrust::validate("trusted").is_ok());
        assert!(WorkspaceTrust::validate("declined").is_ok());
        assert!(WorkspaceTrust::validate("").is_ok());
        let err = WorkspaceTrust::validate("maybe").expect_err("third spelling refused");
        assert!(
            err.to_string()
                .contains("workspace trust \"maybe\": must be trusted or declined")
        );
    }

    #[test]
    fn update_rejects_an_invalid_allow_rule() {
        let mut store = GrantStore::new();
        let err = store
            .update("ws", |mut grant| {
                grant.allow = vec!["()".to_owned()];
                grant
            })
            .expect_err("malformed rule refused");
        assert!(err.to_string().contains("invalid permission rule \"()\""));
        assert!(store.grant("ws").allow.is_empty());
    }

    #[test]
    fn grant_validate_and_store_round_trip_per_workspace() {
        let mut store = GrantStore::new();
        store
            .update("ws-a", |mut grant| {
                grant.allow = vec!["read_file".to_owned()];
                grant.allow_write = vec!["/tmp/a".to_owned()];
                grant.trust = WorkspaceTrust::Trusted;
                grant
            })
            .expect("valid grant");
        store
            .update("ws-b", |mut grant| {
                grant.trust = WorkspaceTrust::Declined;
                grant
            })
            .expect("valid decline");

        assert_eq!(store.trust("ws-a"), WorkspaceTrust::Trusted);
        assert_eq!(store.trust("ws-b"), WorkspaceTrust::Declined);
        assert_eq!(store.trust("ws-unknown"), WorkspaceTrust::Undecided);

        let grant = store.grant("ws-a");
        assert_eq!(grant.allow, vec!["read_file".to_owned()]);
        assert_eq!(grant.allow_write, vec!["/tmp/a".to_owned()]);
    }

    #[test]
    fn set_trust_replaces_only_the_trust_decision() {
        let mut store = GrantStore::new();
        store
            .update("ws", |mut grant| {
                grant.allow_write = vec!["/tmp/a".to_owned()];
                grant
            })
            .expect("grant");
        store
            .set_trust("ws", WorkspaceTrust::Declined)
            .expect("decline");
        let grant = store.grant("ws");
        assert_eq!(grant.trust, WorkspaceTrust::Declined);
        assert_eq!(grant.allow_write, vec!["/tmp/a".to_owned()]);
    }

    #[test]
    fn append_missing_keeps_base_order_and_dedupes() {
        let merged = append_missing(
            &["a".to_owned(), "b".to_owned()],
            &["b".to_owned(), "c".to_owned()],
        );
        assert_eq!(merged, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    }
}
