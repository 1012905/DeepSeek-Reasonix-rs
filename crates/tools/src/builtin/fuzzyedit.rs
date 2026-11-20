//! The narrow fuzzy fallback and the not-found / not-unique refusal texts
//! (encoding_helpers.go, second half). The exact-match engine lives in
//! [`super::editengine`]; this half answers only for the misses read_file
//! introduces or hides: trailing whitespace, tab-vs-spaces indentation,
//! read_file line prefixes, and a blank-line run of a different length.
//!
//! Content is held as bytes, the way Go strings are; see editengine.rs for
//! the boundary reasoning.

#![forbid(unsafe_code)]

use super::editengine::{
    EditRange, LineSeg, contains, line_has_newline, replace_bytes, split_line_segments, trim_left,
    trim_right,
};

/// encoding_helpers.go `fuzzyMode`.
#[derive(Debug, Clone, Copy)]
struct FuzzyMode {
    strip_old_read_prefixes: bool,
    trim_trailing: bool,
    expand_tabs: bool,
    trim_leading: bool,
}

/// encoding_helpers.go `fuzzyEditRanges`: the ordered mode ladder, then the
/// blank-run relaxation as a standalone last resort so it cannot compose
/// with tab expansion or prefix stripping.
pub fn fuzzy_edit_ranges(content: &[u8], old: &[u8]) -> Vec<EditRange> {
    if old.is_empty() || content.is_empty() {
        return Vec::new();
    }
    let content_lines = split_line_segments(content);
    let old_lines = split_line_segments(old);
    if old_lines.is_empty() {
        return Vec::new();
    }
    if old_lines.len() > content_lines.len() {
        // Equal-length window modes cannot match here, but a blank-line
        // run reproduced longer than the file's still can.
        return blank_run_ranges(&content_lines, &old_lines);
    }

    let old_has_read_prefixes = all_lines_have_read_file_prefix(&old_lines);
    let mut modes = vec![
        FuzzyMode {
            strip_old_read_prefixes: false,
            trim_trailing: true,
            expand_tabs: false,
            trim_leading: false,
        },
        FuzzyMode {
            strip_old_read_prefixes: false,
            trim_trailing: true,
            expand_tabs: true,
            trim_leading: false,
        },
    ];
    if old_has_read_prefixes {
        modes.push(FuzzyMode {
            strip_old_read_prefixes: true,
            trim_trailing: true,
            expand_tabs: false,
            trim_leading: false,
        });
        modes.push(FuzzyMode {
            strip_old_read_prefixes: true,
            trim_trailing: true,
            expand_tabs: true,
            trim_leading: false,
        });
    }

    for mode in &modes {
        let norm_old: Vec<Vec<u8>> = old_lines
            .iter()
            .map(|line| {
                normalize_fuzzy_line(
                    &line.raw,
                    line_has_newline(&line.raw),
                    *mode,
                    mode.strip_old_read_prefixes,
                )
            })
            .collect();
        let mut ranges = Vec::new();
        let mut i = 0usize;
        while i + old_lines.len() <= content_lines.len() {
            if fuzzy_window_matches(
                &content_lines[i..i + old_lines.len()],
                &old_lines,
                &norm_old,
                mode,
            ) {
                ranges.push(EditRange {
                    start: content_lines[i].start,
                    end: fuzzy_window_end(
                        &content_lines[i + old_lines.len() - 1],
                        &old_lines[old_lines.len() - 1],
                    ),
                });
                i += old_lines.len();
                continue;
            }
            i += 1;
        }
        if !ranges.is_empty() {
            return ranges;
        }
    }
    blank_run_ranges(&content_lines, &old_lines)
}

