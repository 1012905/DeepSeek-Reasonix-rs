//! TOML text surgery primitives (CONFIG-028, config.md §15.4; `edit.go:1447-1939`).
//!
//! Every primitive works on plain strings and byte spans: no file IO, no
//! locking, no indentation reflow. The line ending is the source document's
//! own — this layer never converts `\n` to `\r\n` (that lives in the index
//! layer). Values delimited by arrays track `[`/`]` only; brace depth is not
//! counted here (surgery-layer deviation from the index layer).

use crate::toml_lex::{
    toml_comments_in_spans, toml_edit_section_header, toml_inline_comment, toml_key_value,
    toml_line_spans, toml_structural_line_mask, toml_top_level_key, toml_value_end_span,
};

fn trim_right_newlines(value: &str) -> &str {
    value.trim_end_matches(&['\n', '\r'][..])
}

/// Replaces a named section, header line included, with `new_content`. Array
/// and plain-table headers follow their own span rules; a missing section is
/// appended at the end (`edit.go:1447-1505`).
pub fn replace_toml_section(body: &str, section_name: &str, new_content: &str) -> String {
    let spans = toml_line_spans(body);
    let structural = toml_structural_line_mask(&spans);

    let mut array_idx: Option<usize> = None;
    for (i, span) in spans.iter().enumerate() {
        if !structural[i] {
            continue;
        }
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if ok && is_array && name == section_name {
            array_idx = Some(i);
            break;
        }
    }

    if let Some(array_idx) = array_idx {
        let start = spans[array_idx].start;
        let mut end = body.len();
        for next in spans.iter().skip(array_idx + 1) {
            let (name, is_array, ok) = toml_edit_section_header(&next.text);
            if !ok {
                continue;
            }
            if (is_array && name == section_name) || name.starts_with(&format!("{section_name}.")) {
                continue;
            }
            end = next.start;
            break;
        }
        return format!(
            "{}{}\n{}",
            &body[..start],
            trim_right_newlines(new_content),
            &body[end..]
        );
    }

    for (i, span) in spans.iter().enumerate() {
        if !structural[i] {
            continue;
        }
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if !ok || is_array || name != section_name {
            continue;
        }
        let mut end = body.len();
        for (next_idx, next) in spans.iter().enumerate() {
            if !structural[next_idx] {
                continue;
            }
            if next.start <= span.start {
                continue;
            }
            let (_, _, ok) = toml_edit_section_header(&next.text);
            if ok {
                end = next.start;
                break;
            }
        }
        return format!("{}{}{}", &body[..span.start], new_content, &body[end..]);
    }

    format!("{}\n\n{}", trim_right_newlines(body), new_content)
}

/// Inserts or replaces one key inside a plain table, preserving the leading
/// comment group of a multi-line value or the inline comment of a single-line
/// value (`edit.go:1507-1552`).
pub fn upsert_toml_section_key(body: &str, section_name: &str, key: &str, line: &str) -> String {
    let mut line = format!("{}\n", trim_right_newlines(line));
    let spans = toml_line_spans(body);
    let structural = toml_structural_line_mask(&spans);

    let mut section_idx: Option<usize> = None;
    let mut section_end = body.len();

    for (i, span) in spans.iter().enumerate() {
        if !structural[i] {
            continue;
        }
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if ok {
            if section_idx.is_some() {
                section_end = span.start;
                break;
            }
            if !is_array && name == section_name {
                section_idx = Some(i);
            }
            continue;
        }
        if section_idx.is_some() {
            let (got, _, ok) = toml_key_value(&span.text);
            if ok && got == key {
                let end_idx = toml_value_end_span(&spans, i);
                let end = spans[end_idx].end;
                if end_idx > i {
                    let comments = toml_comments_in_spans(&spans, i, end_idx);
                    if !comments.is_empty() {
                        line = format!("{}\n{line}", comments.join("\n"));
                    }
                } else {
                    let comment = toml_inline_comment(&spans[end_idx].text);
                    if !comment.is_empty() {
                        line = format!("{} {comment}\n", trim_right_newlines(&line));
                    }
                }
                return format!("{}{}{}", &body[..span.start], line, &body[end..]);
            }
        }
    }

    if section_idx.is_none() {
        let block = format!("[{section_name}]\n{line}");
        return replace_toml_section(body, section_name, &block);
    };

    let mut prefix = body[..section_end].to_owned();
    if !prefix.is_empty() && !prefix.ends_with('\n') {
        prefix.push('\n');
    }
    format!("{prefix}{line}{}", &body[section_end..])
}

