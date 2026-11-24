//! Shared line-level TOML lexer for the document index (CONFIG-023) and the
//! text surgery primitives (CONFIG-028).
//!
//! This mirrors `edit.go:1554-1830` / `tomldoc.go:175-203` byte for byte: it
//! tracks only the string state needed to tell a structural line apart from a
//! line inside a multi-line string, and to find the inline comment. It does
//! not parse values; bracket depth tracking lives where each layer needs it
//! (the index layer counts braces too, the surgery layer counts only `[]`).

use std::fmt::Write as _;

/// Which quote kind a scanner is currently inside (`edit.go:1559-1567`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TomlStringKind {
    #[default]
    None,
    Basic,
    Literal,
    MultilineBasic,
    MultilineLiteral,
}

/// Mutable scanner state carried across lines so a multi-line string keeps
/// its kind until its closing quote run.
#[derive(Debug, Clone, Default)]
pub struct TomlLexState {
    kind: TomlStringKind,
    escaped: bool,
}

impl TomlLexState {
    /// Whether the scanner is inside a multi-line string (`edit.go:1569-1571`).
    pub fn in_multiline_string(&self) -> bool {
        matches!(
            self.kind,
            TomlStringKind::MultilineBasic | TomlStringKind::MultilineLiteral
        )
    }
}

/// Counts the consecutive `quote` bytes at `start` (`edit.go:1664-1670`).
fn quote_run(line: &[u8], start: usize, quote: u8) -> usize {
    let mut end = start;
    while end < line.len() && line[end] == quote {
        end += 1;
    }
    end - start
}

/// Scans one line, returning the byte offset of the first `#` outside a
/// string, or `-1` when the whole line is structural (`edit.go:1573-1662`).
/// `outside_string` receives every byte scanned outside a string, so callers
/// can track bracket depth without re-lexing.
pub fn scan_toml_line(
    line: &str,
    state: &mut TomlLexState,
    mut outside_string: Option<&mut dyn FnMut(u8)>,
) -> isize {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        match state.kind {
            TomlStringKind::Basic => {
                if state.escaped {
                    state.escaped = false;
                    i += 1;
                    continue;
                }
                match ch {
                    b'\\' => state.escaped = true,
                    b'"' => state.kind = TomlStringKind::None,
                    _ => {}
                }
                i += 1;
                continue;
            }
            TomlStringKind::Literal => {
                if ch == b'\'' {
                    state.kind = TomlStringKind::None;
                }
                i += 1;
                continue;
            }
            TomlStringKind::MultilineBasic => {
                if state.escaped {
                    state.escaped = false;
                    i += 1;
                    continue;
                }
                match ch {
                    b'\\' => {
                        state.escaped = true;
                        i += 1;
                        continue;
                    }
                    b'"' => {
                        let run = quote_run(bytes, i, b'"');
                        if run >= 3 {
                            state.kind = TomlStringKind::None;
                        }
                        i += run;
                        continue;
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }
            TomlStringKind::MultilineLiteral => {
                if ch == b'\'' {
                    let run = quote_run(bytes, i, b'\'');
                    if run >= 3 {
                        state.kind = TomlStringKind::None;
                    }
                    i += run;
                    continue;
                }
                i += 1;
                continue;
            }
            TomlStringKind::None => {}
        }

        match ch {
            b'#' => return i as isize,
            b'"' => {
                let run = quote_run(bytes, i, b'"');
                match run {
                    1 => state.kind = TomlStringKind::Basic,
                    3..=5 => state.kind = TomlStringKind::MultilineBasic,
                    _ => {}
                }
                i += run;
            }
            b'\'' => {
                let run = quote_run(bytes, i, b'\'');
                match run {
                    1 => state.kind = TomlStringKind::Literal,
                    3..=5 => state.kind = TomlStringKind::MultilineLiteral,
                    _ => {}
                }
                i += run;
            }
            _ => {
                if let Some(outside) = outside_string.as_mut() {
                    outside(ch);
                }
                i += 1;
            }
        }
    }
    -1
}

/// One line of the source, with its byte span and original terminator
/// (`edit.go:1810-1830`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlLineSpan {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Splits body on `\n`; a final line without a terminator is still a span.
/// `text` keeps the line terminator.
pub fn toml_line_spans(body: &str) -> Vec<TomlLineSpan> {
    if body.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let bytes = body.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        let mut end = bytes.len();
        if let Some(rel) = body[start..].find('\n') {
            end = start + rel + 1;
        }
        spans.push(TomlLineSpan {
            start,
            end,
            text: body[start..end].to_owned(),
        });
        start = end;
    }
    spans
}

/// Strips the inline comment, returning the value-side prefix (`tomldoc.go:175-181`).
pub fn toml_strip_comment(line: &str) -> &str {
    let mut state = TomlLexState::default();
    match scan_toml_line(line, &mut state, None) {
        i if i >= 0 => &line[..i as usize],
        _ => line,
    }
}

