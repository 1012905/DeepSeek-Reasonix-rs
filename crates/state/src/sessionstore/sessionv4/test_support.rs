use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub const ID: &str = "0123456789abcdef0123456789abcdef";
pub const CODEC: &str = "reasonix.session.linear/v4";

pub struct Fixture {
    pub temp: TempDir,
    pub root: PathBuf,
    pub dir: PathBuf,
    pub next_seq: u64,
}

impl Fixture {
    pub fn new(id: &str) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("sessions-v4");
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("session dir");
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": 4,
                "codec": CODEC,
                "storageRevision": 3,
                "contentRoot": "../.content-v1",
                "sessionId": id,
                "createdAt": "2026-09-25T12:00:00Z"
            }))
            .expect("manifest"),
        )
        .expect("write manifest");
        Self {
            temp,
            root,
            dir,
            next_seq: 1,
        }
    }

    pub fn sessions(&self) -> PathBuf {
        self.temp.path().join("sessions")
    }

    pub fn batch(&mut self, events: Vec<Value>, end: End) {
        let first = self.next_seq;
        let event_count = events.len() as u64;
        let mut digest = Sha256::new();
        let mut bytes = Vec::new();
        let begin = json!({
            "schemaVersion": 4,
            "codec": CODEC,
            "recordType": "batch/begin",
            "commitId": format!("c{first}"),
            "firstSeq": first,
            "eventCount": events.len()
        });
        append_record(&mut bytes, &begin, true, &mut digest);
        for (index, event) in events.into_iter().enumerate() {
            let record = json!({
                "schemaVersion": 4,
                "codec": CODEC,
                "recordType": "batch/event",
                "event": {
                    "id": format!("e{}", first + index as u64),
                    "seq": first + index as u64,
                    "kind": event["kind"],
                    "payload": base64::engine::general_purpose::STANDARD.encode(
                        event.get("payload").cloned().unwrap_or(Value::Null).to_string().as_bytes()
                    ),
                    "payloadRef": event.get("payloadRef").cloned().unwrap_or(Value::Null)
                }
            });
            append_record(&mut bytes, &record, true, &mut digest);
        }
        if end != End::Torn {
            let sum = if end == End::BadSum {
                "00".repeat(32)
            } else {
                hex(&digest.clone().finalize())
            };
            let record = json!({
                "schemaVersion": 4,
                "codec": CODEC,
                "recordType": "batch/end",
                "commitId": format!("c{first}"),
                "sha256": sum
            });
            append_record(&mut bytes, &record, false, &mut digest);
            self.next_seq = first + event_count;
        }
        let path = self.dir.join("events.frames");
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open frames");
        file.write_all(&bytes).expect("append frames");
    }

    pub fn object(&self, data: &[u8], media_type: &str) -> Value {
        let digest = hex(&Sha256::digest(data));
        let path = self
            .root
            .join(".content-v1/objects")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(&digest);
        fs::create_dir_all(path.parent().expect("parent")).expect("pool dirs");
        fs::write(path, data).expect("pool object");
        json!({"digest": digest, "bytes": data.len(), "mediaType": media_type})
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum End {
    Ended,
    Torn,
    BadSum,
}

pub fn message(id: &str, role: &str, content: &str) -> Value {
    json!({"kind": "message/complete", "payload": {"message": {"id": id, "role": role, "content": content}}})
}

fn append_record(out: &mut Vec<u8>, record: &Value, hashed: bool, digest: &mut Sha256) {
    let raw = serde_json::to_vec(record).expect("record json");
    if hashed {
        digest.update(&raw);
        digest.update([0]);
    }
    out.extend(frame(&raw));
}

pub fn frame(raw: &[u8]) -> Vec<u8> {
    let compressed = ruzstd::encoding::compress_to_vec(
        Cursor::new(raw),
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    let mut out = b"RX4F".to_vec();
    out.extend((compressed.len() as u32).to_be_bytes());
    out.extend((raw.len() as u32).to_be_bytes());
    out.extend(compressed);
    out
}

pub fn write_frames(dir: &Path, records: &[Value]) {
    let mut bytes = Vec::new();
    for record in records {
        let mut record = record.clone();
        if let Some(payload) = record
            .get_mut("event")
            .and_then(|event| event.get_mut("payload"))
        {
            *payload = Value::String(
                base64::engine::general_purpose::STANDARD
                    .encode(payload.clone().to_string().as_bytes()),
            );
        }
        bytes.extend(frame(&serde_json::to_vec(&record).expect("record")));
    }
    fs::write(dir.join("events.frames"), bytes).expect("write frames");
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
