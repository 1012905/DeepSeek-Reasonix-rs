//! Blob path validation and checksum-before-decode reads.

use std::path::{Path, PathBuf};

use super::Result;
use super::disk::StoreState;
#[cfg(test)]
use super::disk_fs::{ensure_private_dir, open_private_child_dir, write_atomic_file};
use super::disk_fs::{read_regular_file_in, validate_private_child_dir, validate_private_dir};
use super::disk_recovery::{BLOB_SUFFIX, BLOBS_NAME};
use super::manifest::valid_blob_stem;
use super::sha256_hex;
use super::types::{Error, PromptEnvelope};

impl super::disk::Store {
    pub(super) fn blob_path_at_locked(
        &self,
        state: &StoreState,
        blob_name: &str,
    ) -> Result<PathBuf> {
        if !valid_blob_stem(blob_name) {
            return Err(Error::InvalidItemPath);
        }
        Ok(state
            .binding
            .dir
            .join(BLOBS_NAME)
            .join(format!("{blob_name}{BLOB_SUFFIX}")))
    }

    #[cfg(test)]
    pub(super) fn write_blob_at_locked(
        &self,
        state: &StoreState,
        blob_name: &str,
        data: &[u8],
    ) -> Result<()> {
        let inbox = ensure_private_dir(&state.binding.dir)?;
        let blobs_path = state.binding.dir.join(BLOBS_NAME);
        let blobs = open_private_child_dir(&inbox, BLOBS_NAME.as_ref(), &blobs_path)?;
        let path = self.blob_path_at_locked(state, blob_name)?;
        let name = path.file_name().ok_or(Error::InvalidItemPath)?;
        reasonix_core::fileutil::crash("inbox-blob-write", &path.to_string_lossy());
        write_atomic_file(&blobs, Path::new(name), &path, data)?;
        reasonix_core::fileutil::crash("inbox-blob-rename", &path.to_string_lossy());
        Ok(())
    }

    pub(super) fn read_blob_at_locked(
        &self,
        state: &StoreState,
        blob_name: &str,
        want_checksum: &str,
    ) -> Result<PromptEnvelope> {
        let inbox = validate_private_dir(&state.binding.dir)?;
        let blobs_path = state.binding.dir.join(BLOBS_NAME);
        let blobs = validate_private_child_dir(&inbox, BLOBS_NAME.as_ref(), &blobs_path)?;
        let path = self.blob_path_at_locked(state, blob_name)?;
        let name = path.file_name().ok_or(Error::InvalidItemPath)?;
        let max_bytes = u64::try_from(self.limits.max_item_bytes).unwrap_or(u64::MAX);
        let data = read_regular_file_in(&blobs, Path::new(name), &path, max_bytes)?;
        if !want_checksum.is_empty() && sha256_hex(&data) != want_checksum {
            return Err(Error::ChecksumMismatch);
        }
        Ok(serde_json::from_slice::<Option<PromptEnvelope>>(&data)?.unwrap_or_default())
    }
}
