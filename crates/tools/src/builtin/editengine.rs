//! The shared edit_file / multi_edit contract (encoding_helpers.go): exact
//! match first, then a narrow fuzzy match for the mismatches read_file
//! introduces or hides, plus the not-found / not-unique refusal texts.
//!
//! Content is held as bytes, the way Go strings are: a LossyUTF8 file's
//! bytes are not valid UTF-8 and must survive an edit untouched. Model
//! input (old_string / new_string) arrives as UTF-8 text, so searches and
//! receipts operate on byte slices that are valid UTF-8 everywhere except
//! regions the edit never touched.

#![forbid(unsafe_code)]

use super::fuzzyedit::fuzzy_edit_ranges;
use super::receipt::{MAX_CAPTURED_RECEIPT_SPAN_BYTES, clip_post_write_span};

/// encoding_helpers.go `editApplyResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditApplyResult {
    pub updated: Vec<u8>,
    pub applied: usize,
    pub matches: usize,
    pub fuzzy: bool,
    pub receipt: EditReplacementReceipt,
}

/// encoding_helpers.go `editReplacementReceipt`: records only the span the
/// tool actually matched and the span it wrote in its place. It
/// deliberately excludes surrounding file content so a successful edit can
/// ground the next model turn without widening provider-visible workspace
/// data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditReplacementReceipt {
    pub matched: Vec<u8>,
    pub replacement: Vec<u8>,
    pub occurrences: usize,
    pub fuzzy: bool,
}

/// encoding_helpers.go `editRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditRange {
    pub start: usize,
    pub end: usize,
}

/// encoding_helpers.go `lineSegment`. Raw keeps the terminator, like Go.
#[derive(Debug, Clone)]
pub struct LineSeg {
    pub raw: Vec<u8>,
    pub start: usize,
    pub end: usize,
}

/// encoding.go `matchLineEndings`: adapts an edit's old/new text to a CRLF
/// file when the literal old_string isn't present but its CRLF form is.
/// read_file strips '\r', so a model's multi-line old_string arrives
/// LF-only while a Windows/CJK source stores '\r\n'.
pub fn match_line_endings(content: &[u8], old: &[u8], new: &[u8]) -> (Vec<u8>, Vec<u8>) {
    if contains(content, old) || !contains(content, b"\r\n") {
        return (old.to_vec(), new.to_vec());
    }
    let crlf_old = to_crlf(old);
    if contains(content, &crlf_old) {
        return (crlf_old, to_crlf(new));
    }
    (old.to_vec(), new.to_vec())
}

pub fn to_crlf(s: &[u8]) -> Vec<u8> {
    let normalized: Vec<u8> = replace_bytes(s, b"\r\n", b"\n", true).0;
    replace_bytes(&normalized, b"\n", b"\r\n", true).0
}

/// encoding_helpers.go `lineEndingAt`: whether the line holding `at` ends
/// in CRLF (that line's newline, or the previous line's when it is the
/// file's last and unterminated).
fn line_ending_at(content: &[u8], at: usize) -> bool {
    let i = match content[at..].iter().position(|&b| b == b'\n') {
        Some(i) => Some(i + at),
        None => content[..at].iter().rposition(|&b| b == b'\n'),
    };
    match i {
        Some(i) if i > 0 => content[i - 1] == b'\r',
        _ => false,
    }
}