/// Removes a named section and its child sections; an unknown section leaves
/// the body untouched (`edit.go:1741-1763`).
pub fn remove_toml_section(body: &str, section_name: &str) -> String {
    let spans = toml_line_spans(body);
    for (i, span) in spans.iter().enumerate() {
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if !ok || name != section_name {
            continue;
        }
        let mut end = body.len();
        for next in spans.iter().skip(i + 1) {
            let (next_name, next_is_array, ok) = toml_edit_section_header(&next.text);
            if !ok {
                continue;
            }
            if (is_array && next_is_array && next_name == section_name)
                || next_name.starts_with(&format!("{section_name}."))
            {
                continue;
            }
            end = next.start;
            break;
        }
        return format!(
            "{}\n{}",
            trim_right_newlines(&body[..span.start]),
            &body[end..]
        );
    }
    body.to_owned()
}

/// Removes one key from a plain table. When that leaves only blank/comment
/// lines, the whole table goes; otherwise only the key span drops
/// (`edit.go:1765-1808`).
pub fn remove_toml_section_key(body: &str, section_name: &str, key: &str) -> String {
    let spans = toml_line_spans(body);
    let mut section_idx: Option<usize> = None;
    let mut key_idx: Option<usize> = None;
    let mut end_idx = spans.len();

    for (i, span) in spans.iter().enumerate() {
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if ok {
            if section_idx.is_some() {
                end_idx = i;
                break;
            }
            if !is_array && name == section_name {
                section_idx = Some(i);
            }
            continue;
        }
        if section_idx.is_some() && key_idx.is_none() {
            let (got, _, ok) = toml_key_value(&span.text);
            if ok && got == key {
                key_idx = Some(i);
            }
        }
    }

    let (Some(section_idx), Some(key_idx)) = (section_idx, key_idx) else {
        return body.to_owned();
    };
    let key_end_idx = toml_value_end_span(&spans, key_idx);

    for i in (section_idx + 1)..end_idx {
        if i >= key_idx && i <= key_end_idx {
            continue;
        }
        let trimmed = spans[i].text.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return format!(
            "{}{}",
            &body[..spans[key_idx].start],
            &body[spans[key_end_idx].end..]
        );
    }

    let section_start = spans[section_idx].start;
    let section_end = if end_idx < spans.len() {
        spans[end_idx].start
    } else {
        body.len()
    };
    format!(
        "{}\n{}",
        trim_right_newlines(&body[..section_start]),
        &body[section_end..]
    )
}

/// Replaces a top-level field, inserting it before the first header when it
/// is absent (`edit.go:1851-1864`).
pub fn replace_toml_top_level_field(body: &str, key: &str, new_line: &str) -> String {
    let spans = toml_line_spans(body);
    let mut insert_at = body.len();
    for span in &spans {
        let (_, _, ok) = toml_edit_section_header(&span.text);
        if ok {
            insert_at = span.start;
            break;
        }
        let (got, ok) = toml_top_level_key(&span.text);
        if ok && got == key {
            return format!("{}{}{}", &body[..span.start], new_line, &body[span.end..]);
        }
    }
    format!("{}{}{}", &body[..insert_at], new_line, &body[insert_at..])
}

/// Whether a top-level (pre-first-header) key is present (`edit.go:1919-1929`).
pub fn toml_body_has_top_level_key(body: &str, key: &str) -> bool {
    for span in toml_line_spans(body) {
        let (_, _, ok) = toml_edit_section_header(&span.text);
        if ok {
            return false;
        }
        let (got, ok) = toml_top_level_key(&span.text);
        if ok && got == key {
            return true;
        }
    }
    false
}

/// Whether any header names the section (`edit.go:1931-1939`).
pub fn toml_body_has_section(body: &str, section_name: &str) -> bool {
    toml_line_spans(body).iter().any(|span| {
        let (name, _, ok) = toml_edit_section_header(&span.text);
        ok && name == section_name
    })
}

/// The trimmed value of a key inside a plain table (`edit.go:1890-1906`).
pub fn toml_section_key_value(body: &str, section_name: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for span in toml_line_spans(body) {
        let (name, is_array, ok) = toml_edit_section_header(&span.text);
        if ok {
            in_section = !is_array && name == section_name;
            continue;
        }
        if !in_section {
            continue;
        }
        let (got, value, ok) = toml_key_value(&span.text);
        if ok && got == key {
            return Some(value);
        }
    }
    None
}

