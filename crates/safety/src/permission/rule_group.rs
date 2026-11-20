//! Tools that one rule name answers for. A rule group is a set of tools one
//! grant covers: approving an edit answers for every file writer, and
//! approving a site answers for loading, reading and operating it alike, and
//! approving an application answers for reading and operating that
//! application.

const FILE_MUTATION_GROUP: &str = "file_mutation";
const BROWSER_GROUP: &str = "browser";
const COMPUTER_GROUP: &str = "computer";

/// Reports whether a tool operates the agent's browser.
pub fn is_browser_tool(tool_name: &str) -> bool {
    group_of(tool_name) == BROWSER_GROUP
}

/// Reports whether a tool reads or operates another application.
pub fn is_computer_tool(tool_name: &str) -> bool {
    group_of(tool_name) == COMPUTER_GROUP
}

pub(crate) fn group_of(tool_name: &str) -> &'static str {
    match tool_name {
        "write_file" | "edit_file" | "multi_edit" | "move_file" | "notebook_edit"
        | "delete_range" | "delete_symbol" => FILE_MUTATION_GROUP,
        "browser_open" | "browser_read" | "browser_act" => BROWSER_GROUP,
        "computer_read" | "computer_act" => COMPUTER_GROUP,
        _ => "",
    }
}

/// Reports whether `tool_name` belongs to the group a canonical rule tool
/// names. A rule tool that names no group covers no tool through this.
pub(crate) fn in_group(rule_tool: &str, tool_name: &str) -> bool {
    !rule_tool.is_empty() && group_of(tool_name) == rule_tool
}

/// The grant a group records. File writers are granted as a whole, a site by
/// its origin and an application by its bundle id; a call that names neither
/// has no grant any later call could reuse.
pub(crate) fn group_grant_rule(tool_name: &str, subject: &str) -> Option<String> {
    match group_of(tool_name) {
        g if g == FILE_MUTATION_GROUP => Some("Edit".to_string()),
        g if g == BROWSER_GROUP && !subject.is_empty() => Some(format!("Browser={subject}")),
        g if g == COMPUTER_GROUP && !subject.is_empty() => Some(format!("Computer={subject}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_membership() {
        for tool in [
            "write_file",
            "edit_file",
            "multi_edit",
            "move_file",
            "notebook_edit",
            "delete_range",
            "delete_symbol",
        ] {
            assert!(in_group("file_mutation", tool));
            assert!(in_group("file_mutation", tool));
        }
        for tool in ["browser_open", "browser_read", "browser_act"] {
            assert!(is_browser_tool(tool));
        }
        for tool in ["computer_read", "computer_act"] {
            assert!(is_computer_tool(tool));
        }
        assert!(!in_group("Edit", "bash"));
        assert!(!in_group("bash", "write_file"));
        assert!(!is_browser_tool("web_fetch"));
    }

    #[test]
    fn group_grant_rules() {
        assert_eq!(
            group_grant_rule("edit_file", "a.go").as_deref(),
            Some("Edit")
        );
        assert_eq!(group_grant_rule("write_file", "").as_deref(), Some("Edit"));
        assert_eq!(
            group_grant_rule("browser_act", "https://github.com").as_deref(),
            Some("Browser=https://github.com")
        );
        assert_eq!(group_grant_rule("browser_act", ""), None);
        assert_eq!(
            group_grant_rule("computer_act", "com.apple.Notes").as_deref(),
            Some("Computer=com.apple.Notes")
        );
        assert_eq!(group_grant_rule("bash", "ls"), None);
    }
}
