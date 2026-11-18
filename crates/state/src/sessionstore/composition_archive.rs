//! The native session archive: a self-contained, versioned export of a
//! saved session (Go: internal/state/sessionstore/composition_archive.go).
//!
//! Parity contract (docs/contracts/session-format.md 7, SESS-012): the
//! envelope is `{"format":"reasonix-native-session","version":2|3,"records":[...]}`.
//! Version 3 marks a live composition binding; the payload may not exceed
//! 192 MiB or 100000 records. Decoding refuses unknown fields at every
//! depth (Go: `json.Decoder` with `DisallowUnknownFields`)
//! through `serde_ignored`, rejects trailing data, and admits a
//! composition binding only when it sits on the leading system message.
//!
//! Known divergences (both stay inside the same error class):
//!
//! * `PrepareSessionPath` (the 1.x v4 import) is not ported yet, so
//!   `load_session` skips it (SESS-023).
//! * On a payload with both an unknown field and a type error, Go reports
//!   the first unknown field in document order; serde ignores the unknown
//!   field and reports the type error first. Both classify as
//!   `invalid native session archive`.

use std::borrow::Cow;

use reasonix_contract::agentcomposition::{Binding, CompositionError, LIVE_BINDING_VERSION};
use reasonix_contract::go_struct;
use reasonix_contract::gojson;
use reasonix_contract::gojson::{go_vec, null_default};
use reasonix_contract::provider::{Message, Role};
use serde::{Serialize, Serializer};
use thiserror::Error;

use super::eventlog::SESSION_EVENT_REPLAY_MAX_MESSAGES;
use super::session::{LoadError, Session, load_session};

/// Archive format marker (Go: NativeArchiveFormat).
pub const NATIVE_ARCHIVE_FORMAT: &str = "reasonix-native-session";
/// Envelope version of a plain transcript archive (Go: NativeArchiveVersion).
pub const NATIVE_ARCHIVE_VERSION: i64 = 2;
/// Envelope version that also carries a live composition binding (Go:
/// NativeArchiveLiveVersion).
pub const NATIVE_ARCHIVE_LIVE_VERSION: i64 = 3;
/// Hard ceiling on the archive envelope (Go: MaxNativeArchiveBytes).
pub const MAX_NATIVE_ARCHIVE_BYTES: usize = 192 << 20;

/// Failures at the archive boundary (Go: the sentinel errors of
/// composition_archive.go plus the load and composition errors it passes
/// through bare).
#[derive(Debug, Error)]
pub enum NativeArchiveError {
    /// Go: ErrSessionReplayLimitExceeded — the envelope exceeds the safe
    /// replay limits.
    #[error("session history exceeds safe replay limits")]
    ReplayLimit,
    /// Go: fmt.Errorf("%w: %w", ErrNativeArchiveInvalid, err) plus the
    /// hand-rolled causes ("trailing data", "no system message").
    #[error("invalid native session archive: {cause}")]
    Invalid {
        /// The wrapped cause text.
        cause: Cow<'static, str>,
    },
    /// Go: ErrNativeArchiveUnsupported, reported bare when a live binding
    /// would ride a legacy (version 2) archive.
    #[error("unsupported native session archive version")]
    Unsupported,
    /// Go: fmt.Errorf("%w: %d", ErrNativeArchiveUnsupported, version).
    #[error("unsupported native session archive version: {version}")]
    UnsupportedVersion {
        /// The envelope version that was rejected.
        version: i64,
    },
    /// The composition validation failure (Go returns the
    /// agentcomposition error bare).
    #[error(transparent)]
    Composition(#[from] CompositionError),
    /// Loading the source session failed (Go: LoadSession's error,
    /// returned bare).
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Marshaling the envelope failed (Go returns the json.Marshal error
    /// bare).
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
}

impl NativeArchiveError {
    /// True when Go answers errors.Is(err, ErrNativeArchiveUnsupported);
    /// the serve layer maps this to the session.archive_unsupported
    /// refusal.
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            NativeArchiveError::Unsupported | NativeArchiveError::UnsupportedVersion { .. }
        )
    }
}

/// Serializes the record slice the way Go marshals the session snapshot:
/// an empty history is a nil slice and renders as `null`.
fn serialize_records<S>(records: &[Message], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if records.is_empty() {
        serializer.serialize_none()
    } else {
        records.serialize(serializer)
    }
}

go_struct! {
    /// The native archive envelope (Go: NativeArchive).
    pub struct NativeArchive : NativeArchiveWire {
        #[serde(rename = "format", default, deserialize_with = "null_default")]
        pub format: String,
        #[serde(rename = "version", default, deserialize_with = "null_default")]
        pub version: i64,
        #[serde(
            rename = "records",
            default,
            deserialize_with = "go_vec",
            serialize_with = "serialize_records"
        )]
        pub records: Vec<Message>,
    }
}

