//! Whether a rule has a shape the host itself records for a session grant.

use super::policy::bash_command_prefix;

/// The tool-name spelling of a bare grant: one letter then letters, digits,
/// and `.:-_` separators, capped at 128 chars (Go: `bareGrantTool` regexp).
fn is_bare_grant_tool(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    let rest = chars.as_str();
    (1 + rest.chars().count()) <= 128
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// Reports whether rule has a shape the host itself records for a session
/// grant. A grant read back from disk is only as trusted as the file, so
/// anything the host could not have written — a wildcard command, a prefix
/// shorter or broader than [`bash_command_prefix`] derives — is dropped
/// instead of widening what the conversation may do.
pub fn valid_session_grant_rule(rule: &str) -> bool {
    if rule.is_empty()
        || rule.trim() != rule
        || rule.len() > 512
        || rule.contains(['\n', '\r', '\0'])
    {
        return false;
    }
    if let Some(rest) = rule.strip_prefix("Bash(") {
        let Some(inner) = rest.strip_suffix(')') else {
            return false;
        };
        let Some(cmd) = inner.strip_suffix(":*") else {
            return false;
        };
        return bash_command_prefix(cmd).is_some_and(|derived| derived == inner);
    }
    if let Some(exact) = rule.strip_prefix("Bash=") {
        return !exact.trim().is_empty();
    }
    if let Some((tool, subject)) = rule.split_once('=') {
        return is_bare_grant_tool(tool) && !subject.trim().is_empty();
    }
    is_bare_grant_tool(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_the_host_records_is_a_valid_grant() {
        let cases = [
            ("bash", "go test ./..."),
            ("bash", "npm run build"),
            ("bash", "rm -rf /tmp/x"),
            ("bash", "echo hi | wc -l"),
            ("edit_file", "a.go"),
            ("write_file", "b.go"),
            ("web_fetch", "https://example.com/a"),
            ("browser_open", "https://example.com"),
            ("computer_act", "Calculator"),
            ("mcp__files__read", ""),
        ];
        for (tool, subject) in cases {
            let rule = super::super::policy::session_grant_rule_for_scope(tool, subject);
            assert!(
                valid_session_grant_rule(&rule),
                "host rule {rule:?} for {tool}/{subject:?} was rejected"
            );
        }
    }

    #[test]
    fn rules_the_host_could_not_have_written_are_rejected() {
        for rule in [
            "",
            "   ",
            "*",
            "Bash(*)",
            "Bash(*:*)",
            "Bash(rm:*)",
            "Bash(go:*)",
            "Bash(rm -rf:*)",
            "Bash(go test:*",
            "Bash(go test)",
            "Bash(go test:*)\n",
            "Bash=",
            "bash*",
            "Edit(*)",
            "tool=",
            "=x",
            "a b",
            "Bash(sudo make:*)",
        ] {
            assert!(
                !valid_session_grant_rule(rule),
                "{rule:?} was accepted as a session grant"
            );
        }
    }
}