/// encoding_helpers.go `replaceLiteral`: swaps exact matches of old and
/// gives each replacement the line ending of the line it lands in, so a
/// CRLF line never receives a bare LF and an LF line in a mixed file never
/// receives a CR. Returns the first replacement as written, for the
/// receipt.
pub fn replace_literal(
    content: &[u8],
    old: &[u8],
    replacement: &[u8],
    all: bool,
) -> (Vec<u8>, Vec<u8>, usize) {
    if old.is_empty() {
        // Go strings.Replace with an empty old inserts between runes; the
        // tools refuse an empty old_string before reaching here.
        return replace_empty_old(content, replacement, all);
    }
    let mut b: Vec<u8> = Vec::with_capacity(content.len());
    let mut pos = 0usize;
    let mut n = 0usize;
    let mut first: Vec<u8> = Vec::new();
    while let Some(i) = find_sub(content, old, pos) {
        let mut rep = replacement.to_vec();
        if line_ending_at(content, i) {
            rep = to_crlf(replacement);
        }
        if n == 0 {
            first = rep.clone();
        }
        b.extend_from_slice(&content[pos..i]);
        b.extend_from_slice(&rep);
        pos = i + old.len();
        n += 1;
        if !all {
            break;
        }
    }
    b.extend_from_slice(&content[pos..]);
    (b, first, n)
}

fn replace_empty_old(content: &[u8], replacement: &[u8], all: bool) -> (Vec<u8>, Vec<u8>, usize) {
    let mut b = Vec::with_capacity(content.len());
    let mut n = 0usize;
    for &byte in content {
        if (byte & 0xC0) != 0x80 && (all || n == 0) {
            b.extend_from_slice(replacement);
            n += 1;
        }
        b.push(byte);
    }
    if all || n == 0 {
        b.extend_from_slice(replacement);
        n += 1;
    }
    let first = if n > 0 {
        replacement.to_vec()
    } else {
        Vec::new()
    };
    (b, first, n)
}

pub fn match_replacement_line_endings(content: &[u8], replacement: &[u8]) -> Vec<u8> {
    if contains(content, b"\r\n") {
        to_crlf(replacement)
    } else {
        replacement.to_vec()
    }
}

/// encoding_helpers.go `applyOldStringEdit`: the shared edit contract. It
/// preserves the exact-match rule first, then falls back to a narrow fuzzy
/// match for the mismatches read_file introduces or hides: trailing
/// whitespace, tab-vs-spaces indentation, read_file line prefixes, and a
/// blank-line run of a different length. Non-replace_all edits still
/// require exactly one match.
pub fn apply_old_string_edit(
    content: &[u8],
    old_string: &[u8],
    new_string: &[u8],
    replace_all: bool,
) -> EditApplyResult {
    let (old, new_str) = match_line_endings(content, old_string, new_string);
    if replace_all {
        let count = count_occurrences(content, &old);
        if count > 0 {
            let (updated, first, _) = replace_literal(content, &old, &new_str, true);
            return EditApplyResult {
                updated,
                applied: count,
                matches: count,
                fuzzy: false,
                receipt: EditReplacementReceipt {
                    matched: old,
                    replacement: first,
                    occurrences: count,
                    fuzzy: false,
                },
            };
        }
        let ranges = fuzzy_edit_ranges(content, &old);
        if ranges.is_empty() {
            return EditApplyResult {
                updated: content.to_vec(),
                applied: 0,
                matches: 0,
                fuzzy: false,
                receipt: EditReplacementReceipt::default(),
            };
        }
        let replacement = match_replacement_line_endings(content, &new_str);
        return EditApplyResult {
            updated: replace_edit_ranges(content, &ranges, &replacement),
            applied: ranges.len(),
            matches: ranges.len(),
            fuzzy: true,
            receipt: EditReplacementReceipt {
                matched: matched_range_sample(content, &old, &ranges),
                replacement: replacement.clone(),
                occurrences: ranges.len(),
                fuzzy: true,
            },
        };
    }

    match count_occurrences(content, &old) {
        0 => {
            let ranges = fuzzy_edit_ranges(content, &old);
            if ranges.len() != 1 {
                return EditApplyResult {
                    updated: content.to_vec(),
                    applied: 0,
                    matches: ranges.len(),
                    fuzzy: false,
                    receipt: EditReplacementReceipt::default(),
                };
            }
            let replacement = match_replacement_line_endings(content, &new_str);
            EditApplyResult {
                updated: replace_edit_ranges(content, &ranges, &replacement),
                applied: 1,
                matches: 1,
                fuzzy: true,
                receipt: EditReplacementReceipt {
                    matched: matched_range_sample(content, &old, &ranges),
                    replacement: replacement.clone(),
                    occurrences: 1,
                    fuzzy: true,
                },
            }
        }
        1 => {
            let (updated, replacement, _) = replace_literal(content, &old, &new_str, false);
            EditApplyResult {
                updated,
                applied: 1,
                matches: 1,
                fuzzy: false,
                receipt: EditReplacementReceipt {
                    matched: old,
                    replacement,
                    occurrences: 1,
                    fuzzy: false,
                },
            }
        }
        count => EditApplyResult {
            updated: content.to_vec(),
            applied: 0,
            matches: count,
            fuzzy: false,
            receipt: EditReplacementReceipt::default(),
        },
    }
}

