//! Post-write replacement receipts (post_write_receipt.go): appended after
//! every edit to ground the next model turn. The receipt contains only the
//! matched and replacement spans, never unchanged same-line or neighboring
//! data, and keeps its dynamic tail bounded.

#![forbid(unsafe_code)]

use super::editengine::EditReplacementReceipt;

/// post_write_receipt.go bounds: the receipt rides every edit in the
/// provider-visible conversation.
pub const MAX_POST_WRITE_RECEIPT_BYTES: usize = 2048;
pub const MAX_CAPTURED_RECEIPT_SPAN_BYTES: usize = 896;

pub const POST_WRITE_SPAN_TRUNCATED: &str = "…[replacement span truncated]…";
pub const POST_WRITE_RECEIPT_TRUNCATED: &str =
    "…[replacement receipt truncated; use read_file for complete current contents]…";

/// Go `withActualPostWriteReceipts`: the summary plus the bounded receipt
/// body, or the summary alone when there is nothing to report.
pub fn with_actual_post_write_receipts(
    summary: &str,
    receipts: &[EditReplacementReceipt],
) -> String {
    if receipts.is_empty() {
        return String::from(summary);
    }
    let body = render_post_write_receipts(receipts);
    if body.trim().is_empty() {
        return String::from(summary);
    }
    format!("{summary}\nActual replacement receipt after write:\n{body}")
}

/// Go `renderPostWriteReceipts`: first and last receipt only once the
/// count passes two, spans clipped, whole body bounded.
pub fn render_post_write_receipts(receipts: &[EditReplacementReceipt]) -> String {
    let indexes = receipt_indexes(receipts.len());
    if indexes.is_empty() {
        return String::new();
    }
    // Share the bounded body between the selected first/last receipts and
    // their matched/replacement fields. The final clip remains a
    // defensive cap for unusually large counts or labels.
    let field_budget = ((MAX_POST_WRITE_RECEIPT_BYTES - 256) / (2 * indexes.len()))
        .clamp(128, MAX_CAPTURED_RECEIPT_SPAN_BYTES);

    let mut b: Vec<u8> = Vec::with_capacity(MAX_POST_WRITE_RECEIPT_BYTES);
    for (pos, idx) in indexes.iter().enumerate() {
        if pos > 0 {
            b.push(b'\n');
        }
        if receipts.len() > 2 && pos == 1 {
            b.extend_from_slice(
                format!(
                    "…[{} intermediate replacement receipt(s) omitted]…\n\n",
                    receipts.len() - 2
                )
                .as_bytes(),
            );
        }
        let r = &receipts[*idx];
        let mut occurrences = r.occurrences;
        if occurrences == 0 {
            occurrences = 1;
        }
        let mut fuzzy = String::new();
        if r.fuzzy {
            fuzzy = String::from(", fuzzy match");
            if occurrences > 1 {
                fuzzy.push_str(", first matched sample shown");
            }
        }
        b.extend_from_slice(
            format!(
                "@@ replacement {} of {} ({} occurrence(s){}) @@\n",
                idx + 1,
                receipts.len(),
                occurrences,
                fuzzy
            )
            .as_bytes(),
        );
        append_receipt_span(
            &mut b,
            b'-',
            &clip_post_write_span(&r.matched, field_budget),
        );
        append_receipt_span(
            &mut b,
            b'+',
            &clip_post_write_span(&r.replacement, field_budget),
        );
    }
    clip_post_write_receipt(&b)
}

/// Go `receiptIndexes`.
fn receipt_indexes(count: usize) -> Vec<usize> {
    match count {
        0 => Vec::new(),
        1 => vec![0],
        2 => vec![0, 1],
        _ => vec![0, count - 1],
    }
}

