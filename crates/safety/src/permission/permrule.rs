//! The syntax of a permission rule: the form a rule is written in and stored
//! under in configuration. What a rule decides is [`super::policy`]'s; this
//! only says what a rule is.

/// A parsed permission rule. `subject` non-empty constrains the call's
/// subject; a glob subject matches by wildcard, a literal subject by exact
/// equality. Empty subject matches every call to the tool.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rule {
    pub tool: String,
    pub subject: String,
    /// Matches `subject` by exact equality rather than as a glob, so a
    /// remembered concrete command keeps any `*`/`?` as ordinary characters.
    pub literal: bool,
}

/// Parses `"ToolName"`, `"ToolName(glob)"`, or the legacy `"ToolName=literal"`
/// form, trimming surrounding whitespace. The `=literal` form (taken when the
/// `=` precedes any `(`) matches the rest verbatim. Returns `false` for a
/// malformed entry (empty tool name), so a caller can warn rather than
/// install a rule that matches nothing.
pub fn parse(s: &str) -> (Rule, bool) {
    let s = s.trim();
    if s.is_empty() {
        return (Rule::default(), false);
    }
    if let Some(eq) = s.find('=')
        && eq > 0
        && s.find('(').is_none_or(|p| eq < p)
    {
        let tool = s[..eq].trim();
        if tool.is_empty() {
            return (Rule::default(), false);
        }
        return (
            Rule {
                tool: tool.to_string(),
                subject: s[eq + 1..].to_string(),
                literal: true,
            },
            true,
        );
    }
    if let Some(i) = s.find('(')
        && s.ends_with(')')
    {
        let tool = s[..i].trim();
        if tool.is_empty() {
            return (Rule::default(), false);
        }
        return (
            Rule {
                tool: tool.to_string(),
                subject: s[i + 1..s.len() - 1].to_string(),
                literal: false,
            },
            true,
        );
    }
    (
        Rule {
            tool: s.to_string(),
            subject: String::new(),
            literal: false,
        },
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rule_forms() {
        let cases: &[(&str, &str, &str, bool, bool)] = &[
            ("bash", "bash", "", false, true),
            ("Bash(npm run build)", "Bash", "npm run build", false, true),
            ("Edit(docs/**)", "Edit", "docs/**", false, true),
            ("bash(rm -rf*)", "bash", "rm -rf*", false, true),
            ("  read_file  ", "read_file", "", false, true),
            // subject preserved verbatim
            (
                "bash( go test ./... )",
                "bash",
                " go test ./... ",
                false,
                true,
            ),
            // first '(' wins, trailing ')'
            ("bash(echo (hi))", "bash", "echo (hi)", false, true),
            // literal: '*' is not a wildcard
            ("bash=rm *.log", "bash", "rm *.log", true, true),
            // split on first '=' only
            ("bash=make FOO=bar", "bash", "make FOO=bar", true, true),
            // '=' before '(' -> literal, parens kept
            ("bash=echo (hi)", "bash", "echo (hi)", true, true),
            // '(' before '=' -> still a glob
            ("bash(make FOO=*)", "bash", "make FOO=*", false, true),
            ("get-user", "get-user", "", false, true),
            ("Set-Content", "Set-Content", "", false, true),
            ("set-content", "set-content", "", false, true),
            ("git", "git", "", false, true),
            ("Get-CustomThing", "Get-CustomThing", "", false, true),
            ("", "", "", false, false),
            ("(noTool)", "", "", false, false),
        ];
        for (input, tool, subject, literal, ok) in cases {
            let (r, got) = parse(input);
            assert_eq!(got, *ok, "parse({input:?}) ok");
            if got {
                assert_eq!(r.tool, *tool, "parse({input:?}) tool");
                assert_eq!(r.subject, *subject, "parse({input:?}) subject");
                assert_eq!(r.literal, *literal, "parse({input:?}) literal");
            }
        }
    }
}
