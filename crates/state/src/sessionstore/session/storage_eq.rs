//! Storage-identity comparators (Go: save.go).
//!
//! Two messages compare equal for storage when their identity forms
//! marshal identically. These comparators drive the append/rewrite
//! decisions in the save pipeline: a save may append only when the new
//! transcript is a prefix-compatible extension of the persisted one.

use sha2::{Digest as _, Sha256};

use reasonix_contract::gojson;
use reasonix_contract::provider::{Message, Role};

use super::digest::message_for_session_identity;

/// Byte-equality of the identity forms (Go: `messagesEqualForStorage`).
pub(crate) fn messages_equal_for_storage(a: &Message, b: &Message) -> bool {
    let a = message_for_session_identity(a);
    let b = message_for_session_identity(b);
    match (gojson::marshal(&a), gojson::marshal(&b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// List variant (Go: `messagesEqualForStorageList`).
pub(crate) fn messages_equal_for_storage_list(a: &[Message], b: &[Message]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| messages_equal_for_storage(x, y))
}

/// Borrows the slice minus a leading system message if present (Go:
/// `messagesWithoutLeadingSystem`).
pub(crate) fn messages_without_leading_system(msgs: &[Message]) -> &[Message] {
    if !msgs.is_empty() && msgs[0].role == Role::SYSTEM {
        &msgs[1..]
    } else {
        msgs
    }
}

/// Equality modulo a swapped-in system prompt (Go:
/// `messagesCompatibleForStorageBaseline`).
pub(crate) fn messages_compatible_for_storage_baseline(a: &[Message], b: &[Message]) -> bool {
    messages_equal_for_storage_list(a, b)
        || messages_equal_for_storage_list(
            messages_without_leading_system(a),
            messages_without_leading_system(b),
        )
}

/// Prefix test under storage equality (Go: `messagesHavePrefix`).
pub(crate) fn messages_have_prefix(full: &[Message], prefix: &[Message]) -> bool {
    if prefix.len() > full.len() {
        return false;
    }
    prefix
        .iter()
        .zip(full)
        .all(|(p, f)| messages_equal_for_storage(p, f))
}

/// Prefix test that tolerates a different leading system message (Go:
/// `messagesHavePrefixWithCompatibleSystem`).
#[allow(dead_code)] // wired by the save-pipeline slice (SESS-016+)
pub(crate) fn messages_have_prefix_with_compatible_system(
    full: &[Message],
    prefix: &[Message],
) -> bool {
    messages_have_prefix(
        messages_without_leading_system(full),
        messages_without_leading_system(prefix),
    )
}

/// How many leading messages of `msgs` hash to `target` (Go:
/// `messagesPrefixDigestDepth`); `-1` when none do.
pub(crate) fn messages_prefix_digest_depth(msgs: &[Message], target: &[u8; 32]) -> i64 {
    let mut h = Sha256::new();
    for (i, msg) in msgs.iter().enumerate() {
        let m = message_for_session_identity(msg);
        let Ok(line) = gojson::marshal(&m) else {
            return -1;
        };
        h.update(line.as_bytes());
        h.update(b"\n");
        let sum = h.clone().finalize();
        let mut candidate = [0u8; 32];
        candidate.copy_from_slice(&sum);
        if candidate == *target {
            return i as i64 + 1;
        }
    }
    -1
}

/// Whether appending `next` to a transcript whose persisted
/// baseline is `existing`/`base_digest` covers the baseline
/// (Go: `appendCoversPersistedBaseline`).
///
/// When the plain prefix digest misses but both transcripts lead with
/// differing system messages, the check retries with the existing system
/// message spliced in front of `next` (a system swap alone must
/// not force a rewrite).
#[allow(dead_code)] // wired by the save-pipeline slice (SESS-016+)
pub(crate) fn append_covers_persisted_baseline(
    next: &[Message],
    existing: &[Message],
    base_digest: &[u8; 32],
) -> bool {
    let mut depth = messages_prefix_digest_depth(next, base_digest);
    if depth < 0
        && !next.is_empty()
        && !existing.is_empty()
        && next[0].role == Role::SYSTEM
        && existing[0].role == Role::SYSTEM
        && !messages_equal_for_storage(&next[0], &existing[0])
    {
        let mut variant = Vec::with_capacity(next.len());
        variant.push(existing[0].clone());
        variant.extend_from_slice(&next[1..]);
        depth = messages_prefix_digest_depth(&variant, base_digest);
    }
    depth >= 0 && existing.len() as i64 >= depth
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

    #[test]
    fn storage_equality_matrix() {
        let a = msg(Role::USER, "hello", 1);
        let b = msg(Role::USER, "hello", 99);
        let c = msg(Role::USER, "world", 1);
        assert!(messages_equal_for_storage(&a, &b)); // createdAt ignored.
        assert!(!messages_equal_for_storage(&a, &c));
        assert!(messages_equal_for_storage_list(
            &[a.clone(), c.clone()],
            &[b.clone(), c.clone()]
        ));
        assert!(!messages_equal_for_storage_list(
            std::slice::from_ref(&a),
            &[b.clone(), c.clone()]
        ));
    }

    #[test]
    fn leading_system_tolerances() {
        let system_old = msg(Role::SYSTEM, "old prompt", 0);
        let system_new = msg(Role::SYSTEM, "new prompt", 0);
        let u = msg(Role::USER, "hi", 0);
        let a = msg(Role::ASSISTANT, "there", 0);

        assert!(messages_equal_for_storage_list(
            messages_without_leading_system(&[system_old.clone(), u.clone()]),
            std::slice::from_ref(&u),
        ));
        assert!(messages_equal_for_storage_list(
            messages_without_leading_system(std::slice::from_ref(&u)),
            std::slice::from_ref(&u),
        ));
        assert!(messages_equal_for_storage_list(
            messages_without_leading_system(&[]),
            &[],
        ));

        // Compatible baseline tolerates a swapped system message.
        assert!(messages_compatible_for_storage_baseline(
            &[system_old.clone(), u.clone(), a.clone()],
            &[system_new.clone(), u.clone(), a.clone()],
        ));
        // But not a differing tail.
        assert!(!messages_compatible_for_storage_baseline(
            &[system_old.clone(), u.clone()],
            &[system_new.clone(), u.clone(), a.clone()],
        ));
    }

    #[test]
    fn prefix_checks() {
        let u = msg(Role::USER, "one", 0);
        let a = msg(Role::ASSISTANT, "two", 0);
        let full = [u.clone(), a.clone()];
        assert!(messages_have_prefix(&full, std::slice::from_ref(&u)));
        assert!(messages_have_prefix(&full, &full));
        assert!(!messages_have_prefix(&full, std::slice::from_ref(&a)));
        assert!(!messages_have_prefix(
            &full,
            &[u.clone(), a.clone(), u.clone()]
        ));

        let system_old = msg(Role::SYSTEM, "old", 0);
        let system_new = msg(Role::SYSTEM, "new", 0);
        let with_old = [system_old, u.clone(), a.clone()];
        let with_new = [system_new.clone(), u.clone(), a.clone()];
        assert!(messages_have_prefix_with_compatible_system(
            &with_old, &with_new
        ));
        assert!(messages_have_prefix_with_compatible_system(
            &with_old,
            &[u.clone(), a.clone()]
        ));
        assert!(!messages_have_prefix_with_compatible_system(
            &with_old,
            &[system_new, msg(Role::USER, "different", 0)],
        ));
    }

    #[test]
    fn prefix_digest_depth_and_append_covers() {
        let u = msg(Role::USER, "one", 0);
        let a = msg(Role::ASSISTANT, "two", 0);
        let t = msg(Role::TOOL, "three", 0);
        let base = [u.clone(), a.clone()];
        let full_digest = super::super::digest::digest_session_messages(&base).unwrap();
        let prefix_digest =
            super::super::digest::digest_session_messages(std::slice::from_ref(&u)).unwrap();

        assert_eq!(messages_prefix_digest_depth(&base, &full_digest), 2);
        assert_eq!(messages_prefix_digest_depth(&base, &prefix_digest), 1);
        assert_eq!(
            messages_prefix_digest_depth(&[u.clone(), t.clone()], &full_digest),
            -1
        );

        // A pure extension covers the baseline.
        let next = [u.clone(), a.clone(), t.clone()];
        assert!(append_covers_persisted_baseline(&next, &base, &full_digest));
        // A diverging tail does not.
        let next_bad = [u.clone(), t.clone()];
        assert!(!append_covers_persisted_baseline(
            &next_bad,
            &base,
            &full_digest
        ));

        // A system swap alone still covers: the spliced variant matches.
        let system_old = msg(Role::SYSTEM, "old", 0);
        let system_new = msg(Role::SYSTEM, "new", 0);
        let existing = [system_old, u.clone(), a.clone()];
        let existing_digest = super::super::digest::digest_session_messages(&existing).unwrap();
        let next_swapped = [system_new, u.clone(), a.clone(), t.clone()];
        assert!(append_covers_persisted_baseline(
            &next_swapped,
            &existing,
            &existing_digest
        ));
        // Without the system-swap escape the same input would miss.
        assert!(!messages_have_prefix(&next_swapped, &existing));
    }
}