/// encoding_helpers.go `blankRunRanges`: collapses each run of consecutive
/// blank lines into one token, so only a run's length is free, never its
/// presence: a blank line the caller invented still never matches.
fn blank_run_ranges(content_lines: &[LineSeg], old_lines: &[LineSeg]) -> Vec<EditRange> {
    let c = blank_run_tokens(content_lines);
    let o = blank_run_tokens(old_lines);
    if o.is_empty() || o.len() > c.len() {
        return Vec::new();
    }
    // Only a run bounded by non-blank lines on both sides may drift in
    // length. A window that starts or ends blank has no anchoring text on
    // that side, so an all-blank old_string would otherwise name every
    // blank run in the file.
    if o[0].blank || o[o.len() - 1].blank {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i + o.len() <= c.len() {
        if blank_run_window_matches(&c[i..i + o.len()], &o) {
            let last = &c[i + o.len() - 1];
            ranges.push(EditRange {
                start: c[i].start,
                end: fuzzy_window_end(&last.last_line, &o[o.len() - 1].last_line),
            });
            i += o.len();
            continue;
        }
        i += 1;
    }
    ranges
}

fn blank_run_window_matches(window: &[BlankRunToken], old: &[BlankRunToken]) -> bool {
    for (i, tok) in window.iter().enumerate() {
        if tok.blank != old[i].blank || (!old[i].blank && tok.text != old[i].text) {
            return false;
        }
        // Same trailing-newline rule the other fuzzy modes apply: an
        // old_string that ends in a newline must not match a final line
        // that lacks one, or the write would append a newline the file
        // never had.
        if line_has_newline(&old[i].last_line.raw) && !line_has_newline(&tok.last_line.raw) {
            return false;
        }
    }
    true
}

/// encoding_helpers.go `blankRunToken`: one non-blank line, or one whole
/// run of consecutive blank lines collapsed into a single token.
#[derive(Debug, Clone)]
struct BlankRunToken {
    text: Vec<u8>,
    blank: bool,
    start: usize,
    last_line: LineSeg,
}

fn blank_run_tokens(lines: &[LineSeg]) -> Vec<BlankRunToken> {
    let mut out: Vec<BlankRunToken> = Vec::new();
    for line in lines {
        let body = line.raw.strip_suffix(b"\n").unwrap_or(&line.raw);
        if is_blank_body(body) {
            if let Some(last) = out.last_mut()
                && last.blank
            {
                last.last_line = line.clone();
                continue;
            }
            out.push(BlankRunToken {
                text: Vec::new(),
                blank: true,
                start: line.start,
                last_line: line.clone(),
            });
            continue;
        }
        out.push(BlankRunToken {
            text: trim_right(body, b" \t\r"),
            blank: false,
            start: line.start,
            last_line: line.clone(),
        });
    }
    out
}

/// Go `strings.TrimSpace(body) == ""` for the blankness decision: only
/// whitespace separates a blank line from content.
fn is_blank_body(body: &[u8]) -> bool {
    String::from_utf8_lossy(body).trim().is_empty()
}

fn fuzzy_window_matches(
    content_window: &[LineSeg],
    old_lines: &[LineSeg],
    norm_old: &[Vec<u8>],
    mode: &FuzzyMode,
) -> bool {
    for (i, content_line) in content_window.iter().enumerate() {
        let old_has_newline = line_has_newline(&old_lines[i].raw);
        if old_has_newline && !line_has_newline(&content_line.raw) {
            return false;
        }
        let got = normalize_fuzzy_line(&content_line.raw, old_has_newline, *mode, false);
        if got != norm_old[i] {
            return false;
        }
    }
    true
}

/// encoding_helpers.go `fuzzyWindowEnd`: an old_string that omits the
/// file's trailing newline must not consume it.
fn fuzzy_window_end(content_last: &LineSeg, old_last: &LineSeg) -> usize {
    if line_has_newline(&old_last.raw) || !line_has_newline(&content_last.raw) {
        return content_last.end;
    }
    let mut end = content_last.end - 1;
    if end > content_last.start && content_last.raw[content_last.raw.len() - 2] == b'\r' {
        end -= 1;
    }
    end
}

fn normalize_fuzzy_line(
    line: &[u8],
    include_newline: bool,
    mode: FuzzyMode,
    strip_read_prefix: bool,
) -> Vec<u8> {
    let mut body = line.strip_suffix(b"\n").unwrap_or(line).to_vec();
    if strip_read_prefix && let Some(stripped) = strip_read_file_line_prefix(&body) {
        body = stripped;
    }
    if mode.trim_trailing {
        body = trim_right(&body, b" \t\r");
    }
    if mode.expand_tabs {
        body = replace_bytes(&body, b"\t", b"    ", true).0;
    }
    if mode.trim_leading {
        body = trim_left(&body, b" \t");
    }
    if include_newline {
        body.push(b'\n');
    }
    body
}

fn all_lines_have_read_file_prefix(lines: &[LineSeg]) -> bool {
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| {
        strip_read_file_line_prefix(line.raw.strip_suffix(b"\n").unwrap_or(&line.raw)).is_some()
    })
}

