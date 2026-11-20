//! Whether a rule can reach any tool at all.

use super::permrule::parse as parse_permrule;
use super::policy::{canonical_rule_tool, legacy_deny_cmdlet, rule_tool_matches};

/// The typed refusal for a rule naming no registered tool. The refusal
/// carries the list, the rule and the name, so a caller never reads the
/// sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("permission rule {rule:?} in {list} names no registered tool {tool:?}")]
pub struct UnknownToolError {
    pub list: String,
    pub rule: String,
    pub tool: String,
}

/// The set of tool names a rule's Tool is matched against.
/// Reports a name in a namespace a connection supplies after startup.
pub type OpenNamespace = Box<dyn Fn(&str) -> bool + Send + Sync>;

pub struct Vocabulary {
    pub names: Vec<String>,
    /// Reports a name in a namespace a connection supplies after startup. A
    /// rule there may match tools that are not connected yet, so it is not
    /// judged.
    pub open: Option<OpenNamespace>,
}

impl Vocabulary {
    /// Reports the rule when no tool in the vocabulary can ever match it, and
    /// None when it can, when it is in an open namespace, or when it is not a
    /// rule. It uses the matcher the gate runs, so the two cannot disagree.
    pub fn check(&self, list: &str, raw: &str) -> Result<(), UnknownToolError> {
        let (r, ok) = parse_permrule(raw);
        if !ok {
            return Ok(());
        }
        if list == "deny" && r.subject.is_empty() && legacy_deny_cmdlet(&r.tool).is_some() {
            return Ok(());
        }
        if let Some(open) = &self.open
            && open(&r.tool)
        {
            return Ok(());
        }
        for name in &self.names {
            if rule_tool_matches(&r.tool, name) {
                return Ok(());
            }
        }
        Err(UnknownToolError {
            list: list.to_string(),
            rule: raw.to_string(),
            tool: canonical_rule_tool(&r.tool),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vocabulary() -> Vocabulary {
        Vocabulary {
            names: [
                "bash",
                "read_file",
                "write_file",
                "edit_file",
                "multi_edit",
                "grep",
                "web_fetch",
                "browser_open",
                "computer_act",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            open: Some(Box::new(|n: &str| n.starts_with("mcp__"))),
        }
    }

    #[test]
    fn check_names_the_rule_that_reaches_no_tool() {
        let v = test_vocabulary();
        let cases: &[(&str, &str, bool, &str)] = &[
            ("ask", "rm", true, "rm"),
            ("deny", "git reset", true, "git reset"),
            ("allow", "Bash(rm", true, "Bash(rm"),
            ("deny", "Read(*.env)", true, "Read"),
            ("ask", "bash", false, ""),
            ("ask", "Bash(rm:*)", false, ""),
            ("deny", "Edit(*.env*)", false, ""),
            ("deny", "file_mutation(*.env*)", false, ""),
            ("allow", "Browser=https://example.com", false, ""),
            ("ask", "mcp__github__create_issue", false, ""),
            ("deny", "Set-Content", false, ""),
            ("allow", "Set-Content", true, "Set-Content"),
            ("allow", "", false, ""),
        ];
        for (list, rule, unknown, tool) in cases {
            let err = v.check(list, rule);
            if !*unknown {
                assert!(err.is_ok(), "{list} {rule:?}: unexpected {err:?}");
                continue;
            }
            let err = err.unwrap_err();
            assert_eq!(err.list, *list, "{list} {rule:?}");
            assert_eq!(err.rule, *rule);
            assert_eq!(err.tool, *tool);
        }
    }

    #[test]
    fn dormant_rules_decide_nothing() {
        // A rule the vocabulary calls dormant must change no decision:
        // dropping it from the policy leaves every verdict as it was.
        let v = test_vocabulary();
        let mut lists: Vec<(&str, Vec<&str>)> = vec![
            ("allow", vec!["rm", "Bash(go test:*)", "git status", "Edit"]),
            (
                "ask",
                vec![
                    "rm",
                    "git reset",
                    "Bash(git push:*)",
                    "read_file(*.env*)",
                    "Read",
                ],
            ),
            (
                "deny",
                vec![
                    "rm",
                    "git reset",
                    "Set-Content",
                    "Bash(rm -rf:*)",
                    "file_mutation(*.env*)",
                    "Write",
                ],
            ),
        ];
        let calls: &[(&str, &str)] = &[
            ("bash", r#"{"command":"rm x"}"#),
            ("bash", r#"{"command":"rm -rf build"}"#),
            ("bash", r#"{"command":"git reset --hard"}"#),
            ("bash", r#"{"command":"git status"}"#),
            ("bash", r#"{"command":"go test ./..."}"#),
            ("bash", r#"{"command":"git push origin main"}"#),
            ("bash", r#"{"command":"Set-Content a.txt hi"}"#),
            ("bash", r#"{"command":"ls | wc -l"}"#),
            ("write_file", r#"{"path":".env","content":"x"}"#),
            ("write_file", r#"{"path":"a.go","content":"x"}"#),
            ("edit_file", r#"{"path":"a.go"}"#),
            ("read_file", r#"{"path":".env"}"#),
            ("read_file", r#"{"path":"a.go"}"#),
            ("grep", r#"{"pattern":"x"}"#),
            ("web_fetch", r#"{"url":"https://example.com"}"#),
        ];
        let mut dormant = 0;
        for (list, rules) in &mut lists {
            rules.retain(|r| {
                if v.check(list, r).is_err() {
                    dormant += 1;
                    false
                } else {
                    true
                }
            });
        }
        assert!(dormant > 0, "corpus has no dormant rule");
        let (allow, ask, deny) = (
            lists.iter().find(|(l, _)| *l == "allow").unwrap().1.clone(),
            lists.iter().find(|(l, _)| *l == "ask").unwrap().1.clone(),
            lists.iter().find(|(l, _)| *l == "deny").unwrap().1.clone(),
        );
        let full = super::super::policy::Policy::new("ask", &allow, &ask, &deny);
        let pruned = super::super::policy::Policy::new("ask", &allow, &ask, &deny);
        for (tool, args) in calls {
            let args = serde_json::value::RawValue::from_string(args.to_string()).unwrap();
            for ro in [true, false] {
                assert_eq!(
                    full.decide(tool, ro, &args),
                    pruned.decide(tool, ro, &args),
                    "{tool} {args} readOnly={ro}"
                );
            }
        }
    }
}
