use std::collections::BTreeMap;

use super::disk_tests::support;
use super::{Error, PromptEnvelope, encode_envelope, idempotency_request_hash, sha256_hex};

#[test]
fn envelope_encoding_is_compact_go_json_with_html_escaping() {
    let env = PromptEnvelope {
        display_text: "<tag>&\u{2028}\u{2029}".to_owned(),
        raw_text: "raw".to_owned(),
        submit_text: "submit".to_owned(),
        ..PromptEnvelope::default()
    };
    let (bytes, checksum, size) = encode_envelope(&env).unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        r#"{"displayText":"\u003ctag\u003e\u0026\u2028\u2029","rawText":"raw","submitText":"submit"}"#
    );
    assert_eq!(checksum, sha256_hex(&bytes));
    assert_eq!(size, i64::try_from(bytes.len()).unwrap());
}

#[test]
fn fingerprint_v1_is_accepted_verbatim() {
    let fingerprint = "a".repeat(64);
    let env = PromptEnvelope {
        fingerprint_version: 1,
        request_fingerprint: fingerprint.clone(),
        ..PromptEnvelope::default()
    };
    assert_eq!(idempotency_request_hash(&env).unwrap(), fingerprint);
}

#[test]
fn unsupported_or_malformed_fingerprints_are_rejected() {
    for (version, fingerprint) in [(2, "a".repeat(64)), (1, "not-a-hash".to_owned())] {
        let env = PromptEnvelope {
            fingerprint_version: version,
            request_fingerprint: fingerprint,
            ..PromptEnvelope::default()
        };
        assert!(matches!(
            idempotency_request_hash(&env),
            Err(Error::InvalidRequestFingerprint)
        ));
    }
}

#[test]
fn fallback_fingerprint_sorts_invocations_and_uses_go_field_order() {
    let mut first = support::invocation("first", 2);
    first.args.insert("a".to_owned(), "b".to_owned());
    let mut second = support::invocation("second", 9);
    second.kind = "ignored".to_owned();
    second.args.insert("z".to_owned(), "y".to_owned());
    let env = PromptEnvelope {
        display_text: "d<".to_owned(),
        raw_text: "r".to_owned(),
        submit_text: "s".to_owned(),
        invocations: vec![second, first],
        format: "f".to_owned(),
        attachments: vec!["a".to_owned()],
        attachment_identities: vec!["id".to_owned()],
        explicit_refs: vec!["ref".to_owned()],
        source: "src".to_owned(),
        extra: BTreeMap::from([("k".to_owned(), "v".to_owned())]),
        ..PromptEnvelope::default()
    };
    let stable = r#"{"displayText":"d\u003c","rawText":"r","submitText":"s","invocations":[{"name":"first","args":{"a":"b"},"display":"display first"},{"name":"second","args":{"z":"y"},"display":"display second"}],"format":"f","attachments":["a"],"attachmentIdentities":["id"],"explicitRefs":["ref"],"source":"src","extra":{"k":"v"}}"#;
    assert_eq!(
        idempotency_request_hash(&env).unwrap(),
        sha256_hex(stable.as_bytes())
    );
}

#[test]
fn fallback_fingerprint_uses_the_legacy_singular_invocation() {
    let env = PromptEnvelope {
        display_text: "display".to_owned(),
        invocation: Some(support::invocation("only", 7)),
        ..PromptEnvelope::default()
    };
    let stable = r#"{"displayText":"display","rawText":"","submitText":"","invocations":[{"name":"only","display":"display only"}]}"#;
    assert_eq!(
        idempotency_request_hash(&env).unwrap(),
        sha256_hex(stable.as_bytes())
    );
}

#[test]
fn attachment_identities_change_the_fallback_fingerprint() {
    let mut env = PromptEnvelope {
        display_text: "image".to_owned(),
        attachments: vec!["image.png".to_owned()],
        attachment_identities: vec!["sha:one".to_owned()],
        ..PromptEnvelope::default()
    };
    let first = idempotency_request_hash(&env).unwrap();
    env.attachment_identities[0] = "sha:two".to_owned();
    assert_ne!(first, idempotency_request_hash(&env).unwrap());
}