/// Merges a clean delta into body: top-level fields first, then each named
/// section through `replace_toml_section`. Consecutive same-name array blocks
/// merge into one replacement (`edit.go:1268-1319`).
pub fn merge_toml_delta(body: &str, delta: &str) -> String {
    #[derive(Default)]
    struct Section {
        name: String,
        content: String,
        is_array: bool,
    }

    let mut top_level = String::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut cur_name = String::new();
    let mut cur_buf = String::new();
    let mut cur_is_array = false;

    let flush =
        |cur_name: &str, cur_buf: &mut String, cur_is_array: bool, sections: &mut Vec<Section>| {
            if cur_name.is_empty() {
                return;
            }
            let content = cur_buf.clone();
            if cur_is_array
                && let Some(last) = sections.last_mut()
                && last.is_array
                && last.name == cur_name
            {
                last.content.push_str(&content);
            } else {
                sections.push(Section {
                    name: cur_name.to_owned(),
                    content,
                    is_array: cur_is_array,
                });
            }
            cur_buf.clear();
        };

    for line in delta.split('\n') {
        let (name, is_array, ok) = toml_edit_section_header(line);
        if ok {
            flush(&cur_name, &mut cur_buf, cur_is_array, &mut sections);
            cur_name = name;
            cur_is_array = is_array;
            cur_buf.push_str(line);
            cur_buf.push('\n');
            continue;
        }
        if !cur_name.is_empty() {
            cur_buf.push_str(line);
            cur_buf.push('\n');
            continue;
        }
        if !line.trim().is_empty() {
            top_level.push_str(line);
            top_level.push('\n');
        }
    }
    flush(&cur_name, &mut cur_buf, cur_is_array, &mut sections);

    let mut body = body.to_owned();
    if !top_level.trim().is_empty() {
        body = merge_toml_top_level_fields(&body, &top_level);
    }
    for section in &sections {
        body = replace_toml_section(&body, &section.name, &section.content);
    }
    body
}