/// Returns the inline comment including its leading `#`, `\r\n` trimmed
/// (`edit.go:1714-1720`); empty when the line has no outside-string comment.
pub fn toml_inline_comment(line: &str) -> String {
    let mut state = TomlLexState::default();
    match scan_toml_line(line, &mut state, None) {
        i if i >= 0 => line[i as usize..]
            .trim_end_matches(&['\r', '\n'][..])
            .to_owned(),
        _ => String::new(),
    }
}

/// Marks spans that are structural: a line inside an open multi-line string
/// is not a header or assignment (`edit.go:1672-1680`).
pub fn toml_structural_line_mask(spans: &[TomlLineSpan]) -> Vec<bool> {
    let mut structural = vec![false; spans.len()];
    let mut state = TomlLexState::default();
    for (i, span) in spans.iter().enumerate() {
        structural[i] = !state.in_multiline_string();
        scan_toml_line(&span.text, &mut state, None);
    }
    structural
}

/// Locates the last line of the value on `start`. A value whose trimmed head
/// is not `[` ends on its own line. Arrays track `[`/`]` depth only — this
/// layer does not count braces (`edit.go:1682-1712`, surgery-layer deviation).
pub fn toml_value_end_span(spans: &[TomlLineSpan], start: usize) -> usize {
    if start >= spans.len() {
        return start;
    }
    let (_, value, ok) = toml_key_value(&spans[start].text);
    if !ok || !value.trim_start().starts_with('[') {
        return start;
    }
    let mut depth = 0;
    let mut seen_array = false;
    let mut state = TomlLexState::default();
    for (i, span) in spans.iter().enumerate().skip(start) {
        let mut closed = false;
        scan_toml_line(
            &span.text,
            &mut state,
            Some(&mut |ch: u8| match ch {
                b'[' => {
                    seen_array = true;
                    depth += 1;
                }
                b']' if seen_array => {
                    depth -= 1;
                    if depth == 0 {
                        closed = true;
                    }
                }
                _ => {}
            }),
        );
        if closed {
            return i;
        }
    }
    start
}

/// Collects comment lines across `start..=end`, preserving their original
/// indentation (`edit.go:1722-1739`).
pub fn toml_comments_in_spans(spans: &[TomlLineSpan], start: usize, end: usize) -> Vec<String> {
    let mut state = TomlLexState::default();
    let mut comments = Vec::new();
    for span in spans.iter().take(end + 1).skip(start) {
        let comment_at = scan_toml_line(&span.text, &mut state, None);
        if comment_at < 0 {
            continue;
        }
        let mut indent_end = 0;
        let bytes = span.text.as_bytes();
        while indent_end < bytes.len() && (bytes[indent_end] == b' ' || bytes[indent_end] == b'\t')
        {
            indent_end += 1;
        }
        let comment = span.text[comment_at as usize..].trim_end_matches(&['\r', '\n'][..]);
        comments.push(format!("{}{}", &span.text[..indent_end], comment));
    }
    comments
}

/// Finds the `=` that ends a quoted-or-bare key, outside any quoted key. A
/// `#` before it means the line is a comment (`tomldoc.go:184-203`).
pub fn toml_assignment_index(line: &str) -> isize {
    let bytes = line.as_bytes();
    let mut quote = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if quote != 0 {
            if ch == b'\\' && quote == b'"' {
                i += 1;
            } else if ch == quote {
                quote = 0;
            }
        } else {
            match ch {
                b'"' | b'\'' => quote = ch,
                b'=' => return i as isize,
                b'#' => return -1,
                _ => {}
            }
        }
        i += 1;
    }
    -1
}

/// Splits a possibly dotted, possibly quoted key into parts by decoding a
/// `probe = 0` document (`tomldoc.go:206-225`). `None` when the probe fails.
pub fn split_toml_key(raw: &str) -> Option<Vec<String>> {
    let probe = format!("{} = 0", raw.trim());
    let value: toml::Value = toml::from_str(&probe).ok()?;
    let mut segs = Vec::new();
    let mut current = Some(value);
    while let Some(toml::Value::Table(table)) = current {
        if table.len() != 1 {
            return None;
        }
        let (key, next) = table.into_iter().next()?;
        segs.push(key);
        match next {
            toml::Value::Table(_) => current = Some(next),
            _ => break,
        }
    }
    (!segs.is_empty()).then_some(segs)
}

/// Parses a section header. Returns `(name, is_array, ok)` (`edit.go:1832-1849`).
pub fn toml_edit_section_header(line: &str) -> (String, bool, bool) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return (String::new(), false, false);
    }
    let trimmed = match trimmed.split_once('#') {
        Some((before, _)) => before.trim(),
        None => trimmed,
    };
    if let Some(inner) = trimmed
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
    {
        let name = inner.trim();
        return (name.to_owned(), true, !name.is_empty());
    }
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let name = inner.trim();
        return (name.to_owned(), false, !name.is_empty());
    }
    (String::new(), false, false)
}

