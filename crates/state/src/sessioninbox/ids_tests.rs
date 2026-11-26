use super::*;

#[test]
fn sha256_hex_matches_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(&[0xff; 32]),
        "af9613760f72635fbdb44a5a0a63c39f12af30f950a6ee5c971be188e89c4051"
    );
}

#[test]
fn preview_trims_and_collapses_only_go_preview_whitespace() {
    assert_eq!(
        preview_text("  alpha\n\r\t beta  gamma\tdelta  ", 120),
        "alpha beta gamma delta"
    );
    assert_eq!(preview_text("a\u{00a0}\tb", 120), "a\u{00a0} b");
    assert_eq!(
        preview_text("a\u{000b}\u{000c}b", 120),
        "a\u{000b}\u{000c}b"
    );
    assert_eq!(preview_text(" \n\r\t ", 120), "");
}

#[test]
fn preview_counts_unicode_scalars_and_marks_remaining_content() {
    assert_eq!(preview_text("😀é中z", 2), "😀é…");
    assert_eq!(preview_text("😀é中z", 4), "😀é中z");
    assert_eq!(preview_text("😀é中z", 1), "😀…");
    assert_eq!(preview_text("  😀é中z  ", 3), "😀é中…");
}

#[test]
fn preview_uses_default_limit_for_nonpositive_values() {
    let text = format!("{}y", "x".repeat(120));
    assert_eq!(preview_text(&text, 0), format!("{}…", "x".repeat(120)));
    assert_eq!(preview_text(&text, -1), format!("{}…", "x".repeat(120)));
    assert_eq!(preview_text(&"x".repeat(120), 0), "x".repeat(120));
}

#[test]
fn preview_space_at_limit_follows_go_trim_behavior() {
    assert_eq!(preview_text("a bc", 2), "a");
    assert_eq!(preview_text("ab c", 2), "ab…");
    assert_eq!(preview_text("a  bc", 2), "a");
    assert_eq!(preview_text("a b", 3), "a b");
}

#[test]
fn preview_does_not_materialize_a_large_body() {
    let text = "x".repeat(1 << 20);
    let preview = preview_text(&text, 40);
    assert_eq!(preview, format!("{}…", "x".repeat(40)));
}

#[test]
fn process_run_id_is_stable_and_uuid_v4_shaped() {
    let first = process_run_id();
    assert_eq!(first, process_run_id());
    assert_run_id_shape(first);
}

#[test]
fn fresh_random_ids_are_uuid_v4_shaped() {
    for _ in 0..32 {
        assert_run_id_shape(&super::ids::new_random_id());
    }
}

fn assert_run_id_shape(id: &str) {
    assert_eq!(id.len(), 36, "{id}");
    assert_eq!(&id[8..9], "-", "{id}");
    assert_eq!(&id[13..14], "-", "{id}");
    assert_eq!(&id[18..19], "-", "{id}");
    assert_eq!(&id[23..24], "-", "{id}");
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    assert_eq!(hex.len(), 32, "{id}");
    assert!(
        hex.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{id}"
    );
    assert_eq!(&id[14..15], "4", "{id}");
    assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
}
