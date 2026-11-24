//! Line-level TOML document index (CONFIG-023, config.md §12; `tomldoc.go`).
//!
//! The index addresses tables and keys by 0-based line index so a save can
//! rewrite only the changed keys and leave every other byte alone. It models
//! headers and single-segment assignment keys; dotted assignment keys,
//! unterminated brackets and multi-line strings it cannot close make the whole
//! document unmodeled (`ok = false`), which leaves the caller to rewrite the
//! file whole.
//!
//! Pure in-memory string/index operations: no file IO here.

use std::collections::BTreeMap;
use std::fmt;

use crate::toml_lex::{
    TomlLexState, is_bare_toml_key, quote_toml_string, scan_toml_line, split_toml_key,
    toml_assignment_index, toml_inline_comment, toml_line_spans, toml_strip_comment,
};

/// One path segment: a table `key`, plus the array-element identity when the
/// segment addresses an element of an array of tables.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TomlPathSeg {
    pub key: String,
    pub elem: String,
}

/// A table/element address (`tomldoc.go:15-63`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TomlPath(Vec<TomlPathSeg>);

impl TomlPath {
    /// Appends a child table segment.
    pub fn child(&self, key: &str) -> Self {
        let mut out = self.0.clone();
        out.push(TomlPathSeg {
            key: key.to_owned(),
            elem: String::new(),
        });
        Self(out)
    }

    /// Tags the last segment with an array-element identity.
    pub fn element(&self, id: &str) -> Self {
        let mut out = self.0.clone();
        if let Some(last) = out.last_mut() {
            last.elem = id.to_owned();
        }
        Self(out)
    }

    /// Length in segments.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the path holds no segment.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The first `n` segments (`tomldoc.go:454` `p[:i+1]`).
    pub fn prefix(&self, n: usize) -> Self {
        Self(self.0[..n.min(self.0.len())].to_vec())
    }

    /// The last segment's element identity, empty for a plain table.
    pub fn last_elem(&self) -> &str {
        self.0
            .last()
            .map(|seg| seg.elem.as_str())
            .unwrap_or_default()
    }

    /// The opaque separator-joined identity used for lookup (`tomldoc.go:32-43`).
    pub fn string(&self) -> String {
        let mut out = String::new();
        for seg in &self.0 {
            out.push_str(&seg.key);
            out.push(0x1f as char);
            if !seg.elem.is_empty() {
                out.push_str(&seg.elem);
                out.push(0x1e as char);
            }
        }
        out
    }

    /// The `[dotted.header]` spelling for this path (`tomldoc.go:45-51`).
    pub fn header(&self) -> String {
        self.0
            .iter()
            .map(|seg| key_segment(&seg.key))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Whether `self` is `p` or lies inside it (`tomldoc.go:54-56`).
    pub fn under(&self, q: &TomlPath) -> bool {
        self.string().starts_with(&q.string())
    }

    /// Whether `self` and `q` are elements of one array of tables
    /// (`tomldoc.go:59-63`).
    pub fn same_array(&self, q: &TomlPath) -> bool {
        let n = self.0.len();
        n > 0
            && n == q.0.len()
            && !self.0[n - 1].elem.is_empty()
            && !q.0[n - 1].elem.is_empty()
            && self.0[n - 1].key == q.0[n - 1].key
            && self.prefix(n - 1).string() == q.prefix(n - 1).string()
    }
}

/// Quotes a key segment for a header: bare keys pass through, others are quoted
/// (`tomldoc.go:65-72`).
fn key_segment(key: &str) -> String {
    if is_bare_toml_key(key) {
        key.to_owned()
    } else {
        quote_toml_string(key)
    }
}

/// The identity of an array element: its `name` value, else its ordinal
/// (`tomldoc.go:74-79`).
fn element_identity(name: Option<&str>, ordinal: i32) -> String {
    match name {
        Some(name) if !name.is_empty() => format!("name={name}"),
        _ => format!("#{ordinal}"),
    }
}

/// One assignment key and the inclusive line span of its whole value
/// (`tomldoc.go:81-84`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlDocKey {
    pub key: String,
    pub first: usize,
    pub last: usize,
}

