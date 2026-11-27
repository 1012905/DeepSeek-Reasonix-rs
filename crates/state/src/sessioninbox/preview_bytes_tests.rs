use super::*;

#[test]
fn byte_preview_skips_invalid_utf8_but_preserves_an_encoded_replacement() {
    assert_eq!(
        preview_text_bytes(b"a\xffb\xc0\xafc\xed\xa0\x80d", 120),
        "abcd"
    );
    assert_eq!(preview_text_bytes(b"a\xef\xbf\xbdb", 120), "a\u{fffd}b");
    assert_eq!(preview_text_bytes(b"\xef\xbf\xbdrest", 1), "\u{fffd}…");
    assert_eq!(preview_text_bytes(b"\xff\xfe\xf5\x80\x80\x80", 120), "");
}

#[test]
fn byte_preview_trims_go_unicode_whitespace_but_collapses_only_four_characters() {
    let whitespace = [
        '\u{0009}', '\u{000a}', '\u{000b}', '\u{000c}', '\u{000d}', '\u{0020}', '\u{0085}',
        '\u{00a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
        '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ];
    for space in whitespace {
        let text = format!("{space}alpha{space}");
        assert_eq!(preview_text(&text, 120), "alpha", "{space:?}");
        if !matches!(space, '\n' | '\r' | '\t' | ' ') {
            let internal = format!("a{space}b");
            assert_eq!(preview_text(&internal, 120), internal, "{space:?}");
        }
    }
    assert_eq!(preview_text("a\n\r\t b", 120), "a b");
    assert_eq!(
        preview_text("\u{200b}alpha\u{200b}", 120),
        "\u{200b}alpha\u{200b}"
    );
    assert_eq!(
        preview_text("\u{feff}alpha\u{feff}", 120),
        "\u{feff}alpha\u{feff}"
    );
}

#[test]
fn byte_preview_uses_raw_remaining_bytes_for_the_ellipsis() {
    assert_eq!(preview_text_bytes(b"ab\xff", 2), "ab…");
    assert_eq!(preview_text_bytes(b"a\xffb", 2), "ab");
    assert_eq!(preview_text_bytes(b"a\xffbc", 2), "ab…");
    assert_eq!(preview_text_bytes(b"a \xff", 1), "a…");
    assert_eq!(preview_text_bytes(b"a \xff", 2), "a");
    assert_eq!(preview_text_bytes(b"\xffa", 1), "a");
    assert_eq!(preview_text_bytes("\u{00a0}a\u{00a0}".as_bytes(), 1), "a");
}

#[test]
fn string_preview_delegates_to_the_byte_algorithm_and_keeps_default_limits() {
    for text in ["  alpha\n beta  ", "😀é中z", "a\u{00a0}\tb", ""] {
        for limit in [-1, 0, 1, 2, 120] {
            assert_eq!(
                preview_text(text, limit),
                preview_text_bytes(text.as_bytes(), limit),
                "{text:?} {limit}"
            );
        }
    }
    let text = format!("{}y", "x".repeat(120));
    assert_eq!(
        preview_text_bytes(text.as_bytes(), 0),
        format!("{}…", "x".repeat(120))
    );
}

#[test]
fn reference_summaries_preview_raw_bytes_without_lossy_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("session.jsonl"), Limits::default()).unwrap();
    let receipt = store
        .enqueue(EnqueueRequest {
            intent: INTENT_FOLLOWUP.clone(),
            envelope: PromptEnvelope {
                submit_text: "body".to_owned(),
                refs: vec![RefSnapshot {
                    kind: "frozen".to_owned(),
                    path: "reference.txt".to_owned(),
                    content: b"\xffA".to_vec(),
                    ..RefSnapshot::default()
                }],
                ..PromptEnvelope::default()
            },
            source: "test".to_owned(),
            ..EnqueueRequest::default()
        })
        .unwrap();

    let meta = store.read_item(&receipt.item_id).unwrap().0;
    assert_eq!(meta.refs[0].preview, "A");
}
