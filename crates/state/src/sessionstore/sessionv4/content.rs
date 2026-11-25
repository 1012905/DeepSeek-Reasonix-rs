use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::{Damage, Limit, V4Error};
use super::frames::Event;
use super::fs::ConfinedRoot;
use super::session::Session;

pub const MAX_OBJECT_BYTES: i64 = 256 << 20;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentRef {
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub bytes: i64,
    #[serde(rename = "mediaType", default)]
    pub media_type: String,
}

#[derive(Clone, Debug)]
pub struct ContentPool {
    store: ConfinedRoot,
    prefix: PathBuf,
}

impl ContentPool {
    pub fn for_session(session: &Session) -> Self {
        let prefix = if session.manifest.content_root == ".content-v1" {
            Path::new(&session.name).join(".content-v1")
        } else {
            PathBuf::from(".content-v1")
        };
        Self {
            store: session.store.clone(),
            prefix,
        }
    }

    pub fn read(&self, reference: &ContentRef) -> Result<Vec<u8>, V4Error> {
        if reference.bytes > MAX_OBJECT_BYTES {
            return Err(Limit::Object {
                digest: reference.digest.clone(),
                bytes: reference.bytes,
            }
            .into());
        }
        if reference.bytes < 0
            || reference.digest.len() != 64
            || !reference
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Damage::ContentRef.into());
        }
        let relative = self
            .prefix
            .join("objects")
            .join(&reference.digest[..2])
            .join(&reference.digest[2..4])
            .join(&reference.digest);
        let file = self.store.open(&relative).map_err(|err| {
            V4Error::Damaged(Damage::ContentObject {
                path: relative.clone(),
                source: std::io::Error::other(err),
            })
        })?;
        let mut data = Vec::new();
        file.take(reference.bytes as u64 + 1)
            .read_to_end(&mut data)
            .map_err(|err| {
                V4Error::Damaged(Damage::ContentObject {
                    path: relative.clone(),
                    source: err,
                })
            })?;
        if data.len() as i64 != reference.bytes {
            return Err(Damage::ContentSize {
                actual: data.len() as u64,
                expected: reference.bytes,
            }
            .into());
        }
        if super::hex(&Sha256::digest(&data)) != reference.digest {
            return Err(Damage::ContentDigest.into());
        }
        Ok(data)
    }

    pub fn payload(&self, event: &Event) -> Result<Vec<u8>, V4Error> {
        match &event.payload_ref {
            Some(reference) => self
                .read(reference)
                .map_err(|err| Damage::EventPayload(Box::new(err)).into()),
            None => Ok(event.payload.clone()),
        }
    }
}