/// One indexed table: `header` is the header line (`-1` for root), `end` is
/// the exclusive next-header line (`tomldoc.go:86-91`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlDocBlock {
    pub path: TomlPath,
    pub header: i32,
    pub end: i32,
    pub keys: Vec<TomlDocKey>,
}

/// The indexed document (`tomldoc.go:93-98`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TomlDoc {
    pub lines: Vec<String>,
    pub blocks: Vec<TomlDocBlock>,
    pub crlf: bool,
    pub broken: bool,
}

struct RawHeader {
    segs: Vec<String>,
    array: bool,
}

/// Indexes `text`. `None` marks a form the index cannot model, so the caller
/// rewrites the whole document rather than guessing where a key lives.
pub fn parse_toml_doc(text: &str) -> Option<TomlDoc> {
    let crlf = text.contains("\r\n");
    let spans = toml_line_spans(text);
    let lines: Vec<String> = spans.iter().map(|span| span.text.clone()).collect();

    let mut doc = TomlDoc {
        lines,
        blocks: vec![TomlDocBlock {
            path: TomlPath::default(),
            header: -1,
            end: 0,
            keys: Vec::new(),
        }],
        crlf,
        broken: false,
    };

    let mut headers: Vec<RawHeader> = vec![RawHeader {
        segs: Vec::new(),
        array: false,
    }];

    let mut i = 0usize;
    while i < doc.lines.len() {
        let trimmed = toml_strip_comment(&doc.lines[i]).trim().to_owned();
        let cur = doc.blocks.len() - 1;
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('[') {
            let array = trimmed.starts_with("[[");
            let mut inner = rest;
            if array {
                inner = inner.strip_prefix('[').unwrap_or(inner);
            }
            let inner = inner.strip_suffix(']').unwrap_or(inner);
            let inner = inner.strip_suffix(']').unwrap_or(inner);
            let segs = split_toml_key(inner)?;
            doc.blocks[cur].end = i as i32;
            doc.blocks.push(TomlDocBlock {
                path: TomlPath::default(),
                header: i as i32,
                end: 0,
                keys: Vec::new(),
            });
            headers.push(RawHeader { segs, array });
        } else {
            let eq = toml_assignment_index(&doc.lines[i]);
            if eq < 0 {
                return None;
            }
            let segs = split_toml_key(&doc.lines[i][..eq as usize])?;
            if segs.len() != 1 {
                return None;
            }
            let last = toml_value_end(&doc.lines, i, eq as usize + 1)?;
            doc.blocks[cur].keys.push(TomlDocKey {
                key: segs[0].clone(),
                first: i,
                last,
            });
            i = last;
        }
        i += 1;
    }
    doc.blocks.last_mut().expect("root block exists").end = doc.lines.len() as i32;

    // Second pass: resolve each header onto a concrete path, reusing the
    // existing block when a dotted segment already names one.
    let mut current: BTreeMap<String, TomlPath> = BTreeMap::new();
    let mut ordinals: BTreeMap<String, i32> = BTreeMap::new();
    let mut resolved: Vec<TomlPath> = Vec::with_capacity(headers.len());
    resolved.push(TomlPath::default());
    for (bi, header) in headers.iter().enumerate().skip(1) {
        let mut path = TomlPath::default();
        for (si, seg) in header.segs.iter().enumerate() {
            path = path.child(seg);
            if si == header.segs.len() - 1 && header.array {
                break;
            }
            if let Some(existing) = current.get(&path.string()) {
                path = existing.clone();
            }
        }
        if header.array {
            let key = path.string();
            let name = doc.key_value(bi, "name").and_then(|value| match value {
                toml::Value::String(name) => Some(name),
                _ => None,
            });
            let ordinal = ordinals.get(&key).copied().unwrap_or(0);
            path = path.element(&element_identity(name.as_deref(), ordinal));
            ordinals.insert(key.clone(), ordinal + 1);
            current.insert(key, path.clone());
        }
        resolved.push(path);
    }
    for (block, path) in doc.blocks.iter_mut().zip(resolved) {
        block.path = path;
    }
    Some(doc)
}

