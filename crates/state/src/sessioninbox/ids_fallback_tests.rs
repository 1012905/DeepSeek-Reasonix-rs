use super::*;

#[test]
fn fallback_id_hashes_array_text_and_returns_the_first_sixteen_digest_bytes() {
    let bytes = [0u8; 16];
    let expected = sha256_hex(format!("fallback-{:p}", &bytes).as_bytes());

    let id = super::ids::fallback_random_id(&bytes);

    assert_eq!(id, expected[..32]);
    assert_eq!(id.len(), 32);
    assert!(!id.contains('-'));
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn entropy_failure_returns_the_exact_fallback_before_uuid_formatting() {
    let bytes = [0x5a; 16];
    let fallback_calls = std::cell::Cell::new(0);
    let id = super::ids::random_id_with(
        |buffer| {
            *buffer = bytes;
            false
        },
        |buffer| {
            assert_eq!(buffer, &bytes);
            fallback_calls.set(fallback_calls.get() + 1);
            "injected fallback".to_owned()
        },
    );

    assert_eq!(id, "injected fallback");
    assert_eq!(fallback_calls.get(), 1);
}