/// Parses a single assignment line: `(key, value, ok)`. Dotted keys, comments
/// and malformed lines are rejected (`edit.go:1871-1888`).
pub fn toml_key_value(line: &str) -> (String, String, bool) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return (String::new(), String::new(), false);
    }
    let trimmed = match trimmed.split_once('#') {
        Some((before, _)) => before.trim(),
        None => trimmed,
    };
    let Some((key, value)) = trimmed.split_once('=') else {
        return (String::new(), String::new(), false);
    };
    let key = key.trim();
    if key.is_empty() || key.contains('.') {
        return (String::new(), String::new(), false);
    }
    (key.to_owned(), value.trim().to_owned(), true)
}

/// The simple key on a top-level assignment line (`edit.go:1866-1869`).
pub fn toml_top_level_key(line: &str) -> (String, bool) {
    let (key, _, ok) = toml_key_value(line);
    (key, ok)
}

/// Bare keys are `[_-a-zA-Z0-9]`; anything else quotes (`render.go:822-833`).
pub fn is_bare_toml_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|r| {
            r.is_ascii_lowercase()
                || r.is_ascii_uppercase()
                || r.is_ascii_digit()
                || r == '_'
                || r == '-'
        })
}

/// A `strconv.Quote`-style quoted TOML basic string: double-quoted, with
/// backslashes and quotes escaped.
pub fn quote_toml_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Renders one key part bare when possible, else quoted (`render.go:807-812`).
pub fn render_toml_key_part(key: &str) -> String {
    if is_bare_toml_key(key) {
        key.to_owned()
    } else {
        quote_toml_string(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_spans_keep_terminators_and_trailing_line() {
        let spans = toml_line_spans("a = 1\nb = 2\n");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "a = 1\n");
        assert_eq!(spans[1].text, "b = 2\n");
        let spans = toml_line_spans("a = 1\nb = 2");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].text, "b = 2");
        assert!(toml_line_spans("").is_empty());
    }

    #[test]
    fn comments_outside_strings_are_found_inside_kept() {
        let mut state = TomlLexState::default();
        assert_eq!(
            scan_toml_line(r#"k = "v # not a comment""#, &mut state, None),
            -1
        );
        let mut state = TomlLexState::default();
        assert_eq!(scan_toml_line("k = 1   # trailing", &mut state, None), 8);
        assert_eq!(toml_strip_comment("k = 1   # trailing"), "k = 1   ");
        assert_eq!(toml_inline_comment("k = 1   # trailing"), "# trailing");
    }

    #[test]
    fn multiline_string_lines_are_not_structural() {
        let body = "a = \"\"\"\nline one\nline two\n\"\"\"\nb = 2\n";
        let spans = toml_line_spans(body);
        let mask = toml_structural_line_mask(&spans);
        assert_eq!(mask, vec![true, false, false, false, true]);
    }

    #[test]
    fn value_end_span_follows_brackets_only_not_braces() {
        let body = r#"items = ["a", "b"]
next = 1
"#;
        let spans = toml_line_spans(body);
        assert_eq!(toml_value_end_span(&spans, 0), 0);
        let body = "items = [\n  \"a\",\n  \"b\"\n]\nnext = 1\n";
        let spans = toml_line_spans(body);
        assert_eq!(toml_value_end_span(&spans, 0), 3);
    }

    #[test]
    fn split_key_handles_bare_quoted_and_dotted() {
        assert_eq!(split_toml_key("name"), Some(vec!["name".to_owned()]));
        assert_eq!(
            split_toml_key("agent.subagent_models"),
            Some(vec!["agent".to_owned(), "subagent_models".to_owned()])
        );
        assert_eq!(
            split_toml_key(r#""quoted.key""#),
            Some(vec!["quoted.key".to_owned()])
        );
        assert!(split_toml_key("").is_none());
    }

    #[test]
    fn headers_parse_array_and_plain_tables() {
        assert_eq!(
            toml_edit_section_header("[[providers]]"),
            ("providers".to_owned(), true, true)
        );
        assert_eq!(
            toml_edit_section_header("[agent.persona]"),
            ("agent.persona".to_owned(), false, true)
        );
        let (_, _, ok) = toml_edit_section_header("  # comment header\n");
        assert!(!ok);
        let (_, _, ok) = toml_edit_section_header("[ ]");
        assert!(!ok);
    }

    #[test]
    fn key_value_rejects_dotted_and_comment_lines() {
        let (key, value, ok) = toml_key_value("name = \"x\"");
        assert!(ok);
        assert_eq!(key, "name");
        assert_eq!(value, "\"x\"");
        let (_, _, ok) = toml_key_value("agent.name = \"x\"");
        assert!(!ok);
        let (_, _, ok) = toml_key_value("# name = 1");
        assert!(!ok);
    }

    #[test]
    fn comments_before_value_keep_indentation() {
        let body = "[s]\n  # one\n  # two\nk = 1\n";
        let spans = toml_line_spans(body);
        let comments = toml_comments_in_spans(&spans, 1, 2);
        assert_eq!(comments, vec!["  # one", "  # two"]);
    }

    #[test]
    fn assignment_index_skips_quoted_keys_and_refutes_comment() {
        assert_eq!(toml_assignment_index(r#""weird = key" = 1"#), 14);
        assert_eq!(toml_assignment_index("# pure comment"), -1);
        assert_eq!(toml_assignment_index("no equals"), -1);
    }
}