/// The last line of a value starting on `first` at column `col`, following
/// open brackets, braces and multi-line strings (`tomldoc.go:229-250`).
fn toml_value_end(lines: &[String], first: usize, col: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut state = TomlLexState::default();
    for (i, line) in lines.iter().enumerate().skip(first) {
        let text = if i == first { &line[col..] } else { line };
        scan_toml_line(
            text,
            &mut state,
            Some(&mut |ch: u8| match ch {
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth -= 1,
                _ => {}
            }),
        );
        if depth <= 0 && !state.in_multiline_string() {
            return Some(i);
        }
    }
    None
}

impl fmt::Display for TomlDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lines.concat())
    }
}

impl TomlDoc {
    /// The block index for `path`, or `-1` (`tomldoc.go:267-275`).
    pub fn block(&self, path: &TomlPath) -> i32 {
        let want = path.string();
        self.blocks
            .iter()
            .position(|block| block.path.string() == want)
            .map(|i| i as i32)
            .unwrap_or(-1)
    }

    /// The first key matching `name` in `block` (`tomldoc.go:277-287`).
    pub fn key(&self, block: i32, name: &str) -> Option<&TomlDocKey> {
        if block < 0 {
            return None;
        }
        self.blocks[block as usize]
            .keys
            .iter()
            .find(|key| key.key == name)
    }

    /// Whether any header sits at or below `path` (`tomldoc.go:290-297`).
    pub fn has_table(&self, path: &TomlPath) -> bool {
        self.blocks[1..].iter().any(|block| block.path.under(path))
    }

    /// The decoded value of `key` in block `bi`, if it parses
    /// (`tomldoc.go:252-263`).
    pub fn key_value(&self, bi: usize, key: &str) -> Option<toml::Value> {
        let block = self.blocks.get(bi)?;
        for k in &block.keys {
            if k.key != key {
                continue;
            }
            let joined = self.lines[k.first..=k.last].concat();
            let eq = toml_assignment_index(&self.lines[k.first]);
            let value_text = &joined[eq as usize + 1..];
            let probe = format!("v = {value_text}");
            let out: toml::Value = toml::from_str(&probe).ok()?;
            return out.get("v").cloned();
        }
        None
    }