/// encoding_helpers.go `stripReadFileLinePrefix`: optional leading blanks,
/// digits, then the read_file marker U+2192.
pub fn strip_read_file_line_prefix(line: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0usize;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    let mut j = i;
    while j < line.len() && line[j].is_ascii_digit() {
        j += 1;
    }
    if j == i || !line[j..].starts_with("→".as_bytes()) {
        return None;
    }
    Some(line[j + "→".len()..].to_vec())
}

/// encoding_helpers.go `nearestContentLine`: the first old line, normalized
/// the way the fuzzy ladder does, scored against every content line by
/// common prefix length.
fn nearest_content_line(old_string: &[u8], content: &[u8]) -> Option<(usize, String)> {
    let old_lines = split_line_segments(old_string);
    let first = old_lines.first()?;
    let normalized = normalize_fuzzy_line(
        &first.raw,
        false,
        FuzzyMode {
            strip_old_read_prefixes: false,
            trim_trailing: true,
            expand_tabs: true,
            trim_leading: false,
        },
        true,
    );
    let target = String::from_utf8_lossy(&normalized).trim().to_owned();
    if target.is_empty() {
        return None;
    }
    let mut best_line = 0usize;
    let mut best_score = 0usize;
    let mut best_text = String::new();
    for (i, line) in split_line_segments(content).iter().enumerate() {
        let text =
            String::from_utf8_lossy(line.raw.strip_suffix(b"\n").unwrap_or(&line.raw)).into_owned();
        let expanded = text.replace('\t', "    ");
        let score = common_prefix_len(expanded.trim(), &target);
        if score > best_score {
            best_line = i + 1;
            best_score = score;
            best_text = text;
        }
    }
    if best_score < 3 {
        return None;
    }
    Some((best_line, best_text))
}

/// encoding_helpers.go `oldStringNotFoundHint`.
fn old_string_not_found_hint(old_string: &[u8], content: &[u8]) -> String {
    let base = " Re-read the current file before retrying; if several related edits target the same area, combine the final replacements in one multi_edit call.";
    if !contains(content, b"\r\n") {
        return String::from(base);
    }
    let normalized_content = replace_bytes(content, b"\r\n", b"\n", true).0;
    let normalized_old = replace_bytes(old_string, b"\r\n", b"\n", true).0;
    if contains(&normalized_content, &normalized_old) {
        return String::from(
            " The target file uses CRLF line endings; edit_file/multi_edit normally normalize LF-only old_string for CRLF files, so this is likely stale context. Re-read the current file before retrying.",
        );
    }
    String::from(
        " The target file uses CRLF line endings, but edit_file/multi_edit already tolerate LF-only old_string for CRLF files; check for stale, incomplete, or non-unique context before retrying.",
    )
}

/// encoding_helpers.go `blankLinePresenceSpan`: the one span old_string
/// names once blank lines are dropped on both sides. The matcher never
/// accepts it, because a blank line's presence is not free, but it is the
/// cause the caller has to be told.
fn blank_line_presence_span(old_string: &[u8], content: &[u8]) -> Option<(usize, usize)> {
    let old = non_blank_lines(&split_line_segments(old_string));
    if old.len() < 2 {
        return None;
    }
    let lines = non_blank_lines(&split_line_segments(content));
    let mut matches = 0usize;
    let mut at = 0usize;
    for i in 0..=lines.len().saturating_sub(old.len()) {
        if lines[i..i + old.len()]
            .iter()
            .zip(old.iter())
            .all(|(a, b)| a.text == b.text)
        {
            matches += 1;
            at = i;
        }
    }
    if matches != 1 {
        return None;
    }
    let span = &lines[at..at + old.len()];
    for k in 0..old.len() {
        // Same layout means the blank lines agree and the miss lies
        // elsewhere.
        if span[k].line - span[0].line != old[k].line - old[0].line {
            return Some((span[0].line, span[span.len() - 1].line));
        }
    }
    None
}

