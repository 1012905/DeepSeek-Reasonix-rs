//! Widening a delegated run's write confinement, and the tools whose
//! authorization reads their subject.

use super::permrule::parse as parse_permrule;

/// The capability a delegated run asks for when it needs to write outside
/// the paths it declared. It is not one of the run's tools: the subject is
/// the path being asked for, and the answer decides whether the fence moves,
/// not whether one write happens.
pub const EXTEND_WRITE_PATHS: &str = "extend_write_paths";

/// Asks whether bash may reach a host outside [sandbox] allowed_domains; the
/// subject is the host.
pub const NETWORK_EGRESS: &str = "network_egress";

/// Asks whether web_search may use a backend the vendor charges for; the
/// subject is the backend. Only the user can agree to be charged.
pub const PAID_SEARCH: &str = "paid_search";

/// Asks whether the browser may be sent to an authorization host to sign in
/// to an MCP server; the subject names the server and the host. The server
/// chose that host, so only the user answers.
pub const MCP_SIGNIN: &str = "mcp_signin";

/// The install-plan grade no blanket allow covers. It mirrors
/// installsource.RiskHigh, which the permission layer cannot import without
/// pulling the install stack underneath it.
pub const SELF_EXTEND_HUMAN_RISK: &str = "high:";

/// The rule-name form self-extension is decided under.
pub const INSTALL_SOURCE_TOOL: &str = "install_source";

/// Whether the tool's authorization reads its subject, and therefore a
/// person must answer whatever the fallback says.
pub(crate) fn subject_requires_human(tool_name: &str, subject: &str) -> bool {
    match super::policy::canonical_rule_tool(tool_name).as_str() {
        t if t == EXTEND_WRITE_PATHS
            || t == NETWORK_EGRESS
            || t == PAID_SEARCH
            || t == MCP_SIGNIN =>
        {
            true
        }
        t if t == INSTALL_SOURCE_TOOL => subject.starts_with(SELF_EXTEND_HUMAN_RISK),
        _ => false,
    }
}

/// A tool a session grant may not cover by name alone. A browser grant names
/// its origin and a computer grant its application.
pub(crate) fn subject_scoped_grant(tool_name: &str) -> bool {
    let canonical = super::policy::canonical_rule_tool(tool_name);
    matches!(
        canonical.as_str(),
        EXTEND_WRITE_PATHS | NETWORK_EGRESS | PAID_SEARCH | MCP_SIGNIN | INSTALL_SOURCE_TOOL
    ) || super::rule_group::is_browser_tool(tool_name)
        || super::rule_group::is_computer_tool(tool_name)
}

/// The rule a session grant records for tools that are not bash or a file
/// mutation. Approving one of these answered for that subject, never for the
/// tool, so the subject stays in the rule.
pub(crate) fn session_grant_rule(tool_name: &str, subject: &str) -> String {
    if !subject.is_empty() && subject_scoped_grant(tool_name) {
        format!("{tool_name}={subject}")
    } else {
        tool_name.to_string()
    }
}

/// Reports whether a recorded grant covers this call, and is stricter than
/// [`super::policy::rule_matches_string`] where the tool decides on its
/// subject: a bare-tool grant there predates grants carrying subjects and
/// stands for an answer nobody gave. It now covers nothing, so the question
/// is asked once more.
pub fn session_grant_matches(rule: &str, tool_name: &str, subject: &str) -> bool {
    if subject_scoped_grant(tool_name) && !rule_names_subject(rule) {
        return false;
    }
    super::policy::rule_matches_string(rule, tool_name, subject)
}

fn rule_names_subject(rule: &str) -> bool {
    let (r, ok) = parse_permrule(rule);
    ok && !r.subject.trim().is_empty()
}

