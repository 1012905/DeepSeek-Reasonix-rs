//! The decision type a policy returns, and the subject/glob machinery the
//! rule matchers read.

use serde_json::Value;

/// The outcome of evaluating a tool call against a Policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decision {
    /// Runs the tool without prompting.
    #[default]
    Allow,
    /// Defers to an interactive Approver (or, with none, resolves to Allow).
    Ask,
    /// Blocks the tool in every mode.
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }
}

/// Parses a config string to a Decision. Unknown / empty input defaults to
/// Ask — the conservative posture for a writer fallback.
pub fn parse_decision(s: &str) -> Decision {
    match s.trim().to_ascii_lowercase().as_str() {
        "allow" => Decision::Allow,
        "deny" => Decision::Deny,
        _ => Decision::Ask,
    }
}

/// The JSON argument keys, in priority order, that carry a tool call's
/// "subject" — the thing a rule subject matches against. Generic so tools
/// need not implement a permission-specific method: bash exposes command, the
/// file tools expose path / file_path, grep & glob expose pattern. planId
/// leads because it is a ticket rather than a target — see
/// [`super::write_fence::subject_requires_human`].
const SUBJECT_KEYS: [&str; 9] = [
    "planId",
    "command",
    "file_path",
    "path",
    "source_path",
    "destination_path",
    "pattern",
    "origin",
    "app",
];

fn string_arg(m: &serde_json::Map<String, Value>, key: &str) -> String {
    match m.get(key) {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => String::new(),
    }
}

/// Extracts the primary matchable subject string from a call's raw JSON
/// args, returning "" when none of the known keys is present (such a call
/// only matches bare tool-name rules).
pub fn subject(args: &serde_json::value::RawValue) -> String {
    let mut subjects = subjects(args);
    if subjects.is_empty() {
        String::new()
    } else {
        subjects.remove(0)
    }
}

/// Extracts every matchable subject from a call's raw JSON args. Most tools
/// expose one subject; move_file exposes both source_path and
/// destination_path so path-scoped permission rules can protect either
/// endpoint.
pub fn subjects(args: &serde_json::value::RawValue) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(args.get()) else {
        return Vec::new();
    };
    let Some(m) = v.as_object() else {
        return Vec::new();
    };
    let src = string_arg(m, "source_path");
    let dst = string_arg(m, "destination_path");
    if !src.is_empty() && !dst.is_empty() {
        let mut out = vec![src];
        if dst != out[0] {
            out.push(dst);
        }
        return out;
    }
    let origin = string_arg(m, "origin");
    if super::browser::browser_subject_requires_explicit_approval(&origin) {
        return super::browser::browser_subjects(&origin);
    }
    let app = string_arg(m, "app");
    if super::computer::computer_subject_takes_pointer(&app) {
        return super::computer::computer_subjects(&app);
    }
    for k in SUBJECT_KEYS {
        let s = string_arg(m, k);
        if !s.is_empty() {
            return vec![s];
        }
    }
    let to = string_arg(m, "to");
    let text = string_arg(m, "text");
    if !to.is_empty() && !text.is_empty() {
        // A message to another session: who it goes to and every word of it,
        // so the person approving reads what would leave this session.
        return vec![format!("{to}: {text}")];
    }
    let project = string_arg(m, "project");
    if !project.is_empty() && !text.is_empty() {
        // A task handed to a session in another project: where it goes and
        // every word of it, for the same reason as a message.
        return vec![format!("{project}: {text}")];
    }
    let server = string_arg(m, "server");
    let uri = string_arg(m, "uri");
    if !server.is_empty() && !uri.is_empty() {
        // A resource read from an MCP server: the whole address, in the form
        // the chat's @server:uri reference uses, so it is judged as written.
        return vec![format!("{server}:{uri}")];
    }
    Vec::new()
}