/// encoding_helpers.go `numberedLine`.
#[derive(Debug, Clone)]
struct NumberedLine {
    line: usize,
    text: Vec<u8>,
}

fn non_blank_lines(lines: &[LineSeg]) -> Vec<NumberedLine> {
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let no_newline = line.raw.strip_suffix(b"\n").unwrap_or(&line.raw);
        let text = trim_right(no_newline, b" \t\r");
        if !is_blank_body(&text) {
            out.push(NumberedLine { line: i + 1, text });
        }
    }
    out
}

/// Go `%q` of a string, for the nearest-line refusal text.
fn go_quote(text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2);
    s.push('"');
    for c in text.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                s.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// encoding_helpers.go `oldStringNotFoundError`.
pub fn old_string_not_found_error(path: &str, old_string: &[u8], content: &[u8]) -> String {
    if let Some((first, last)) = blank_line_presence_span(old_string, content) {
        return format!(
            "old_string not found in {path}: lines {first}-{last} match it only when blank lines are ignored, so its blank lines differ from the file's there. Copy that span again with every blank line exactly as the file has it."
        );
    }
    let hint = old_string_not_found_hint(old_string, content);
    if let Some((line, text)) = nearest_content_line(old_string, content) {
        return format!(
            "old_string not found in {path} (nearest line {line}: {}).{hint}",
            go_quote(&text)
        );
    }
    format!("old_string not found in {path}.{hint}")
}

/// encoding_helpers.go `oldStringMatchLineSummary`: content lines holding
/// the first non-empty old line, capped.
fn old_string_match_line_summary(old_string: &[u8], content: &[u8], limit: usize) -> String {
    if limit == 0 {
        return String::new();
    }
    let target = first_non_empty_line(old_string);
    if target.is_empty() {
        return String::new();
    }
    let mut matches: Vec<usize> = Vec::new();
    for (i, line) in split_line_segments(content).iter().enumerate() {
        let text = String::from_utf8_lossy(line.raw.strip_suffix(b"\n").unwrap_or(&line.raw))
            .trim_end_matches('\r')
            .to_owned();
        if text.contains(&target) {
            matches.push(i + 1);
        }
    }
    if matches.is_empty() {
        return String::new();
    }
    let mut b = String::from("; matching lines include ");
    for (i, line) in matches.iter().enumerate() {
        if i >= limit {
            b.push_str(", ...");
            break;
        }
        if i > 0 {
            b.push_str(", ");
        }
        b.push_str(&line.to_string());
    }
    b
}

