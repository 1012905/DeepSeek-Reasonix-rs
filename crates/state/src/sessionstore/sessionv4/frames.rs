use std::io::{self, Read};
use std::path::Path;

use base64::Engine;
use ruzstd::decoding::StreamingDecoder;
use ruzstd::decoding::errors::{FrameDecoderError, FrameHeaderError};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::content::ContentPool;
use super::error::{Damage, Limit, Unsupported, V4Error};
use super::fs::ConfinedRoot;

pub const FRAME_HEADER_LEN: usize = 12;

#[derive(Debug, Deserialize)]
pub struct Record {
    #[serde(rename = "schemaVersion", default)]
    pub schema_version: i64,
    #[serde(rename = "codec", default)]
    pub codec: String,
    #[serde(rename = "recordType", default)]
    pub record_type: String,
    #[serde(rename = "commitId", default)]
    pub commit_id: String,
    #[serde(rename = "firstSeq", default)]
    pub first_sequence: u64,
    #[serde(rename = "eventCount", default)]
    pub event_count: i64,
    pub event: Option<Event>,
    #[serde(rename = "sha256", default)]
    pub sha256: String,
}

fn decode_payload<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let encoded = Option::<String>::deserialize(deserializer)?;
    match encoded {
        None => Ok(Vec::new()),
        Some(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: u64,
    pub kind: String,
    #[serde(default, deserialize_with = "decode_payload")]
    pub payload: Vec<u8>,
    #[serde(rename = "payloadRef", default)]
    pub payload_ref: Option<super::content::ContentRef>,
}

pub fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<u8>>, V4Error> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    match read_required(reader, &mut header) {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(err) => return Err(V4Error::io("events.frames", err)),
    }
    if &header[..4] != b"RX4F" {
        return Err(Damage::FrameMagic.into());
    }
    let compressed_len = u32::from_be_bytes(header[4..8].try_into().expect("header slice"));
    let raw_len = u32::from_be_bytes(header[8..12].try_into().expect("header slice"));
    if compressed_len > super::MAX_FRAME_BYTES as u32 || raw_len > super::MAX_FRAME_BYTES as u32 {
        return Err(Limit::Frame {
            compressed: compressed_len,
            raw: raw_len,
        }
        .into());
    }
    if compressed_len == 0 || raw_len == 0 {
        return Err(Damage::FrameZero.into());
    }
    let mut compressed = vec![0u8; compressed_len as usize];
    if !read_required(reader, &mut compressed).map_err(|err| V4Error::io("events.frames", err))? {
        return Ok(None);
    }
    decode_frame(&compressed, raw_len as usize).map(Some)
}

fn read_required(reader: &mut impl Read, bytes: &mut [u8]) -> Result<bool, io::Error> {
    let mut read = 0;
    while read < bytes.len() {
        match reader.read(&mut bytes[read..]) {
            Ok(0) => return Ok(false),
            Ok(count) => read += count,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(true)
}

pub fn decode_frame(compressed: &[u8], raw_len: usize) -> Result<Vec<u8>, V4Error> {
    let decoder =
        StreamingDecoder::new_with_max_window_size(compressed, super::MAX_FRAME_BYTES as u64)
            .map_err(|err| {
                if matches!(
                    &err,
                    FrameDecoderError::WindowSizeTooBig { .. }
                        | FrameDecoderError::FrameHeaderError(
                            FrameHeaderError::WindowTooBig { .. }
                        )
                ) {
                    V4Error::TooLarge(Limit::Frame {
                        compressed: compressed.len() as u32,
                        raw: raw_len as u32,
                    })
                } else {
                    V4Error::Damaged(Damage::FrameDecode(io::Error::other(err)))
                }
            })?;
    let mut raw = Vec::with_capacity(raw_len);
    let mut bounded = decoder.take(raw_len as u64 + 1);
    bounded.read_to_end(&mut raw).map_err(|err| {
        let oversized = err
            .get_ref()
            .and_then(|source| source.downcast_ref::<FrameDecoderError>())
            .is_some_and(|kind| {
                matches!(
                    kind,
                    FrameDecoderError::WindowSizeTooBig { .. }
                        | FrameDecoderError::FrameHeaderError(
                            FrameHeaderError::WindowTooBig { .. }
                        )
                )
            });
        if oversized {
            V4Error::TooLarge(Limit::Frame {
                compressed: compressed.len() as u32,
                raw: raw_len as u32,
            })
        } else {
            V4Error::Damaged(Damage::FrameDecode(err))
        }
    })?;
    if raw.len() != raw_len {
        return Err(Damage::FrameLength {
            actual: raw.len(),
            expected: raw_len,
        }
        .into());
    }
    Ok(raw)
}

pub(crate) fn scan_commits(
    store: &ConfinedRoot,
    log: &Path,
    pool: &ContentPool,
    mut visit: impl FnMut(Vec<Event>) -> Result<(), V4Error>,
) -> Result<(), V4Error> {
    let mut file = store.open(log)?;
    let mut pending: Option<Record> = None;
    let mut events = Vec::new();
    let mut digest = Sha256::new();
    let mut next = 1u64;
    loop {
        let Some(raw) = read_frame(&mut file)? else {
            return Ok(());
        };
        let record: Record = serde_json::from_slice(&raw)
            .map_err(|err| V4Error::Damaged(Damage::RecordJson(err)))?;
        if record.schema_version != super::SCHEMA_VERSION || record.codec != super::CODEC {
            return Err(Unsupported::Schema {
                schema: record.schema_version,
                codec: record.codec,
                revision: 0,
            }
            .into());
        }
        match record.record_type.as_str() {
            "batch/begin" => {
                if pending.is_some() || record.first_sequence != next || record.event_count <= 0 {
                    return Err(Damage::BatchBegin.into());
                }
                digest = Sha256::new();
                digest.update(&raw);
                digest.update([0]);
                events = Vec::new();
                pending = Some(record);
            }
            "batch/event" => {
                let Some(begin) = pending.as_ref() else {
                    return Err(Damage::BatchEvent.into());
                };
                let Some(event) = record.event else {
                    return Err(Damage::BatchEvent.into());
                };
                if events.len() >= begin.event_count as usize
                    || event.seq != begin.first_sequence + events.len() as u64
                {
                    return Err(Damage::BatchEvent.into());
                }
                let mut event = event;
                event.payload = pool.payload(&event)?;
                events.push(event);
                digest.update(&raw);
                digest.update([0]);
            }
            "batch/end" => {
                let Some(begin) = pending.as_ref() else {
                    return Err(Damage::BatchEnd.into());
                };
                if record.commit_id != begin.commit_id
                    || events.len() != begin.event_count as usize
                    || super::hex(&digest.clone().finalize()) != record.sha256
                {
                    return Err(Damage::Checksum.into());
                }
                next = begin.first_sequence + begin.event_count as u64;
                pending = None;
                visit(std::mem::take(&mut events))?;
            }
            other => return Err(Unsupported::RecordType(other.to_string()).into()),
        }
    }
}
