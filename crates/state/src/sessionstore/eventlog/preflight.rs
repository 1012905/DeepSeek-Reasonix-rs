//! Message-payload budget walk (session_events_preflight.go).
//!
//! Counts top-level messages and nested collection items before any
//! message is decoded, so oversized payloads are refused whole instead
//! of partially replayed. The walk runs only after the payload proved
//! to be valid JSON, and it is independent of the message field list:
//! future slice fields inherit the same object-graph bound.

use serde::de::IgnoredAny;

use super::{DecodeError, SessionReplayLimits, session_replay_limit_error};

/// Counts produced by the walk: per-payload messages and cumulative
/// collection items (the item counter starts from the replay's running
/// total).
#[derive(Debug)]
pub(crate) struct PreflightCounts {
    pub message_count: usize,
    pub collection_items: usize,
}

/// Walks a raw messages payload, refusing anything past the budgets
/// (Go: preflightSessionEventMessages).
pub(crate) fn preflight_session_event_messages(
    path: &str,
    raw: &[u8],
    existing_messages: usize,
    existing_collection_items: usize,
    limits: &SessionReplayLimits,
) -> Result<PreflightCounts, DecodeError> {
    // Go gates the walk on json.Valid first; an invalid payload fails
    // exactly like a real Unmarshal would, which replay reports as
    // damage.
    serde_json::from_slice::<IgnoredAny>(raw)
        .map_err(|e| DecodeError::Other(format!("invalid messages payload: {e}")))?;
    let raw = trim_ascii(raw);
    if raw.first() != Some(&b'[') {
        return Err(DecodeError::Other("messages must be an array".to_owned()));
    }
    let mut message_count = 0usize;
    let mut collection_items = existing_collection_items;
    let mut containers: Vec<u8> = Vec::with_capacity(64);
    let mut offset = 0usize;
    while offset < raw.len() {
        match raw[offset] {
            b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' => {
                offset += 1;
                continue;
            }
            b']' | b'}' => {
                containers.pop();
                offset += 1;
                continue;
            }
            _ => {}
        }
        if containers.last() == Some(&b'[') {
            if containers.len() == 1 {
                if existing_messages + message_count >= limits.max_messages {
                    return Err(DecodeError::Limit(session_replay_limit_error(
                        path,
                        "messages",
                        (existing_messages + message_count + 1) as i64,
                        limits.max_messages as i64,
                    )));
                }
                message_count += 1;
            } else {
                if collection_items >= limits.max_collection_items {
                    return Err(DecodeError::Limit(session_replay_limit_error(
                        path,
                        "message_collection_items",
                        (collection_items + 1) as i64,
                        limits.max_collection_items as i64,
                    )));
                }
                collection_items += 1;
            }
        }
        if raw[offset] == b'[' || raw[offset] == b'{' {
            containers.push(raw[offset]);
            offset += 1;
        } else {
            offset = preflight_session_event_value(raw, offset);
        }
    }
    Ok(PreflightCounts {
        message_count,
        collection_items,
    })
}

/// Skips one scalar or string value, returning the offset just past it
/// (Go: preflightSessionEventValue).
fn preflight_session_event_value(raw: &[u8], mut offset: usize) -> usize {
    if raw.get(offset) == Some(&b'"') {
        offset += 1;
        loop {
            match raw.get(offset) {
                None => return raw.len(),
                Some(b'\\') => offset += 2,
                Some(b'"') => return offset + 1,
                Some(_) => offset += 1,
            }
        }
    }
    while offset < raw.len() {
        match raw[offset] {
            b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}' => return offset,
            _ => {}
        }
        offset += 1;
    }
    offset
}

/// ASCII whitespace trim (Go: bytes.TrimSpace on the payload).
pub(crate) fn trim_ascii(mut raw: &[u8]) -> &[u8] {
    while matches!(raw.first(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        raw = &raw[1..];
    }
    while matches!(raw.last(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        raw = &raw[..raw.len() - 1];
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> SessionReplayLimits {
        SessionReplayLimits::default()
    }

    #[test]
    fn counts_messages_and_collection_items() {
        let counts = preflight_session_event_messages(
            "log",
            br#"[{"role":"user","tool_calls":[{"a":1}]}]"#,
            0,
            0,
            &limits(),
        )
        .unwrap();
        assert_eq!(counts.message_count, 1);
        // The tool_calls array contents count as collection items.
        assert_eq!(counts.collection_items, 1);
    }

    #[test]
    fn nested_arrays_count_per_element() {
        let counts =
            preflight_session_event_messages("log", b"[[1],[2],[3]]", 0, 0, &limits()).unwrap();
        assert_eq!(counts.message_count, 3);
        assert_eq!(counts.collection_items, 3);
    }

    #[test]
    fn object_values_below_arrays_count() {
        let counts =
            preflight_session_event_messages("log", br#"[{"a":[1,2]}]"#, 0, 0, &limits()).unwrap();
        assert_eq!(counts.message_count, 1);
        assert_eq!(counts.collection_items, 2);
    }

    #[test]
    fn message_budget_refuses_cumulatively() {
        let mut l = limits();
        l.max_messages = 2;
        let err = preflight_session_event_messages("log", b"[{},{}]", 1, 0, &l).unwrap_err();
        match err {
            DecodeError::Limit(e) => {
                assert_eq!(e.resource, "messages");
                assert_eq!(e.value, 3);
                assert_eq!(e.limit, 2);
            }
            DecodeError::Other(msg) => panic!("{msg}"),
        }
    }

    #[test]
    fn collection_budget_refuses_cumulatively() {
        let mut l = limits();
        l.max_collection_items = 1;
        let err = preflight_session_event_messages("log", b"[[1,2]]", 0, 0, &l).unwrap_err();
        match err {
            DecodeError::Limit(e) => {
                assert_eq!(e.resource, "message_collection_items");
                assert_eq!(e.value, 2);
            }
            DecodeError::Other(msg) => panic!("{msg}"),
        }
    }

    #[test]
    fn non_array_payload_is_rejected() {
        for raw in [
            br#"{"a":1}"#.as_slice(),
            br#""x""#.as_slice(),
            b"null".as_slice(),
        ] {
            match preflight_session_event_messages("log", raw, 0, 0, &limits()) {
                Err(DecodeError::Other(msg)) => {
                    assert_eq!(msg, "messages must be an array", "{raw:?}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn invalid_json_is_rejected() {
        let err = preflight_session_event_messages("log", b"[{", 0, 0, &limits()).unwrap_err();
        match err {
            DecodeError::Other(msg) => assert!(msg.starts_with("invalid messages payload: ")),
            other => panic!("{other:?}"),
        }
    }
}