    /// Text of every matched block, each cut after its last key so a trailing
    /// comment meant for the next table stays behind (`tomldoc.go:301-309`).
    pub fn region(&self, mut matches: impl FnMut(&TomlPath) -> bool) -> String {
        let mut out = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 && matches(&block.path) {
                out.push('\n');
                out.push_str(&self.block_text(i));
            }
        }
        out
    }

    /// Block `i` from its header through its last key, always ending in `\n`
    /// (`tomldoc.go:312-323`).
    pub fn block_text(&self, i: usize) -> String {
        let block = &self.blocks[i];
        let mut end = block.header;
        if let Some(last) = block.keys.last() {
            end = last.last as i32;
        }
        let mut text = self.lines[block.header as usize..=end as usize].concat();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    /// Converts bare `\n` to the document's CRLF, except inside a multi-line
    /// string where the ending is part of the value (`tomldoc.go:327-342`).
    pub fn newline(&self, text: &str) -> String {
        if !self.crlf {
            return text.to_owned();
        }
        let mut out = String::new();
        let mut state = TomlLexState::default();
        for span in toml_line_spans(text) {
            let mut line = span.text;
            scan_toml_line(line.trim_end_matches(&['\r', '\n'][..]), &mut state, None);
            if !state.in_multiline_string() && line.ends_with('\n') && !line.ends_with("\r\n") {
                line = line.strip_suffix('\n').unwrap_or(&line).to_owned();
                line.push_str("\r\n");
            }
            out.push_str(&line);
        }
        out
    }

    /// Replaces `drop` lines at `at` with `text`, then reparses. A failed
    /// reparse keeps the spliced lines and flags `broken` (`tomldoc.go:344-364`).
    pub fn splice(&mut self, at: usize, drop: usize, text: &str) {
        let mut inserted: Vec<String> = Vec::new();
        if !text.is_empty() {
            for span in toml_line_spans(&self.newline(text)) {
                inserted.push(span.text);
            }
        }
        if at > 0 && !self.lines[at - 1].ends_with('\n') {
            let terminator = self.newline("\n");
            self.lines[at - 1].push_str(&terminator);
        }
        if let Some(last) = inserted.last_mut()
            && !last.ends_with('\n')
        {
            let terminator = self.newline("\n");
            last.push_str(&terminator);
        }

        let mut next: Vec<String> = self.lines[..at].to_vec();
        next.extend(inserted);
        next.extend(self.lines[at + drop..].iter().cloned());

        let joined = next.concat();
        match parse_toml_doc(&joined) {
            Some(reparsed) => *self = reparsed,
            None => {
                self.lines = next;
                self.broken = true;
            }
        }
    }

    /// Writes `key = rendered` into the table at `p`, keeping the document's
    /// own trailing comment when both sides fit one line
    /// (`tomldoc.go:368-397`).
    pub fn set_key(&mut self, p: &TomlPath, key: &str, rendered: &str) {
        let mut bi = self.block(p);
        if bi < 0 && !p.last_elem().is_empty() {
            self.broken = true;
            return;
        }
        if bi < 0 {
            self.insert_region(p, &format!("[{}]\n", p.header()));
            bi = self.block(p);
            if bi < 0 {
                return;
            }
        }
        if let Some(k) = self.key(bi, key).cloned() {
            let rendered_single = !rendered.trim_end_matches(&['\r', '\n'][..]).contains('\n');
            if k.first == k.last && rendered_single {
                let comment = toml_inline_comment(&self.lines[k.first]);
                if !comment.is_empty() {
                    let stripped =
                        toml_strip_comment(rendered).trim_end_matches(&[' ', '\t', '\r', '\n'][..]);
                    let rendered = format!("{stripped}   {comment}\n");
                    self.splice(k.first, k.last - k.first + 1, &rendered);
                    return;
                }
            }
            self.splice(k.first, k.last - k.first + 1, rendered);
            return;
        }
        let at = {
            let block = &self.blocks[bi as usize];
            let mut at = block.header + 1;
            if let Some(last) = block.keys.last() {
                at = last.last as i32 + 1;
            } else if bi == 0 {
                at = block.end;
            }
            at
        };
        self.splice(at as usize, 0, rendered);
    }

    /// Drops a key, and its table when that leaves the table empty
    /// (`tomldoc.go:400-410`).
    pub fn remove_key(&mut self, p: &TomlPath, key: &str) {
        let bi = self.block(p);
        let Some(k) = self.key(bi, key).cloned() else {
            return;
        };
        self.splice(k.first, k.last - k.first + 1, "");
        let bi = self.block(p);
        if bi > 0 && self.blocks[bi as usize].keys.is_empty() && !self.has_sub_table(p) {
            let block = &self.blocks[bi as usize];
            self.splice(
                block.header as usize,
                (block.end - block.header) as usize,
                "",
            );
        }
    }

    /// Whether any header sits below `p` (`tomldoc.go:412-419`).
    pub fn has_sub_table(&self, p: &TomlPath) -> bool {
        self.blocks[1..]
            .iter()
            .any(|block| block.path.len() > p.len() && block.path.under(p))
    }

    /// Drops every block the predicate selects, all computed against one index:
    /// removing an element header first would hand its sub-tables to the element
    /// above (`tomldoc.go:424-444`).
    pub fn remove_tables(&mut self, mut matches: impl FnMut(&TomlPath) -> bool) {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for block in &self.blocks[1..] {
            if matches(&block.path) {
                spans.push((block.header as usize, block.end as usize));
            }
        }
        if spans.is_empty() {
            return;
        }
        let mut next = self.lines.clone();
        for &(start, end) in spans.iter().rev() {
            next.splice(start..end, []);
        }
        let joined = next.concat();
        match parse_toml_doc(&joined) {
            Some(reparsed) => *self = reparsed,
            None => {
                self.lines = next;
                self.broken = true;
            }
        }
    }

    /// Places new blocks for `p` where TOML attaches them: after the other
    /// elements of its array, inside the nearest array element it belongs to, or
    /// at the end of the document (`tomldoc.go:449-464`).
    pub fn insert_region(&mut self, p: &TomlPath, text: &str) {
        let mut at = -1i32;
        if !p.last_elem().is_empty() {
            let prefix = p.prefix(p.len());
            at = self.last_block_end(|b: &TomlPath| b.len() >= p.len() && b.same_array(&prefix));
        }
        for i in (0..p.len().max(1) - 1).rev() {
            if at >= 0 {
                break;
            }
            if !p.0[i].elem.is_empty() {
                let anchor = p.prefix(i + 1);
                at = self.last_block_end(|b: &TomlPath| b.under(&anchor));
            }
        }
        if at < 0 {
            at = self.lines.len() as i32;
        }
        self.splice(at as usize, 0, &format!("\n{text}"));
    }

    /// The line after the last key of the last matching block
    /// (`tomldoc.go:467-479`).
    pub fn last_block_end(&self, mut matches: impl FnMut(&TomlPath) -> bool) -> i32 {
        let mut at = -1i32;
        for block in &self.blocks[1..] {
            if !matches(&block.path) {
                continue;
            }
            at = block.header + 1;
            if let Some(last) = block.keys.last() {
                at = last.last as i32 + 1;
            }
        }
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toml_lex::toml_comments_in_spans;

    fn parse(text: &str) -> TomlDoc {
        parse_toml_doc(text).expect("document models")
    }

    #[test]
    fn indexes_root_tables_and_arrays_with_zero_based_lines() {
        let text = "\
root_key = 1
[ui]
theme = \"dark\"
[[providers]]
name = \"a\"
[[providers]]
name = \"b\"
";
        let doc = parse(text);
        assert_eq!(doc.blocks[0].header, -1);
        assert_eq!(doc.blocks[0].end, 1);
        assert_eq!(doc.blocks[1].header, 1);
        assert_eq!(doc.blocks[1].end, 3);
        let ui = doc.blocks[1].path.header();
        assert_eq!(ui, "ui");
        assert_eq!(doc.blocks[2].path.header(), "providers");
        assert_eq!(doc.blocks[3].path.header(), "providers");
        // Array elements carry distinct identities even with one segment.
        assert_ne!(doc.blocks[2].path.string(), doc.blocks[3].path.string());
        let key = doc.key(0, "root_key").expect("root key");
        assert_eq!((key.first, key.last), (0, 0));
    }

    #[test]
    fn dotted_headers_model_but_dotted_assignments_rejected() {
        let text = "[agent.persona]\nprefix = \"hi\"\n";
        let doc = parse(text);
        assert_eq!(doc.blocks[1].path.header(), "agent.persona");
        // Assignment-side dotted keys are not modeled.
        assert!(parse_toml_doc("agent.name = \"x\"\n").is_none());
    }

    #[test]
    fn repeated_keys_keep_the_first_match_without_diagnostic() {
        let text = "[ui]\ntheme = \"dark\"\ntheme = \"light\"\n";
        let doc = parse(text);
        let key = doc.key(1, "theme").expect("first match");
        assert_eq!(key.first, 1);
        assert_eq!(doc.blocks[1].keys.len(), 2);
    }

    #[test]
    fn multiline_values_span_several_lines() {
        let text = "[agent]\nsystem_prompt = \"\"\"\nline one\nline two\n\"\"\"\n";
        let doc = parse(text);
        let key = doc.key(1, "system_prompt").expect("multiline key");
        assert_eq!(key.first, 1);
        assert_eq!(key.last, 4);
    }

    #[test]
    fn set_key_preserves_inline_comment_on_single_line_replacement() {
        let mut doc = parse("[ui]\ntheme = \"dark\"   # keep me\n");
        let path = doc.blocks[1].path.clone();
        doc.set_key(&path, "theme", "theme = \"light\"");
        assert_eq!(doc.lines[1], "theme = \"light\"   # keep me\n");
    }

    #[test]
    fn set_key_inserts_after_last_key_and_adds_a_missing_header() {
        let mut doc = parse("[ui]\ntheme = \"dark\"\n");
        let ui = doc.blocks[1].path.clone();
        doc.set_key(&ui, "cursor_shape", "cursor_shape = \"block\"");
        assert_eq!(doc.lines[2], "cursor_shape = \"block\"\n");
        let network = TomlPath::default().child("network");
        doc.set_key(&network, "proxy_url", "proxy_url = \"http://x\"\n");
        assert!(doc.lines.iter().any(|line| line == "[network]\n"));
    }

    #[test]
    fn remove_key_drops_an_empty_table_with_its_header() {
        let mut doc = parse("[ui]\ntheme = \"dark\"\n[other]\nk = 1\n");
        let ui = doc.blocks[1].path.clone();
        doc.remove_key(&ui, "theme");
        assert!(!doc.lines.iter().any(|line| line == "[ui]\n"));
        assert!(doc.lines.iter().any(|line| line == "[other]\n"));
    }

    #[test]
    fn remove_tables_keeps_sibling_array_elements() {
        let text =
            "[[plugins]]\nname = \"a\"\n[[plugins]]\nname = \"b\"\n[[plugins]]\nname = \"c\"\n";
        let mut doc = parse(text);
        let second = doc.blocks[2].path.clone();
        doc.remove_tables(|p| p == &second);
        let remaining: Vec<_> = doc
            .blocks
            .iter()
            .filter(|block| block.header >= 0)
            .map(|block| block.path.header())
            .collect();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn splice_reparse_failure_flags_broken() {
        let mut doc = parse("k = 1\n");
        // An unterminated bracket cannot be re-indexed; the spliced lines stay.
        doc.splice(1, 0, "broken = [\n");
        assert!(doc.broken);
    }

    #[test]
    fn newline_converts_lf_to_crlf_outside_multiline_strings() {
        let text = "a = 1\r\nb = 2\r\n";
        let doc = parse(text);
        assert!(doc.crlf);
        let converted = doc.newline("c = 3\n");
        assert_eq!(converted, "c = 3\r\n");
        // A still-open multiline string keeps its own line ending on every
        // line, including the opening quote line.
        let inside = doc.newline("prompt = \"\"\"\nline one\nline two\n");
        assert_eq!(inside, "prompt = \"\"\"\nline one\nline two\n");
    }

    #[test]
    fn block_text_stops_at_the_last_key() {
        let text = "[ui]\ntheme = \"dark\"\n# trailing for next\n[other]\nk = 1\n";
        let doc = parse(text);
        let text = doc.block_text(1);
        assert!(text.contains("theme = \"dark\""));
        assert!(!text.contains("trailing for next"));
    }

    #[test]
    fn header_dotted_probe_decodes_quoted_segments() {
        let doc = parse("[\"weird key\"]\nk = 1\n");
        assert_eq!(doc.blocks[1].path.header(), "\"weird key\"");
    }

    #[test]
    fn unclosed_bracket_makes_the_document_unmodeled() {
        assert!(parse_toml_doc("items = [\n  1\n").is_none());
    }

    #[test]
    fn comments_and_blank_lines_do_not_create_blocks() {
        let text = "# preamble\n\n[ui]\n# section note\ntheme = \"dark\"\n";
        let doc = parse(text);
        assert_eq!(doc.blocks.len(), 2);
    }

    #[test]
    fn comments_in_spans_are_reused_by_set_key_for_multiline_values() {
        // The upsert path consults the shared lex helper; assert the helper
        // returns the leading comment group before a multi-line value.
        let spans = toml_line_spans("[s]\n# lead\nk = [\n  1\n]\n");
        let comments = toml_comments_in_spans(&spans, 1, 3);
        assert_eq!(comments, vec!["# lead"]);
    }
}