fn first_non_empty_line(s: &[u8]) -> String {
    for line in split_line_segments(s) {
        let text = String::from_utf8_lossy(line.raw.strip_suffix(b"\n").unwrap_or(&line.raw))
            .trim_end_matches('\r')
            .trim()
            .to_owned();
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    let n = a.len().min(b.len());
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    for i in 0..n {
        if ab[i] != bb[i] {
            return i;
        }
    }
    n
}

/// encoding_helpers.go `oldStringNotUniqueError`.
pub fn old_string_not_unique_error(
    path: &str,
    old_string: &[u8],
    content: &[u8],
    matches: usize,
    replace_all_hint: bool,
) -> String {
    let line_hint = old_string_match_line_summary(old_string, content, 5);
    if replace_all_hint {
        return format!(
            "old_string is not unique in {path} ({matches} matches){line_hint}; add nearby unique code, not just repeated separator lines, or set replace_all if every match should change"
        );
    }
    format!(
        "old_string is not unique in {path} ({matches} matches){line_hint}; add nearby unique code, not just repeated separator lines"
    )
}

/// multiedit.go `multiEditFailure`: a lone failure keeps its single-step
/// form. Several are listed together; steps after a failed one ran without
/// its replacement, so a later failure may only be a consequence of an
/// earlier one.
pub fn multi_edit_failure(path: &str, total: usize, failures: &[String]) -> Option<String> {
    match failures.len() {
        0 => None,
        1 => Some(failures[0].clone()),
        _ => Some(format!(
            "{} of {total} edits failed; {path} left untouched (each step after a failed one ran without its replacement):\n{}",
            failures.len(),
            failures.join("\n")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::editengine::apply_old_string_edit;

    fn apply(content: &str, old: &str) -> crate::builtin::editengine::EditApplyResult {
        apply_old_string_edit(content.as_bytes(), old.as_bytes(), b"REPLACED", false)
    }

    #[test]
    fn blank_run_cases() {
        // fuzzy_edit_test.go TestApplyOldStringEditBlankLineRunCases table.
        let cases: Vec<(&str, &str, bool)> = vec![
            (
                "def f():\n    return 1\n\n\ndef g():\n    return 2\n",
                "def f():\n    return 1\n\ndef g():\n    return 2",
                true,
            ),
            ("a = 1\n\nb = 2\n", "a = 1\n\n\nb = 2", true),
            (
                "def f():\n    return 1   \n\n\ndef g():\n    return 2\n",
                "def f():\n    return 1\n\ndef g():\n    return 2",
                true,
            ),
            (
                "def f():\n    return 1\n\n\ndef g():\n    return 2\n",
                "def f():\n    return 99\n\ndef g():\n    return 2",
                false,
            ),
            ("a = 1\nb = 2\n", "a = 1\n\nb = 2", false),
            ("a = 1\nb = 2\nc = 3\n", "a = 1\n\nc = 3", false),
            ("a\n\n\nb\n", "\n\n\n\n\n", false),
            ("a = 1\n\nb = 2\n", "\n\n\nb = 2", false),
            ("a = 1\n\n\nb = 2", "a = 1\n\nb = 2\n", false),
        ];
        for (content, old, want) in cases {
            let got = apply(content, old);
            assert_eq!(
                (got.applied == 1),
                want,
                "content={content:?} old={old:?} applied={} matches={}",
                got.applied,
                got.matches
            );
            if want {
                assert!(got.fuzzy, "a blank-run match must be disclosed as fuzzy");
            }
        }
    }

    #[test]
    fn not_found_error_prefers_blank_line_cause() {
        let msg = old_string_not_found_error(
            "src.ts",
            b"tex.vOffset = v;\n}\n/**",
            b"tex.uOffset = u;\r\ntex.vOffset = v;\r\n}\r\n\r\n/**\r\n * doc\r\n */\r\n",
        );
        assert!(msg.contains("lines 2-5"), "{msg}");
        assert!(msg.contains("blank lines"), "{msg}");
    }

    #[test]
    fn not_unique_error_names_matching_lines() {
        let separator = "    // \u{2550}".repeat(37);
        let body = format!("{separator}\nconst a = 1;\n{separator}\nconst b = 2;\n{separator}\n");
        let msg = old_string_not_unique_error(
            "map.html",
            separator.as_bytes(),
            body.as_bytes(),
            3,
            false,
        );
        for want in [
            "not unique",
            "matching lines include 1, 3, 5",
            "repeated separator lines",
        ] {
            assert!(msg.contains(want), "error should mention {want:?}: {msg}");
        }
    }

    #[test]
    fn multi_edit_failure_single_and_joined() {
        assert!(multi_edit_failure("a.txt", 3, &[]).is_none());
        assert_eq!(
            multi_edit_failure("a.txt", 3, &["edit 1: boom".to_owned()]).unwrap(),
            "edit 1: boom"
        );
        let joined = multi_edit_failure(
            "a.txt",
            3,
            &["edit 1: boom".to_owned(), "edit 2: bang".to_owned()],
        )
        .unwrap();
        assert!(joined.contains("2 of 3 edits failed"), "{joined}");
        assert!(joined.contains("edit 1: boom\nedit 2: bang"), "{joined}");
    }
}