/// encoding_helpers.go `matchedRangeSample`: the actual matched bytes,
/// clipped to the receipt span ceiling; an unclipped sample is cloned out
/// of the buffer so a short substring cannot keep a large intermediate
/// multi_edit buffer alive.
pub fn matched_range_sample(content: &[u8], fallback: &[u8], ranges: &[EditRange]) -> Vec<u8> {
    let Some(r) = ranges.first() else {
        return fallback.to_vec();
    };
    if r.end > content.len() || r.end < r.start {
        return fallback.to_vec();
    }
    clip_post_write_span(&content[r.start..r.end], MAX_CAPTURED_RECEIPT_SPAN_BYTES)
}

pub fn count_occurrences(content: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        // Go strings.Count with an empty needle: one past every rune.
        return content.iter().filter(|&&b| (b & 0xC0) != 0x80).count() + 1;
    }
    let mut n = 0usize;
    let mut pos = 0usize;
    while let Some(i) = find_sub(content, needle, pos) {
        n += 1;
        pos = i + needle.len();
    }
    n
}

pub fn find_sub(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from > haystack.len() - needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find_sub(haystack, needle, 0).is_some()
}

pub fn trim_right(s: &[u8], set: &[u8]) -> Vec<u8> {
    let mut end = s.len();
    while end > 0 && set.contains(&s[end - 1]) {
        end -= 1;
    }
    s[..end].to_vec()
}

pub fn trim_left(s: &[u8], set: &[u8]) -> Vec<u8> {
    let mut start = 0usize;
    while start < s.len() && set.contains(&s[start]) {
        start += 1;
    }
    s[start..].to_vec()
}

pub fn replace_bytes(s: &[u8], from: &[u8], to: &[u8], all: bool) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(s.len());
    let mut pos = 0usize;
    let mut n = 0usize;
    while let Some(i) = find_sub(s, from, pos) {
        out.extend_from_slice(&s[pos..i]);
        out.extend_from_slice(to);
        pos = i + from.len();
        n += 1;
        if !all {
            break;
        }
    }
    out.extend_from_slice(&s[pos..]);
    (out, n)
}

/// encoding_helpers.go `replaceEditRanges`: ranges applied from the back so
/// earlier offsets stay valid.
pub fn replace_edit_ranges(content: &[u8], ranges: &[EditRange], replacement: &[u8]) -> Vec<u8> {
    let mut updated = content.to_vec();
    for r in ranges.iter().rev() {
        let (start, end) = (r.start.min(updated.len()), r.end.min(updated.len()));
        let mut next = Vec::with_capacity(updated.len());
        next.extend_from_slice(&updated[..start]);
        next.extend_from_slice(replacement);
        next.extend_from_slice(&updated[end..]);
        updated = next;
    }
    updated
}

pub fn split_line_segments(s: &[u8]) -> Vec<LineSeg> {
    let mut lines: Vec<LineSeg> = Vec::new();
    let mut start = 0usize;
    for (i, &b) in s.iter().enumerate() {
        if b == b'\n' {
            let end = i + 1;
            lines.push(LineSeg {
                raw: s[start..end].to_vec(),
                start,
                end,
            });
            start = end;
        }
    }
    if start < s.len() {
        lines.push(LineSeg {
            raw: s[start..].to_vec(),
            start,
            end: s.len(),
        });
    }
    lines
}