/// Exports a saved session as a native archive (Go: ExportNativeArchive).
///
/// A live composition binding bumps the envelope to version 3 after
/// re-validating the binding; a bound session whose binding uses any other
/// format version is refused.
pub fn export_native_archive(path: &str) -> Result<Vec<u8>, NativeArchiveError> {
    let session = load_session(path)?;
    let mut archive = NativeArchive {
        format: NATIVE_ARCHIVE_FORMAT.to_string(),
        version: NATIVE_ARCHIVE_VERSION,
        records: session.snapshot(),
    };
    if let Some(summary) = session.composition_summary()
        && summary.format_version != 0
    {
        if summary.format_version != LIVE_BINDING_VERSION {
            return Err(NativeArchiveError::Unsupported);
        }
        let binding = session
            .composition()
            .expect("a composition summary implies a bound system message");
        binding
            .validate()
            .map_err(|err| NativeArchiveError::Invalid {
                cause: Cow::Owned(err.to_string()),
            })?;
        archive.version = NATIVE_ARCHIVE_LIVE_VERSION;
    }
    let data = gojson::marshal(&archive)?;
    if data.len() > MAX_NATIVE_ARCHIVE_BYTES {
        return Err(NativeArchiveError::ReplayLimit);
    }
    Ok(data.into_bytes())
}