/// Reports whether `name` matches `pattern`, where `*` matches any run of
/// characters (including separators) and `?` matches exactly one. Unlike
/// path globs, `*` is not stopped by `/`, which is what command-line and path
/// prefixes ("rm -rf*", "/etc/*") intuitively expect. Linear time with
/// backtracking, byte-oriented.
pub fn match_glob(pattern: &str, name: &str) -> bool {
    let (pb, nb) = (pattern.as_bytes(), name.as_bytes());
    let (mut px, mut nx) = (0usize, 0usize);
    let mut star_px: isize = -1;
    let mut star_nx: usize = 0;
    while nx < nb.len() {
        if px < pb.len() && pb[px] == b'*' {
            star_px = px as isize;
            star_nx = nx;
            px += 1;
        } else if px < pb.len() && (pb[px] == b'?' || pb[px] == nb[nx]) {
            px += 1;
            nx += 1;
        } else if star_px != -1 {
            px = (star_px + 1) as usize;
            star_nx += 1;
            nx = star_nx;
        } else {
            return false;
        }
    }
    while px < pb.len() && pb[px] == b'*' {
        px += 1;
    }
    px == pb.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::value::RawValue;

    fn raw(s: &str) -> Box<RawValue> {
        RawValue::from_string(s.to_string()).unwrap()
    }

    #[test]
    fn parse_decision_table() {
        assert_eq!(parse_decision("allow"), Decision::Allow);
        assert_eq!(parse_decision("ALLOW"), Decision::Allow);
        assert_eq!(parse_decision("  allow  "), Decision::Allow);
        assert_eq!(parse_decision("deny"), Decision::Deny);
        assert_eq!(parse_decision("DENY"), Decision::Deny);
        assert_eq!(parse_decision("ask"), Decision::Ask);
        assert_eq!(parse_decision("unknown"), Decision::Ask);
        assert_eq!(parse_decision(""), Decision::Ask);
        assert_eq!(parse_decision("  "), Decision::Ask);
    }

    #[test]
    fn decision_string() {
        assert_eq!(Decision::Allow.as_str(), "allow");
        assert_eq!(Decision::Ask.as_str(), "ask");
        assert_eq!(Decision::Deny.as_str(), "deny");
    }

    #[test]
    fn match_glob_table() {
        let cases: &[(&str, &str, bool)] = &[
            ("rm -rf*", "rm -rf /tmp/x", true), // '*' crosses '/'
            ("go test*", "go test ./...", true),
            ("rm *", "rm *.log", true),
            ("go test*", "go build", false),
            ("*", "anything at all", true),
            ("git ?ush", "git push", true),
            ("git ?ush", "git rush", true),
            ("git ?ush", "git pull", false),
            ("exact", "exact", true),
            ("exact", "exactly", false),
            ("a*c", "abbbc", true),
            ("a*c", "abbbd", false),
            ("*.go", "main.go", true),
            ("*.go", "main.rs", false),
            // empty pattern matches empty name (both consumed simultaneously)
            ("", "", true),
            ("", "anything", false),
            ("***", "anything", true),
            ("*", "", true),
            ("abcdefgh", "abc", false),
            ("a**c", "abc", true),
            ("?", "a", true),
            ("?", "", false),
            ("?", "ab", false),
        ];
        for (pattern, name, want) in cases {
            assert_eq!(
                match_glob(pattern, name),
                *want,
                "match_glob({pattern:?}, {name:?})"
            );
        }
    }

    #[test]
    fn subject_extraction() {
        let cases: &[(&str, &str)] = &[
            (r#"{"command":"go test ./..."}"#, "go test ./..."),
            (r#"{"file_path":"/a/b.go"}"#, "/a/b.go"),
            (r#"{"path":"/c/d"}"#, "/c/d"),
            // file_path/path beats pattern by key order
            (r#"{"pattern":"TODO","path":"/x"}"#, "/x"),
            (r#"{"other":"x"}"#, ""),
            (r#"{}"#, ""),
            // not-json input: RawValue requires valid JSON, so wrap the text
            // as a string value and confirm it yields no subject.
            (r#""not json""#, ""),
            // array values should not match
            (r#"{"command": ["array", "value"]}"#, ""),
            // null value should not match
            (r#"{"command": null}"#, ""),
            // empty command should not match
            (r#"{"command": ""}"#, ""),
            // command > file_path > path > pattern
            (
                r#"{"pattern":"pat","path":"/p","file_path":"/f","command":"cmd"}"#,
                "cmd",
            ),
        ];
        for (args, want) in cases {
            assert_eq!(subject(&raw(args)), *want, "subject({args:?})");
        }
    }

    #[test]
    fn subjects_move_file_checks_both_endpoints() {
        let got = subjects(&raw(
            r#"{"source_path":"tmp/a.md","destination_path":"secrets/a.md"}"#,
        ));
        assert_eq!(got, vec!["tmp/a.md", "secrets/a.md"]);
        assert_eq!(
            subject(&raw(
                r#"{"source_path":"tmp/a.md","destination_path":"secrets/a.md"}"#
            )),
            "tmp/a.md"
        );
    }

    #[test]
    fn message_and_resource_subjects() {
        let args = raw(r#"{"to":"docs/notes","text":"the rest of the words"}"#);
        assert_eq!(subject(&args), "docs/notes: the rest of the words");
        let only_to = raw(r#"{"to":"docs/notes"}"#);
        assert_eq!(subject(&only_to), "");
        let both = raw(r#"{"to":"alice@example.test","text":"hello"}"#);
        assert_eq!(subject(&both), "alice@example.test: hello");
        // non-string to, empty to, unrelated arguments
        assert_eq!(subject(&raw(r#"{"text":"hello"}"#)), "");
        assert_eq!(subject(&raw(r#"{"to":5,"text":"hello"}"#)), "");
        assert_eq!(subject(&raw(r#"{"to":"","text":"hello"}"#)), "");
        assert_eq!(subject(&raw(r#"{"recipient":"x","body":"y"}"#)), "");
        // a command wins over the pair
        assert_eq!(
            subject(&raw(r#"{"command":"ls","to":"a","text":"b"}"#)),
            "ls"
        );
        // resource read shows server and full address
        let pair = raw(r#"{"server":"docs","uri":"file:///etc/shadow?x=1#frag"}"#);
        assert_eq!(subject(&pair), "docs:file:///etc/shadow?x=1#frag");
        assert_eq!(subject(&raw(r#"{"server":"docs"}"#)), "");
        assert_eq!(subject(&raw(r#"{"uri":"doc://a"}"#)), "");
        assert_eq!(subject(&raw(r#"{"server":"a","uri":5}"#)), "");
        assert_eq!(subject(&raw(r#"{"server":1,"uri":"b"}"#)), "");
        assert_eq!(subject(&raw(r#"{"server":"","uri":"b"}"#)), "");
        assert_eq!(
            subject(&raw(r#"{"command":"ls","server":"a","uri":"b"}"#)),
            "ls"
        );
    }

    #[test]
    fn dispatch_subject_is_project_and_whole_text() {
        let args = raw(r#"{"project":"billing","text":"rotate the key"}"#);
        assert_eq!(subject(&args), "billing: rotate the key");
        assert_eq!(subject(&raw(r#"{"project":"billing"}"#)), "");
        assert_eq!(subject(&raw(r#"{"text":"hello"}"#)), "");
        assert_eq!(subject(&raw(r#"{"project":"","text":"x"}"#)), "");
        assert_eq!(subject(&raw(r#"{"project":1,"text":"x"}"#)), "");
        assert_eq!(
            subject(&raw(r#"{"command":"ls","project":"a","text":"b"}"#)),
            "ls"
        );
    }
}
