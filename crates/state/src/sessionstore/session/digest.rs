//! Transcript identity digests (Go: save_fast_baseline.go
//! `digestSizeAndPrefix` and friends).
//!
//! A transcript's identity is the SHA-256 over every message marshaled
//! in its `createdAt`-zeroed identity form, each followed by a
//! newline (Go's json.Encoder appends one per Encode). Local display
//! metadata stays out of identity so a saved-and-reloaded message keeps
//! the same digest.

use reasonix_contract::gojson;
use reasonix_contract::provider::Message;
use sha2::{Digest as _, Sha256};

/// Returns the message in its transcript-identity form: identical bytes
/// except `createdAt`, which is display-only metadata excluded
/// from identity (Go: `messageForSessionIdentity`).
pub(crate) fn message_for_session_identity(m: &Message) -> Message {
    let mut m = m.clone();
    m.created_at = 0;
    m
}

/// Digest, encoded size, and the digest of the first `k` messages,
/// in one streaming pass (Go: `digestSizeAndPrefix`).
///
/// `k < 0` skips the prefix (plain digest-and-size), `k == 0`
/// yields the SHA-256 of the empty input. The size counts every encoded
/// byte, including the newline each message contributes.
pub(crate) fn digest_size_and_prefix(
    msgs: &[Message],
    k: i64,
) -> Result<([u8; 32], i64, [u8; 32]), serde_json::Error> {
    let mut h = Sha256::new();
    let mut prefix = [0u8; 32];
    if k == 0 {
        prefix.copy_from_slice(&Sha256::digest(b""));
    }
    let mut size: i64 = 0;
    for (i, msg) in msgs.iter().enumerate() {
        let m = message_for_session_identity(msg);
        let line = gojson::marshal(&m)?;
        h.update(line.as_bytes());
        h.update(b"\n");
        size += line.len() as i64 + 1;
        if i as i64 + 1 == k {
            prefix.copy_from_slice(&h.clone().finalize());
        }
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&h.finalize());
    Ok((digest, size, prefix))
}

/// SHA-256 transcript digest (Go: `DigestSessionMessages`).
pub fn digest_session_messages(msgs: &[Message]) -> Result<[u8; 32], serde_json::Error> {
    Ok(digest_size_and_prefix(msgs, -1)?.0)
}

/// Digest plus encoded size (Go: `digestAndSizeSessionMessages`).
pub fn digest_and_size_session_messages(
    msgs: &[Message],
) -> Result<([u8; 32], i64), serde_json::Error> {
    let (digest, size, _) = digest_size_and_prefix(msgs, -1)?;
    Ok((digest, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::provider::Role;

    fn msg(role: Role, content: &str, created_at: i64) -> Message {
        Message {
            role,
            content: content.to_string(),
            created_at,
            ..Default::default()
        }
    }

    fn sha256_hex(parts: &[&str]) -> String {
        let mut h = Sha256::new();
        for p in parts {
            h.update(p.as_bytes());
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&h.finalize());
        out.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn digest_matches_manual_concat() {
        let m0 = msg(Role::USER, "hello", 3);
        let m1 = msg(Role::ASSISTANT, "hi <there> & back", 7);
        let i0 = message_for_session_identity(&m0);
        let i1 = message_for_session_identity(&m1);
        let l0 = gojson::marshal(&i0).unwrap();
        let l1 = gojson::marshal(&i1).unwrap();

        let (digest, size, _) = digest_size_and_prefix(&[m0.clone(), m1.clone()], -1).unwrap();
        let expect = sha256_hex(&[&l0, "\n", &l1, "\n"]);
        let got = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(got, expect);
        assert_eq!(size, (l0.len() + 1 + l1.len() + 1) as i64);
        assert_eq!(
            digest_session_messages(&[m0.clone(), m1.clone()]).unwrap(),
            digest
        );
        assert_eq!(
            digest_and_size_session_messages(&[m0, m1]).unwrap(),
            (digest, size)
        );
    }

    #[test]
    fn digest_ignores_created_at() {
        let a = msg(Role::USER, "same", 5);
        let b = msg(Role::USER, "same", 0);
        let c = msg(Role::USER, "different", 5);
        assert_eq!(
            digest_session_messages(std::slice::from_ref(&a)).unwrap(),
            digest_session_messages(std::slice::from_ref(&b)).unwrap()
        );
        assert_ne!(
            digest_session_messages(&[a]).unwrap(),
            digest_session_messages(&[c]).unwrap()
        );
    }

    #[test]
    fn prefix_and_size_at_k() {
        let m0 = msg(Role::USER, "one", 1);
        let m1 = msg(Role::ASSISTANT, "two", 2);
        let m2 = msg(Role::TOOL, "three", 3);
        let msgs = [m0.clone(), m1.clone(), m2.clone()];
        let i0 = message_for_session_identity(&m0);
        let l0 = gojson::marshal(&i0).unwrap();

        // k == 0: SHA-256 of the empty input.
        let (digest, _, prefix) = digest_size_and_prefix(&msgs, 0).unwrap();
        assert_eq!(
            prefix
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            sha256_hex(&[""])
        );
        // The prefix stops at zero messages; the digest still covers all of them.
        assert_ne!(prefix.to_vec(), digest.to_vec());

        // k == 1: hash covers only the first message.
        let (digest, _, prefix) = digest_size_and_prefix(&msgs, 1).unwrap();
        assert_eq!(
            prefix
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            sha256_hex(&[&l0, "\n"])
        );
        assert_ne!(prefix.to_vec(), digest.to_vec());

        // k == len: the prefix is the full digest.
        let (digest, _, prefix) = digest_size_and_prefix(&msgs, 3).unwrap();
        assert_eq!(prefix, digest);

        // k < 0: the prefix stays zeroed (never assigned in Go either).
        let (digest, _, prefix) = digest_size_and_prefix(&msgs, -1).unwrap();
        assert_eq!(prefix, [0u8; 32]);
        assert_ne!(digest, [0u8; 32]);

        // Size accounts for every message line plus newlines.
        let i1 = message_for_session_identity(&m1);
        let i2 = message_for_session_identity(&m2);
        let l1 = gojson::marshal(&i1).unwrap();
        let l2 = gojson::marshal(&i2).unwrap();
        let (_, size, _) = digest_size_and_prefix(&msgs, -1).unwrap();
        assert_eq!(size, (l0.len() + 1 + l1.len() + 1 + l2.len() + 1) as i64);
    }
}