/// Decodes a native session archive (Go: DecodeNativeArchive).
///
/// Refuses payloads above the replay limits, unknown fields at any depth
/// (Go: DisallowUnknownFields), trailing data, foreign envelope formats,
/// and composition bindings that do not sit sealed on the leading system
/// message.
pub fn decode_native_archive(data: &[u8]) -> Result<Session, NativeArchiveError> {
    if data.len() > MAX_NATIVE_ARCHIVE_BYTES {
        return Err(NativeArchiveError::ReplayLimit);
    }
    let mut de = serde_json::Deserializer::from_slice(data);
    let mut unknown: Option<String> = None;
    let archive: NativeArchive = serde_ignored::deserialize(&mut de, |field| {
        // Go's decoder fails on the first unknown field; the callback
        // cannot abort the parse, so keep the first path and report it
        // once the value finished decoding.
        unknown.get_or_insert_with(|| field.to_string());
    })
    .map_err(|err| NativeArchiveError::Invalid {
        cause: Cow::Owned(err.to_string()),
    })?;
    if let Some(path) = unknown {
        let field = path.rsplit('.').next().unwrap_or(path.as_str());
        return Err(NativeArchiveError::Invalid {
            cause: Cow::Owned(format!("json: unknown field \"{field}\"")),
        });
    }
    de.end().map_err(|_| NativeArchiveError::Invalid {
        cause: Cow::Borrowed("trailing data"),
    })?;
    if archive.format != NATIVE_ARCHIVE_FORMAT
        || (archive.version != NATIVE_ARCHIVE_VERSION
            && archive.version != NATIVE_ARCHIVE_LIVE_VERSION)
    {
        return Err(NativeArchiveError::UnsupportedVersion {
            version: archive.version,
        });
    }
    if archive.records.len() > SESSION_EVENT_REPLAY_MAX_MESSAGES {
        return Err(NativeArchiveError::ReplayLimit);
    }
    let mut binding: Option<Binding> = None;
    for (i, message) in archive.records.iter().enumerate() {
        let Some(candidate) = &message.agent_composition else {
            continue;
        };
        if i != 0 || message.role != Role::SYSTEM || binding.is_some() {
            return Err(NativeArchiveError::Composition(CompositionError::invalid()));
        }
        if candidate.live() && archive.version != NATIVE_ARCHIVE_LIVE_VERSION {
            return Err(NativeArchiveError::Unsupported);
        }
        candidate.validate()?;
        binding = Some(candidate.clone());
    }
    let session = Session::from_archive_records(archive.records);
    if !session.has_system_message() {
        return Err(NativeArchiveError::Invalid {
            cause: Cow::Borrowed("no system message"),
        });
    }
    if let Some(binding) = binding.as_ref()
        && session.has_content()
        && !binding.started
    {
        return Err(NativeArchiveError::Composition(
            CompositionError::invalid_detail("admitted archive is unlocked"),
        ));
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::agentcomposition::{
        LIVE_RESOURCES, Module, Resource, SELECT, Selection, Version, builtins, revision,
    };
    use serde_json::value::RawValue;

    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::store::naming::session_event_log;

    fn sealed_builtin_binding(index: usize, live: bool) -> Binding {
        let d = builtins()
            .into_iter()
            .nth(index)
            .unwrap_or_else(|| panic!("builtin definition {index} exists"));
        let mut binding = Binding {
            resolved: d.clone(),
            version: Version {
                definition: d.clone(),
                revision: revision(&d),
            },
            ..Binding::default()
        };
        if live {
            binding.format_version = LIVE_BINDING_VERSION;
            binding.resource_mode = LIVE_RESOURCES.to_string();
            binding.resources = Some(Vec::new());
        } else {
            binding.resources = Some(vec![Resource {
                id: "retained".to_string(),
                kind: "command".to_string(),
                data: Some(
                    RawValue::from_string(r#"{"body":"immutable source"}"#.to_string())
                        .expect("raw resource data"),
                ),
            }]);
        }
        binding.seal();
        binding
    }

    #[test]
    fn native_archive_round_trips_a_bound_composition() {
        let dir = tempdir();
        let path = dir.path().join("bound.jsonl");
        let path_str = path.to_str().unwrap();
        let binding = sealed_builtin_binding(1, false);

        let session = Session::new("original prompt");
        session.bind_composition(&binding);
        session.add(Message {
            role: Role::USER,
            content: "run it".to_string(),
            ..Message::default()
        });
        session.save(path_str).expect("save");

        let data = export_native_archive(path_str).expect("export");
        let envelope: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(envelope["format"], NATIVE_ARCHIVE_FORMAT);
        assert_eq!(envelope["version"], 2);
        let log = std::fs::read_to_string(session_event_log(&path)).unwrap();
        assert!(
            log.contains("schema_version\":4"),
            "a bound save pins event schema 4"
        );

        let decoded = decode_native_archive(&data).expect("decode");
        let round = decoded
            .composition()
            .expect("the binding survives the round trip");
        assert_eq!(round.digest, binding.digest);
        assert!(round.started, "the archived binding keeps its started flag");
        let resource = round
            .resources
            .as_deref()
            .and_then(|r| r.first())
            .expect("the retained resource survives");
        assert!(
            resource
                .data
                .as_deref()
                .expect("raw resource data")
                .get()
                .contains("immutable source")
        );

        // The decoded session persists once; a second import collides.
        let dest = dir.path().join("imported.jsonl");
        let dest_str = dest.to_str().unwrap();
        decoded.save_if_absent(dest_str).expect("save_if_absent");
        let again = decode_native_archive(&data).unwrap();
        let err = again.save_if_absent(dest_str).unwrap_err();
        assert_eq!(err.to_string(), "file already exists");

        // A tampered envelope version is refused (Go: bytes.Replace 2 -> 999).
        let tampered = String::from_utf8(data)
            .unwrap()
            .replace("\"version\":2", "\"version\":999");
        let err = decode_native_archive(tampered.as_bytes()).unwrap_err();
        assert!(matches!(
            err,
            NativeArchiveError::UnsupportedVersion { version: 999 }
        ));
        assert_eq!(
            err.to_string(),
            "unsupported native session archive version: 999"
        );
    }

    #[test]
    fn native_archive_live_binding_uses_version_three() {
        let dir = tempdir();
        let path = dir.path().join("live.jsonl");
        let path_str = path.to_str().unwrap();
        let binding = sealed_builtin_binding(0, true);

        let session = Session::new("live prompt");
        session.bind_composition(&binding);
        session.add(Message {
            role: Role::USER,
            content: "go".to_string(),
            ..Message::default()
        });
        session.save(path_str).expect("save");

        let summary = session.composition_summary().expect("summary");
        assert_eq!(summary.resource_mode, "live");
        let log = std::fs::read_to_string(session_event_log(&path)).unwrap();
        assert!(
            log.contains("schema_version\":5"),
            "a live save pins event schema 5"
        );

        let data = export_native_archive(path_str).expect("export");
        let envelope: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(envelope["version"], 3);
        let decoded = decode_native_archive(&data).expect("decode");
        assert!(decoded.composition().unwrap().live());

        // Downgrade attack (Go: composition_live_test.go): a live binding
        // must not ride a legacy envelope.
        let downgraded = String::from_utf8(data)
            .unwrap()
            .replace("\"version\":3", "\"version\":2");
        let err = decode_native_archive(downgraded.as_bytes()).unwrap_err();
        assert!(err.is_unsupported());
        assert_eq!(
            err.to_string(),
            "unsupported native session archive version"
        );
    }

    #[test]
    fn native_archive_rejects_unknown_fields_and_trailing_data() {
        // Unknown top-level field (Go: decoder.DisallowUnknownFields).
        let raw = b"{\"format\":\"reasonix-native-session\",\"version\":2,\"rogue\":1}";
        let err = decode_native_archive(raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid native session archive: json: unknown field \"rogue\""
        );

        // Unknown field inside a record: the refusal is tree-wide.
        let raw = b"{\"format\":\"reasonix-native-session\",\"version\":2,\"records\":[{\"role\":\"user\",\"content\":\"hi\",\"rogue_key\":1}]}";
        let err = decode_native_archive(raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid native session archive: json: unknown field \"rogue_key\""
        );

        // Trailing data after the envelope (Go: the second Decode must
        // report io.EOF).
        let raw = b"{\"format\":\"reasonix-native-session\",\"version\":2} trailing";
        let err = decode_native_archive(raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid native session archive: trailing data"
        );
    }

    #[test]
    fn native_archive_rejects_malformed_envelopes() {
        // A foreign format marker reports the version (Go wraps it with
        // ErrNativeArchiveUnsupported).
        let raw = gojson::marshal(&NativeArchive {
            format: "other".to_string(),
            version: 2,
            records: Vec::new(),
        })
        .unwrap()
        .into_bytes();
        let err = decode_native_archive(&raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "unsupported native session archive version: 2"
        );

        // records: null decodes to an empty history (a Go nil slice).
        let raw = serde_json::json!({
            "format": NATIVE_ARCHIVE_FORMAT,
            "version": 2,
            "records": null,
        })
        .to_string()
        .into_bytes();
        let err = decode_native_archive(&raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid native session archive: no system message"
        );

        // A binding outside the leading system message is never admitted.
        let raw = gojson::marshal(&NativeArchive {
            format: NATIVE_ARCHIVE_FORMAT.to_string(),
            version: 2,
            records: vec![Message {
                role: Role::USER,
                content: "hijack".to_string(),
                agent_composition: Some(sealed_builtin_binding(1, false)),
                ..Message::default()
            }],
        })
        .unwrap()
        .into_bytes();
        let err = decode_native_archive(&raw).unwrap_err();
        assert_eq!(err.to_string(), "invalid agent composition");

        // An unlocked binding with content is refused (Go: "admitted
        // archive is unlocked").
        let raw = gojson::marshal(&NativeArchive {
            format: NATIVE_ARCHIVE_FORMAT.to_string(),
            version: 2,
            records: vec![
                Message {
                    role: Role::SYSTEM,
                    content: "sys".to_string(),
                    agent_composition: Some(sealed_builtin_binding(1, false)),
                    ..Message::default()
                },
                Message {
                    role: Role::USER,
                    content: "go".to_string(),
                    ..Message::default()
                },
            ],
        })
        .unwrap()
        .into_bytes();
        let err = decode_native_archive(&raw).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid agent composition: admitted archive is unlocked"
        );
    }

    #[test]
    fn native_archive_import_keeps_a_project_history_binding() {
        let dir = tempdir();
        let path = dir.path().join("history.jsonl");
        let path_str = path.to_str().unwrap();

        let mut d = builtins()
            .into_iter()
            .next()
            .expect("builtin:general exists");
        d.id = "project:legacy-launch".to_string();
        d.mcp = Selection {
            mode: SELECT.to_string(),
            members: Some(vec![Module {
                id: "authorized".to_string(),
                parameters: Some(
                    RawValue::from_string(r#"{"url":"https://other.invalid/mcp"}"#.to_string())
                        .expect("raw parameters"),
                ),
                ..Module::default()
            }]),
        };
        let mut binding = Binding {
            resolved: d.clone(),
            version: Version {
                definition: d.clone(),
                revision: revision(&d),
            },
            ..Binding::default()
        };
        binding.seal();

        let session = Session::new("retained prompt");
        session.bind_composition(&binding);
        session.add(Message {
            role: Role::USER,
            content: "history remains visible".to_string(),
            ..Message::default()
        });
        session.save(path_str).expect("save");

        let loaded = load_session(path_str).expect("load");
        let composition = loaded.composition().expect("binding survives the load");
        assert_eq!(composition.digest, binding.digest);
        assert!(
            composition
                .validate_for_execution()
                .unwrap_err()
                .is_invalid(),
            "a project module may not override a server url"
        );

        let data = export_native_archive(path_str).expect("export");
        let envelope: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(envelope["version"], 2, "history bindings are not live");

        let imported = decode_native_archive(&data).expect("decode");
        let round = imported
            .composition()
            .expect("binding survives the archive");
        assert_eq!(round.digest, binding.digest);
        assert!(
            round.validate_for_execution().unwrap_err().is_invalid(),
            "an imported archive grants no execution authority"
        );
    }

    #[test]
    fn native_archive_exports_empty_sessions_as_null_records() {
        let dir = tempdir();
        let path = dir.path().join("empty.jsonl");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, b"").expect("empty transcript");

        let data = export_native_archive(path_str).expect("export");
        let envelope = String::from_utf8(data.clone()).unwrap();
        assert!(
            envelope.contains("\"records\":null"),
            "an empty session exports nil records"
        );

        let err = decode_native_archive(&data).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid native session archive: no system message"
        );
        assert_eq!(MAX_NATIVE_ARCHIVE_BYTES, 192 << 20);
    }
}
