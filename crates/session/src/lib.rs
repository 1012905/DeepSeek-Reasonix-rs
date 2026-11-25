//! Hub read side: the remembered workspace list, the workspace tree, and the
//! session listings both frontends render, plus the controller write side
//! that starts and steers turns.
//!
//! Parity target: `reference/.../internal/state/workspacelist` plus
//! `internal/frontend/serve` (`hub_tree.go`, `sessions_list.go`, `hub.go`
//! Hub — its Open/publish subset included), and `internal/session/control`'s
//! turn group. The open machine builds runtimes from a typed [`hub::RuntimeSpec`]
//! and an installed kit factory; hubs without one answer open requests with a
//! typed refusal instead of a fake success (docs/contract.md §3, §4.1). Wire
//! shapes are frozen in docs/contracts/http.md §1.2/§7.

#![forbid(unsafe_code)]

pub mod controller;
pub mod fanout;
pub mod hub;
pub mod listing;
pub mod model;
pub mod paths;
pub mod title;
pub mod tree;
pub mod turns;
pub mod workspacelist;

/// Shared scratch space for tests. The workspace target directory is used
/// instead of `env::temp_dir()` because some test environments deny
/// creating directories under the system temp path (same approach as the
/// state crate's eventlog tests).
#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use base64::Engine;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpsession")
            .tempdir_in(&base)
            .expect("create session test temp dir")
    }

    pub(crate) fn write_v4_fixture(dir: &Path, id: &str, events: Vec<Value>) -> PathBuf {
        let root = reasonix_state::store::naming::v4_sessions_root(dir);
        let session = root.join(id);
        fs::create_dir_all(&session).unwrap();
        fs::write(
            session.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 4,
                "codec": "reasonix.session.linear/v4",
                "storageRevision": 3,
                "contentRoot": "../.content-v1",
                "sessionId": id,
                "createdAt": "2026-09-25T12:00:00Z"
            }))
            .unwrap(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        let mut digest = Sha256::new();
        let begin = json!({
            "schemaVersion": 4,
            "codec": "reasonix.session.linear/v4",
            "recordType": "batch/begin",
            "commitId": "c1",
            "firstSeq": 1,
            "eventCount": events.len()
        });
        append_v4_record(&mut bytes, &begin, &mut digest);
        for (index, event) in events.into_iter().enumerate() {
            let record = json!({
                "schemaVersion": 4,
                "codec": "reasonix.session.linear/v4",
                "recordType": "batch/event",
                "event": {
                    "id": format!("e{}", index + 1),
                    "seq": index + 1,
                    "kind": event["kind"],
                    "payload": base64::engine::general_purpose::STANDARD.encode(
                        event["payload"].to_string()
                    ),
                    "payloadRef": Value::Null
                }
            });
            append_v4_record(&mut bytes, &record, &mut digest);
        }
        let end = json!({
            "schemaVersion": 4,
            "codec": "reasonix.session.linear/v4",
            "recordType": "batch/end",
            "commitId": "c1",
            "sha256": hex(&digest.finalize())
        });
        let raw = serde_json::to_vec(&end).unwrap();
        let compressed = ruzstd::encoding::compress_to_vec(
            Cursor::new(&raw),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        append_frame(&mut bytes, &raw, &compressed);
        fs::write(session.join("events.frames"), bytes).unwrap();
        reasonix_state::sessionstore::v4_import::v4_import_path(dir, id)
    }

    fn append_v4_record(bytes: &mut Vec<u8>, record: &Value, digest: &mut Sha256) {
        let raw = serde_json::to_vec(record).unwrap();
        digest.update(&raw);
        digest.update([0]);
        let compressed = ruzstd::encoding::compress_to_vec(
            Cursor::new(&raw),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        append_frame(bytes, &raw, &compressed);
    }

    fn append_frame(bytes: &mut Vec<u8>, raw: &[u8], compressed: &[u8]) {
        bytes.extend(b"RX4F");
        bytes.extend((compressed.len() as u32).to_be_bytes());
        bytes.extend((raw.len() as u32).to_be_bytes());
        bytes.extend(compressed);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
