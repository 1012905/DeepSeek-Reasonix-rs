use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum V4Error {
    #[error("read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Damaged(#[from] Damage),
    #[error(transparent)]
    Unsupported(#[from] Unsupported),
    #[error(transparent)]
    TooLarge(#[from] Limit),
}

#[derive(Debug, Error)]
pub enum Damage {
    #[error("manifest does not name its directory")]
    ManifestIdentity,
    #[error("manifest JSON is invalid: {0}")]
    ManifestJson(#[source] serde_json::Error),
    #[error("manifest path is not a child name")]
    ManifestPath,
    #[error("RX4F frame magic is invalid")]
    FrameMagic,
    #[error("RX4F frame has a zero length")]
    FrameZero,
    #[error("RX4F zstd decoding failed: {0}")]
    FrameDecode(#[source] io::Error),
    #[error("RX4F decoded {actual} bytes, header declares {expected}")]
    FrameLength { actual: usize, expected: usize },
    #[error("record JSON is invalid: {0}")]
    RecordJson(#[source] serde_json::Error),
    #[error("batch begin has an invalid sequence or count")]
    BatchBegin,
    #[error("batch event is outside its declared batch")]
    BatchEvent,
    #[error("batch end does not match its begin or event count")]
    BatchEnd,
    #[error("batch checksum does not match its records")]
    Checksum,
    #[error("payload JSON is invalid: {0}")]
    PayloadJson(#[source] serde_json::Error),
    #[error("message JSON is invalid: {0}")]
    MessageJson(#[source] serde_json::Error),
    #[error("content reference is invalid")]
    ContentRef,
    #[error("content object size mismatch")]
    ContentSize { actual: u64, expected: i64 },
    #[error("content object digest mismatch")]
    ContentDigest,
    #[error("read content object {path}: {source}")]
    ContentObject {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("event payload could not be resolved")]
    EventPayload(#[source] Box<V4Error>),
}

#[derive(Debug, Error)]
pub enum Unsupported {
    #[error("unsupported manifest schema {schema}, codec {codec:?}, revision {revision}")]
    Schema {
        schema: i64,
        codec: String,
        revision: i64,
    },
    #[error("unsupported record type {0:?}")]
    RecordType(String),
}

#[derive(Debug, Error)]
pub enum Limit {
    #[error("manifest exceeds {0} bytes")]
    Manifest(usize),
    #[error("RX4F frame sizes {compressed}/{raw} exceed the limit")]
    Frame { compressed: u32, raw: u32 },
    #[error("content object {digest} declares {bytes} bytes")]
    Object { digest: String, bytes: i64 },
}

impl V4Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