pub fn line_has_newline(line: &[u8]) -> bool {
    line.ends_with(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(content: &str, old: &str, new: &str, all: bool) -> EditApplyResult {
        apply_old_string_edit(content.as_bytes(), old.as_bytes(), new.as_bytes(), all)
    }

    #[test]
    fn literal_edit_keeps_the_line_ending_of_the_line_it_lands_in() {
        // edit_literal_endings_test.go table.
        let cases: Vec<(&str, &str, &str, &str, bool)> = vec![
            (
                "[client]\r\nretries=3\r\nbackoff=2\r\n",
                "retries=3",
                "retries=3\ntimeout=30",
                "[client]\r\nretries=3\r\ntimeout=30\r\nbackoff=2\r\n",
                false,
            ),
            (
                "a=1\r\nb=1\r\n",
                "=1",
                "=1\n# x",
                "a=1\r\n# x\r\nb=1\r\n# x\r\n",
                true,
            ),
            (
                "a\r\nretries=3",
                "retries=3",
                "retries=3\nx",
                "a\r\nretries=3\r\nx",
                false,
            ),
            (
                "[client]\nretries=3\n",
                "retries=3",
                "retries=3\ntimeout=30",
                "[client]\nretries=3\ntimeout=30\n",
                false,
            ),
            (
                "head\r\nretries=3\nbackoff=2\n",
                "retries=3",
                "retries=3\ntimeout=30",
                "head\r\nretries=3\ntimeout=30\nbackoff=2\n",
                false,
            ),
            (
                "retries=3\r\nbackoff=2\nx\n",
                "retries=3",
                "retries=3\ntimeout=30",
                "retries=3\r\ntimeout=30\r\nbackoff=2\nx\n",
                false,
            ),
            ("a=1\r\nb=1\n", "=1", "=1\n#", "a=1\r\n#\r\nb=1\n#\n", true),
            ("a\r\nb\r\n", "a", "a\r\nz", "a\r\nz\r\nb\r\n", false),
            ("a\r\nb\r\n", "a", "c", "c\r\nb\r\n", false),
        ];
        for (content, old, new, want, all) in cases {
            let got = apply(content, old, new, all);
            assert_eq!(
                String::from_utf8_lossy(&got.updated),
                want,
                "updated mismatch for old={old:?} new={new:?}"
            );
        }
    }

    #[test]
    fn crlf_multi_line_old_normalizes_when_literal_missing() {
        // crlf_edit_test.go: LF-only old_string against a CRLF file.
        let got = apply("one\r\ntwo\r\nthree\r\n", "one\ntwo", "ONE\nTWO", false);
        assert_eq!(
            String::from_utf8_lossy(&got.updated),
            "ONE\r\nTWO\r\nthree\r\n"
        );
    }

    #[test]
    fn lf_file_stays_lf() {
        let got = apply("alpha\nbeta\ngamma\n", "alpha\nbeta", "ALPHA\nBETA", false);
        assert_eq!(
            String::from_utf8_lossy(&got.updated),
            "ALPHA\nBETA\ngamma\n"
        );
        assert!(!got.updated.contains(&b'\r'));
    }

    #[test]
    fn multiple_matches_report_count_without_writing() {
        let got = apply("a b a b", "a", "x", false);
        assert_eq!(got.applied, 0);
        assert_eq!(got.matches, 2);
    }

    #[test]
    fn lossy_bytes_outside_the_edit_survive() {
        let content = b"value := 1\n// \xff\n".to_vec();
        let got = apply_old_string_edit(&content, b"value := 1", b"value := 2", false);
        assert_eq!(got.updated, b"value := 2\n// \xff\n".to_vec());
    }
}
