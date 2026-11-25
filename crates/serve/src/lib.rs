//! The HTTP/SSE loopback frontend.
//!
//! Parity target: `reference/.../internal/frontend/serve` — the loopback gate,
//! the `Reason` refusal envelope, the event broadcaster and its SSE transport,
//! the listener and page resolution, and the hub read-side routes.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod auth;
pub mod boot;
pub mod broadcaster;
pub mod commands;
pub mod eventstream;
pub mod gate;
pub mod listen;
pub mod page;
pub mod password;
pub mod refuse;
pub mod router;
pub mod sink;

// Host wiring surface (docs/contract.md S3): the four calls a host needs to
// open the loopback control plane.
pub use gate::{listen_loopback, loopback_origin, to_tokio_listener};
pub use listen::run_graceful_handler;
pub use page::find_page;

/// Shared scratch space for tests. Test temp directories live under the
/// workspace target because the system temp path is denied in some
/// environments (same approach as the session crate's tests).
#[cfg(test)]
pub(crate) mod test_support {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};

    use base64::Engine;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    #[allow(dead_code)]
    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpserve")
            .tempdir_in(&base)
            .expect("create serve test temp dir")
    }

    pub(crate) fn write_v4_fixture(dir: &Path, id: &str, events: Vec<Value>) -> PathBuf {
        let root = reasonix_state::store::naming::v4_sessions_root(dir);
        let session = root.join(id);
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(
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
        std::fs::write(session.join("events.frames"), bytes).unwrap();
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