/// Go `appendReceiptSpan`: every line of the span carries the prefix, so
/// an empty replacement still renders one line.
fn append_receipt_span(b: &mut Vec<u8>, prefix: u8, text: &[u8]) {
    if text.is_empty() {
        b.push(prefix);
        b.extend_from_slice(b"<empty>\n");
        return;
    }
    let mut rest = text;
    while !rest.is_empty() {
        let (line, tail) = match rest.iter().position(|&c| c == b'\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        b.push(prefix);
        b.extend_from_slice(line);
        b.push(b'\n');
        rest = tail;
    }
}

/// Go `clipPostWriteSpan`.
pub fn clip_post_write_span(text: &[u8], budget: usize) -> Vec<u8> {
    if text.len() <= budget {
        return text.to_vec();
    }
    let marker = format!("\n{POST_WRITE_SPAN_TRUNCATED}\n");
    clip_utf8_head_tail(text, budget, marker.as_bytes())
}

/// Go `clipPostWriteReceipt`.
fn clip_post_write_receipt(text: &[u8]) -> String {
    if text.len() <= MAX_POST_WRITE_RECEIPT_BYTES {
        return String::from_utf8_lossy(text).into_owned();
    }
    let marker = format!("\n{POST_WRITE_RECEIPT_TRUNCATED}\n");
    String::from_utf8_lossy(&clip_utf8_head_tail(
        text,
        MAX_POST_WRITE_RECEIPT_BYTES,
        marker.as_bytes(),
    ))
    .into_owned()
}

/// Go `clipUTF8HeadTail`: 3/4 head plus 1/4 tail around the marker, cut on
/// UTF-8 boundaries.
fn clip_utf8_head_tail(text: &[u8], budget: usize, marker: &[u8]) -> Vec<u8> {
    if budget < marker.len() {
        return clip_utf8_prefix(marker, budget);
    }
    let available = budget - marker.len();
    let head_bytes = available * 3 / 4;
    let tail_bytes = available - head_bytes;
    let head_end = utf8_prefix_boundary(text, head_bytes);
    let tail_start = utf8_suffix_boundary(text, text.len().saturating_sub(tail_bytes));
    let mut out = Vec::with_capacity(budget + marker.len());
    out.extend_from_slice(&text[..head_end]);
    out.extend_from_slice(marker);
    out.extend_from_slice(&text[tail_start..]);
    out
}

fn clip_utf8_prefix(text: &[u8], end: usize) -> Vec<u8> {
    if end >= text.len() {
        return text.to_vec();
    }
    text[..utf8_prefix_boundary(text, end)].to_vec()
}

/// Go `utf8PrefixBoundary`: walks back off any partial rune.
pub fn utf8_prefix_boundary(text: &[u8], end: usize) -> usize {
    let mut end = end.min(text.len());
    while end > 0 && !rune_start(text[end]) {
        end -= 1;
    }
    end
}

fn utf8_suffix_boundary(text: &[u8], start: usize) -> usize {
    let mut start = start;
    if start == 0 {
        return 0;
    }
    if start >= text.len() {
        return text.len();
    }
    while start < text.len() && !rune_start(text[start]) {
        start += 1;
    }
    start
}

/// Go `utf8.RuneStart`.
fn rune_start(b: u8) -> bool {
    b & 0xC0 != 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(
        matched: &str,
        replacement: &str,
        occurrences: usize,
        fuzzy: bool,
    ) -> EditReplacementReceipt {
        EditReplacementReceipt {
            matched: matched.as_bytes().to_vec(),
            replacement: replacement.as_bytes().to_vec(),
            occurrences,
            fuzzy,
        }
    }

    #[test]
    fn clip_post_write_span_preserves_utf8_and_both_ends() {
        let text = format!("head\n{}tail\n", "修改后的内容\n".repeat(800));
        let got = clip_post_write_span(text.as_bytes(), 512);
        assert!(got.len() <= 512, "clipped span = {} bytes", got.len());
        assert!(
            std::str::from_utf8(&got).is_ok(),
            "clipped span split a UTF-8 rune"
        );
        let got = String::from_utf8(got).unwrap();
        for want in ["head\n", "tail\n", POST_WRITE_SPAN_TRUNCATED] {
            assert!(
                got.contains(want),
                "clipped span should preserve {want:?}:\n{got}"
            );
        }
    }

    #[test]
    fn render_post_write_receipts_keeps_first_and_last_bounded() {
        let receipts = vec![
            receipt("first-old", "first-new", 1, false),
            receipt("middle-old", "middle-new", 2, false),
            receipt(&"最后一项".repeat(500), "last-new", 1, true),
        ];
        let got = render_post_write_receipts(&receipts);
        assert!(
            got.len() <= MAX_POST_WRITE_RECEIPT_BYTES,
            "receipt = {} bytes",
            got.len()
        );
        assert!(got.is_char_boundary(got.len()));
        for want in [
            "-first-old",
            "+first-new",
            "+last-new",
            "1 intermediate replacement receipt(s) omitted",
            "fuzzy match",
            POST_WRITE_SPAN_TRUNCATED,
        ] {
            assert!(
                got.contains(want),
                "receipt should contain {want:?}:\n{got}"
            );
        }
        assert!(
            !got.contains("middle-old") && !got.contains("middle-new"),
            "bounded receipt should omit intermediate spans:\n{got}"
        );
    }

    #[test]
    fn post_write_receipt_large_span_is_bounded() {
        let large = "large replacement payload\n".repeat(700_000);
        let receipts = vec![receipt(&large, &large, 1, false)];
        let got = with_actual_post_write_receipts("edited large.txt", &receipts);
        assert!(
            got.len() <= MAX_POST_WRITE_RECEIPT_BYTES + 256,
            "bounded result is too large: {} bytes",
            got.len()
        );
        assert!(got.contains(POST_WRITE_SPAN_TRUNCATED));
    }

    #[test]
    fn delete_renders_empty_replacement_line() {
        let got = with_actual_post_write_receipts(
            "edited a.txt",
            &[receipt("remove this line\n", "", 1, false)],
        );
        for want in [
            "Actual replacement receipt after write:",
            "-remove this line",
            "+<empty>",
        ] {
            assert!(
                got.contains(want),
                "delete result should contain {want:?}:\n{got}"
            );
        }
    }
}