/// Applies top-level field lines one by one (`edit.go:1321-1334`).
pub fn merge_toml_top_level_fields(body: &str, fields: &str) -> String {
    let mut body = body.to_owned();
    for line in fields.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, ok) = toml_top_level_key(line);
        if !ok {
            continue;
        }
        body = replace_toml_top_level_field(&body, &key, &format!("{line}\n"));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_replace_collapses_new_content_to_one_trailing_newline() {
        let body = "[[plugins]]\nname = \"a\"\n[[plugins]]\nname = \"b\"\n";
        let out = replace_toml_section(body, "plugins", "[[plugins]]\nname = \"c\"\n\n\n");
        assert_eq!(out, "[[plugins]]\nname = \"c\"\n");
    }

    #[test]
    fn plain_table_replace_keeps_new_content_tail_newlines() {
        let body = "[ui]\ntheme = \"dark\"\n[other]\nk = 1\n";
        let out = replace_toml_section(body, "ui", "theme = \"light\"\n\n");
        assert_eq!(out, "theme = \"light\"\n\n[other]\nk = 1\n");
    }

    #[test]
    fn missing_section_is_appended_with_a_blank_line_separator() {
        let body = "[ui]\ntheme = \"dark\"\n";
        let out = replace_toml_section(body, "network", "proxy_url = \"http://x\"\n");
        assert_eq!(out, "[ui]\ntheme = \"dark\"\n\nproxy_url = \"http://x\"\n");
    }

    #[test]
    fn header_trailing_comment_is_stripped_not_migrated() {
        let body = "[ui]   # old comment\ntheme = \"dark\"\n";
        let out = replace_toml_section(body, "ui", "theme = \"light\"\n");
        assert!(!out.contains("old comment"));
        assert_eq!(out, "theme = \"light\"\n");
    }

    #[test]
    fn upsert_existing_single_line_keeps_inline_comment() {
        let body = "[ui]\ntheme = \"dark\"   # note\n";
        let out = upsert_toml_section_key(body, "ui", "theme", "theme = \"light\"");
        assert_eq!(out, "[ui]\ntheme = \"light\" # note\n");
    }

    #[test]
    fn upsert_multiline_value_keeps_leading_comment_group() {
        let body = "[s]\n# lead one\n# lead two\nitems = [\n  \"a\"\n]\n";
        let out = upsert_toml_section_key(body, "s", "items", "items = [\n  \"b\"\n]");
        assert!(out.starts_with("[s]\n# lead one\n# lead two\nitems = [\n  \"b\"\n]\n"));
    }

    #[test]
    fn upsert_missing_table_builds_a_header() {
        let body = "[ui]\ntheme = \"dark\"\n";
        let out = upsert_toml_section_key(body, "network", "proxy_url", "proxy_url = \"x\"");
        assert!(out.contains("[network]"));
        assert!(out.contains("proxy_url = \"x\"\n"));
    }

    #[test]
    fn remove_section_drops_child_prefixes_but_leaves_other_tables() {
        let body = "[agent]\nk = 1\n[agent.persona]\nprefix = \"x\"\n[ui]\nt = 1\n";
        let out = remove_toml_section(body, "agent");
        assert!(!out.contains("[agent]"));
        assert!(!out.contains("persona"));
        assert!(out.contains("[ui]"));
    }

    #[test]
    fn remove_unknown_section_returns_body_untouched() {
        let body = "[ui]\nt = 1\n";
        assert_eq!(remove_toml_section(body, "nope"), body);
    }

    #[test]
    fn remove_key_keeps_table_when_other_keys_remain() {
        let body = "[ui]\ntheme = \"dark\"\ncursor_shape = \"block\"\n";
        let out = remove_toml_section_key(body, "ui", "theme");
        assert!(out.contains("[ui]"));
        assert!(out.contains("cursor_shape"));
        assert!(!out.contains("theme"));
    }

    #[test]
    fn remove_key_takes_the_whole_table_when_only_comments_remain() {
        let body = "[ui]\ntheme = \"dark\"\n# keep me?\n[other]\nk = 1\n";
        let out = remove_toml_section_key(body, "ui", "theme");
        assert!(!out.contains("[ui]"));
        assert!(out.contains("[other]"));
    }

    #[test]
    fn remove_unknown_key_returns_body_untouched() {
        let body = "[ui]\ntheme = \"dark\"\n";
        assert_eq!(remove_toml_section_key(body, "ui", "nope"), body);
    }

    #[test]
    fn top_level_field_replacement_and_pre_header_insertion() {
        let body = "a = 1\n[ui]\nt = 1\n";
        let out = replace_toml_top_level_field(body, "a", "a = 2\n");
        assert_eq!(out, "a = 2\n[ui]\nt = 1\n");
        let out = replace_toml_top_level_field(body, "b", "b = 3\n");
        assert_eq!(out, "a = 1\nb = 3\n[ui]\nt = 1\n");
    }

    #[test]
    fn comments_and_brackets_inside_multiline_strings_are_not_structural() {
        let body = "x = \"\"\"\n# not a header\n[not-a-table]\n\"\"\"\ny = 1\n";
        let spans = toml_line_spans(body);
        let mask = toml_structural_line_mask(&spans);
        assert_eq!(mask, vec![true, false, false, false, true]);
        // The raw header probe itself does not know about multiline strings;
        // structural consumers above it gate on the mask instead.
        assert!(toml_body_has_section(body, "not-a-table"));
    }

    #[test]
    fn crlf_source_lines_are_preserved_without_conversion() {
        let body = "[ui]\r\ntheme = \"dark\"\r\n";
        let out = upsert_toml_section_key(body, "ui", "theme", "theme = \"light\"");
        // The untouched header keeps its CRLF; the replacement line keeps the
        // primitive's own LF ending (line-ending conversion is a later wip).
        assert_eq!(out, "[ui]\r\ntheme = \"light\"\n");
    }

    #[test]
    fn dotted_assignment_keys_are_rejected_by_key_value() {
        let body = "[ui]\nagent.name = \"x\"\n";
        let out = upsert_toml_section_key(body, "ui", "agent.name", "agent.name = \"y\"");
        // The dotted line does not match, so the new line appends at section end.
        assert!(out.contains("agent.name = \"y\""));
    }

    #[test]
    fn merge_delta_combines_consecutive_array_blocks() {
        let body = "[[plugins]]\nname = \"old\"\n";
        let delta = "[[plugins]]\nname = \"a\"\n[[plugins]]\nname = \"b\"\n";
        let out = merge_toml_delta(body, delta);
        assert!(out.contains("name = \"a\""));
        assert!(out.contains("name = \"b\""));
        assert!(!out.contains("old"));
    }

    #[test]
    fn merge_delta_applies_top_level_fields_before_sections() {
        let body = "[ui]\nt = 1\n";
        let delta = "default_model = \"m\"\n[ui]\nt = 2\n";
        let out = merge_toml_delta(body, delta);
        assert!(out.starts_with("default_model = \"m\"\n"));
        assert!(out.contains("t = 2"));
    }
}