/// Reports an install_source call that only produces a plan. The tool is a
/// writer as a whole, but its preview phase writes nothing. An uninstall is
/// not a preview: it removes what is already installed, and it is the one op
/// with nothing to read first.
pub fn install_source_is_plan_only(args: &serde_json::value::RawValue) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(args.get()) else {
        return false;
    };
    let apply = v.get("apply").and_then(|a| a.as_bool()).unwrap_or(false);
    if apply {
        return false;
    }
    match v.get("op").and_then(|o| o.as_str()) {
        None => true,
        Some(op) => op == "install",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::rule_matches_string;
    use serde_json::value::RawValue;

    fn raw(s: &str) -> Box<RawValue> {
        RawValue::from_string(s.to_string()).unwrap()
    }

    #[test]
    fn subject_scoped_grants_name_their_subject() {
        let granted = "/ws/internal/foo/bar.go";
        let rule = super::session_grant_rule(EXTEND_WRITE_PATHS, granted);
        assert!(rule_matches_string(&rule, EXTEND_WRITE_PATHS, granted));
        for other in [
            "/home/user/.ssh/authorized_keys",
            "/ws/.git/hooks/pre-commit",
            "/ws/secrets/key.pem",
        ] {
            assert!(
                !rule_matches_string(&rule, EXTEND_WRITE_PATHS, other),
                "rule {rule:?} also covers {other:?}"
            );
        }
    }

    #[test]
    fn install_plan_grants_name_the_ticket() {
        let rule = session_grant_rule_for_probe(INSTALL_SOURCE_TOOL, "low:sha256:abc");
        assert!(rule_matches_string(
            &rule,
            INSTALL_SOURCE_TOOL,
            "low:sha256:abc"
        ));
        assert!(!rule_matches_string(
            &rule,
            INSTALL_SOURCE_TOOL,
            "high:sha256:def"
        ));
        assert!(session_grant_matches(
            &rule,
            INSTALL_SOURCE_TOOL,
            "low:sha256:abc"
        ));
        assert!(!session_grant_matches(
            &rule,
            INSTALL_SOURCE_TOOL,
            "high:sha256:def"
        ));
    }

    fn session_grant_rule_for_probe(tool: &str, subject: &str) -> String {
        super::super::policy::session_grant_rule_for_scope(tool, subject)
    }

    #[test]
    fn egress_and_signin_grants_name_the_host() {
        let rule = session_grant_rule_for_probe(NETWORK_EGRESS, "pypi.org");
        assert!(session_grant_matches(&rule, NETWORK_EGRESS, "pypi.org"));
        assert!(!session_grant_matches(
            &rule,
            NETWORK_EGRESS,
            "evil.example"
        ));
        let rule = session_grant_rule_for_probe(MCP_SIGNIN, "figma → auth.example.test");
        assert!(session_grant_matches(
            &rule,
            MCP_SIGNIN,
            "figma → auth.example.test"
        ));
        assert!(!session_grant_matches(
            &rule,
            MCP_SIGNIN,
            "figma → evil.example"
        ));
        assert!(!session_grant_matches(
            &rule,
            MCP_SIGNIN,
            "other → auth.example.test"
        ));
        let rule = session_grant_rule_for_probe(PAID_SEARCH, "deepseek_api");
        assert!(session_grant_matches(&rule, PAID_SEARCH, "deepseek_api"));
        assert!(!session_grant_matches(
            &rule,
            PAID_SEARCH,
            "other_paid_backend"
        ));
    }

    #[test]
    fn every_subject_sensitive_tool_actually_asks_something() {
        let probes = [
            (EXTEND_WRITE_PATHS, vec!["/ws/any/path"]),
            (INSTALL_SOURCE_TOOL, vec!["high:sha256:abc"]),
            (NETWORK_EGRESS, vec!["pypi.org"]),
            (PAID_SEARCH, vec!["deepseek_api"]),
            (MCP_SIGNIN, vec!["figma → auth.example.test"]),
        ];
        for (tool, subjects) in probes {
            assert!(
                subjects.iter().any(|s| subject_requires_human(tool, s)),
                "{tool} never requires a person"
            );
            assert!(
                subject_scoped_grant(tool),
                "{tool} grants by bare tool name"
            );
        }
    }

    #[test]
    fn bare_grants_no_longer_answer_for_every_subject() {
        for tool in [EXTEND_WRITE_PATHS, INSTALL_SOURCE_TOOL] {
            for subject in ["low:sha256:abc", "high:sha256:def", "/ws/secrets/key.pem"] {
                assert!(
                    !session_grant_matches(tool, tool, subject),
                    "stale grant {tool:?} still covers {subject:?}"
                );
            }
        }
    }

    #[test]
    fn bare_grant_still_works_for_ordinary_tools() {
        assert!(session_grant_matches(
            "read_file",
            "read_file",
            "/ws/anything.go"
        ));
    }

    #[test]
    fn install_source_plan_only() {
        assert!(install_source_is_plan_only(&raw(r#"{"source":"x"}"#)));
        assert!(install_source_is_plan_only(&raw(
            r#"{"source":"x","op":"install"}"#
        )));
        assert!(!install_source_is_plan_only(&raw(
            r#"{"source":"x","apply":true}"#
        )));
        assert!(!install_source_is_plan_only(&raw(
            r#"{"source":"x","op":"uninstall"}"#
        )));
    }
}
