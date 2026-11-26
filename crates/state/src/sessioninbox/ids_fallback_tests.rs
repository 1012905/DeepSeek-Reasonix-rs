use super::*;

#[test]
fn fallback_id_hashes_pointer_text_and_returns_the_first_sixteen_digest_bytes() {
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
fn entropy_failure_returns_before_applying_the_uuid_v4_layout() {
    let id = super::ids::random_id_with(|_| false);

    assert_eq!(id.len(), 32);
    assert!(!id.contains('-'));
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}
