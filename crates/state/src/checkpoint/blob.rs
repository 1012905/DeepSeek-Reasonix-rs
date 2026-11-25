use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use reasonix_core::fileutil::atomic_write_file_strict;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlobError {
    #[error("blob store unavailable")]
    Unavailable,
    #[error("invalid blob reference {0}")]
    InvalidReference(String),
    #[error("blob {expected} failed content-address verification: got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug)]
pub struct BlobStore {
    dir: PathBuf,
    write_lock: Mutex<()>,
}

impl BlobStore {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
            write_lock: Mutex::new(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn put(&self, data: &[u8]) -> Result<String, BlobError> {
        if self.dir.as_os_str().is_empty() {
            return Err(BlobError::Unavailable);
        }
        let reference = digest(data);
        let path = self.path(&reference);
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Ok(existing) = fs::read(&path)
            && digest(&existing) == reference
        {
            return Ok(reference);
        }
        let parent = path.parent().ok_or_else(|| BlobError::Io {
            op: "resolve blob parent",
            path: path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "blob path has no parent"),
        })?;
        fs::create_dir_all(parent).map_err(|source| BlobError::Io {
            op: "create blob directory",
            path: parent.to_path_buf(),
            source,
        })?;
        atomic_write_file_strict(&path, data, 0o644).map_err(|source| BlobError::Io {
            op: "write blob",
            path,
            source,
        })?;
        Ok(reference)
    }

    pub fn get(&self, reference: &str) -> Result<Vec<u8>, BlobError> {
        if self.dir.as_os_str().is_empty() {
            return Err(BlobError::Unavailable);
        }
        if !valid_blob_ref(reference) {
            return Err(BlobError::InvalidReference(reference.to_owned()));
        }
        let path = self.path(reference);
        let data = fs::read(&path).map_err(|source| BlobError::Io {
            op: "read blob",
            path,
            source,
        })?;
        let actual = digest(&data);
        if actual != reference {
            return Err(BlobError::DigestMismatch {
                expected: reference.to_owned(),
                actual,
            });
        }
        Ok(data)
    }

    pub fn has(&self, reference: &str) -> bool {
        self.get(reference).is_ok()
    }

    pub fn remove(&self, reference: &str) -> Result<(), BlobError> {
        if self.dir.as_os_str().is_empty() || !valid_blob_ref(reference) {
            return Ok(());
        }
        let path = self.path(reference);
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(BlobError::Io {
                op: "remove blob",
                path,
                source,
            }),
        }
    }

    pub fn prune(&self, live: &HashSet<String>) -> Result<(), BlobError> {
        if self.dir.as_os_str().is_empty() {
            return Ok(());
        }
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        walk_files(&self.dir, &mut |path| {
            let Some(reference) = path.file_name().and_then(|name| name.to_str()) else {
                return Ok(());
            };
            if !valid_blob_ref(reference) || live.contains(reference) {
                return Ok(());
            }
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(BlobError::Io {
                    op: "prune blob",
                    path: path.to_path_buf(),
                    source,
                }),
            }
        })
    }

    pub fn size(&self) -> Result<u64, BlobError> {
        if self.dir.as_os_str().is_empty() {
            return Ok(0);
        }
        let mut total = 0u64;
        walk_files(&self.dir, &mut |path| {
            let metadata = fs::symlink_metadata(path).map_err(|source| BlobError::Io {
                op: "stat blob",
                path: path.to_path_buf(),
                source,
            })?;
            total = total.saturating_add(metadata.len());
            Ok(())
        })?;
        Ok(total)
    }

    pub(crate) fn path(&self, reference: &str) -> PathBuf {
        self.dir
            .join(&reference[..2])
            .join(&reference[2..4])
            .join(reference)
    }
}

fn walk_files(
    dir: &Path,
    visit: &mut dyn FnMut(&Path) -> Result<(), BlobError>,
) -> Result<(), BlobError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(BlobError::Io {
                op: "walk blob directory",
                path: dir.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| BlobError::Io {
            op: "read blob directory entry",
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|source| BlobError::Io {
                op: "stat blob directory entry",
                path: path.clone(),
                source,
            })?
            .is_dir()
        {
            walk_files(&path, visit)?;
        } else {
            visit(&path)?;
        }
    }
    Ok(())
}

pub fn valid_blob_ref(reference: &str) -> bool {
    reference.len() == 64
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn digest(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
