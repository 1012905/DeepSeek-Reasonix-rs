//! Content-addressed image blobs referenced by event records
//! (session_image_blobs.go, read path).
//!
//! Each record may carry image_blobs: per-message slot lists naming a
//! blob key (sha256 of the data URL) or an inline URL. Resolution
//! restores message images from the session's blobs directory; a
//! missing or corrupt blob leaves an explicit unavailable-image entry
//! instead of failing the replay.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use super::record::{SessionEventImages, SessionImageSlot};
use super::{
    DecodeError, SESSION_EVENT_REPLAY_MAX_BYTES, SessionReplayLimits, session_replay_limit_error,
};
use reasonix_contract::provider::{Message, UnavailableImage};

/// Key prefix for blob names (Go: sessionImageBlobScheme).
pub const SESSION_IMAGE_BLOB_SCHEME: &str = "sha256:";
/// Largest blob read back into a message (Go: sessionImageBlobMaxBytes).
pub const SESSION_IMAGE_BLOB_MAX_BYTES: u64 = 64 << 20;

/// Error returned when image payload cannot be restored (Go:
/// ErrSessionImagesUnavailable) — surfaced by later slices' save paths.
pub const ERR_SESSION_IMAGES_UNAVAILABLE: &str = "session image payload unavailable";

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Content key of a data URL (Go: sessionImageKey = "sha256:"+hex(sha256(url))).
pub fn session_image_key(url: &str) -> String {
    let mut sum = [0u8; 32];
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    sum.copy_from_slice(&hasher.finalize());
    let mut key = String::with_capacity(SESSION_IMAGE_BLOB_SCHEME.len() + 64);
    key.push_str(SESSION_IMAGE_BLOB_SCHEME);
    for byte in sum {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0xf) as usize] as char);
    }
    key
}

/// Blob file path for a key, or None when the key is malformed (Go:
/// sessionImageBlobPath: scheme prefix stripped, 64 lowercase hex,
/// clean basename).
pub fn session_image_blob_path(dir: &Path, key: &str) -> Option<PathBuf> {
    let name = key.strip_prefix(SESSION_IMAGE_BLOB_SCHEME)?;
    if name.len() != 64 || !name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    if dir.as_os_str().is_empty() {
        return None;
    }
    let path = dir.join(name);
    if path.file_name()?.to_str()? != name {
        return None;
    }
    Some(path)
}

/// Reads one blob back into its data URL, verifying size and content
/// key (Go: readSessionImageBlob). Returns None for missing files,
/// oversize or mismatched content, or invalid UTF-8 — the last is
/// deliberately stricter than Go, which tolerates invalid UTF-8 bytes
/// in URLs; here such a slot resolves as unavailable instead.
pub fn read_session_image_blob(dir: &Path, key: &str) -> Option<String> {
    let path = session_image_blob_path(dir, key)?;
    let file = File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(SESSION_IMAGE_BLOB_MAX_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > SESSION_IMAGE_BLOB_MAX_BYTES {
        return None;
    }
    let url = String::from_utf8(buf).ok()?;
    if session_image_key(&url) != key {
        return None;
    }
    Some(url)
}

/// Number of slots across all reference groups (Go:
/// sessionImageSlotCount).
pub fn session_image_slot_count(refs: &[SessionEventImages]) -> usize {
    refs.iter().map(|r| r.slots.len()).sum()
}

/// Restores message images from blob references (Go:
/// resolveSessionImages). A reference outside the message range is a
/// hard error; unreadable blobs downgrade to unavailable entries.
pub fn resolve_session_images(
    log_path: &str,
    msgs: &mut [Message],
    refs: &[SessionEventImages],
) -> Result<(), String> {
    let dir = crate::store::session_blobs_dir_for_event_log(Path::new(log_path));
    for group in refs {
        if group.message < 0 || group.message as usize >= msgs.len() {
            return Err(format!(
                "image reference to message {} outside its record",
                group.message
            ));
        }
        let message = &mut msgs[group.message as usize];
        message.images.clear();
        message.unavailable_images.clear();
        for (j, slot) in group.slots.iter().enumerate() {
            if slot.blob.is_empty() {
                message.images.push(slot.url.clone());
            } else if let Some(url) = read_session_image_blob(&dir, &slot.blob) {
                message.images.push(url);
            } else {
                message
                    .unavailable_images
                    .push(reasonix_contract::provider::UnavailableImage {
                        index: j as i64,
                        reference: slot.blob.clone(),
                    });
            }
        }
    }
    Ok(())
}

/// Budget-gated wrapper around resolve (Go:
/// resolveSessionEventImages): reference groups and their slots join
/// the collection-item count before any file is read.
pub(crate) fn resolve_session_event_images(
    path: &str,
    msgs: &mut [Message],
    refs: &[SessionEventImages],
    existing_items: usize,
    limits: &SessionReplayLimits,
) -> Result<usize, DecodeError> {
    if refs.is_empty() {
        return Ok(existing_items);
    }
    let items = existing_items + refs.len() + session_image_slot_count(refs);
    if items > limits.max_collection_items {
        return Err(DecodeError::Limit(session_replay_limit_error(
            path,
            "message_collection_items",
            items as i64,
            limits.max_collection_items as i64,
        )));
    }
    resolve_session_images(path, msgs, refs).map_err(DecodeError::Other)?;
    Ok(items)
}

// ---------------------------------------------------------------------------
// Write path (session_image_blobs.go 60-502): externalization moves image
// payloads into the blob directory before the record naming them is
// encoded, the checkpoint scanner recovers images whose blob is gone, and
// the sweep keeps the blob directory from outliving the images it holds.
// ---------------------------------------------------------------------------

/// A temp blob file must be this old before a sweep removes it (Go:
/// sessionImageBlobTempMaxAge).
pub const SESSION_IMAGE_BLOB_TEMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Whether url is an inline data: URL (Go: isDataURL).
pub fn is_data_url(url: &str) -> bool {
    url.starts_with("data:")
}

/// Whether the message stores image payloads outside the event log (Go:
/// carriesStoredImage): an unavailable-image entry or any inline data: URL.
pub fn carries_stored_image(message: &Message) -> bool {
    !message.unavailable_images.is_empty() || message.images.iter().any(|url| is_data_url(url))
}

/// Lays a message's images out as written: unavailable ones back at their
/// positions, the readable ones filling the rest in order (Go:
/// messageImageSlots).
pub fn message_image_slots(message: &Message) -> Vec<SessionImageSlot> {
    let mut slots =
        vec![SessionImageSlot::default(); message.images.len() + message.unavailable_images.len()];
    let mut taken = vec![false; slots.len()];
    let mut strays = Vec::new();
    for u in &message.unavailable_images {
        let index = u.index;
        if index >= 0 && (index as usize) < slots.len() && !taken[index as usize] {
            slots[index as usize].blob = u.reference.clone();
            taken[index as usize] = true;
            continue;
        }
        strays.push(u.clone());
    }
    let mut next = 0;
    let mut fill = |slot: SessionImageSlot| {
        // The counts always leave a free slot — len(images) plus
        // len(strays) fills at most the slots unavailable entries have
        // not taken — so the scan stays in range.
        while next < taken.len() && taken[next] {
            next += 1;
        }
        if next < slots.len() {
            slots[next] = slot;
            taken[next] = true;
            next += 1;
        }
    };
    for url in &message.images {
        fill(SessionImageSlot {
            blob: String::new(),
            url: url.clone(),
        });
    }
    for u in &strays {
        fill(SessionImageSlot {
            blob: u.reference.clone(),
            url: String::new(),
        });
    }
    slots
}

/// Error writing an image blob or externalizing a record's images
/// (session_image_blobs.go).
#[derive(Debug)]
pub enum ExternalizeError {
    /// The session path is empty and a message carries a stored image.
    EmptySessionPath,
    /// The blob write failed.
    WriteBlob(io::Error),
    /// The blob directory path is empty (unreachable through
    /// externalize, which checks the session path first).
    EmptyBlobDir,
}

impl std::fmt::Display for ExternalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySessionPath => write!(f, "session image blobs: empty session path"),
            Self::WriteBlob(err) => write!(f, "write session image blob: {err}"),
            Self::EmptyBlobDir => write!(f, "session image blobs: empty blob directory"),
        }
    }
}

impl std::error::Error for ExternalizeError {}

/// Outcome of externalization (Go: externalizeSessionImages results).
/// messages is None when no message carried a stored image, so callers
/// keep using their input — Go's same-slice fast path.
#[derive(Debug, Default)]
pub struct ExternalizedImages {
    pub messages: Option<Vec<Message>>,
    pub image_blobs: Vec<SessionEventImages>,
}

/// Writes every data-URL image in msgs to the session's blob directory and
/// returns msgs with those messages' images moved into the side table.
/// Each blob is fsynced before this returns, so the record naming it can
/// never reach disk first. msgs itself is not modified (Go:
/// externalizeSessionImages).
pub fn externalize_session_images(
    session_path: &Path,
    msgs: &[Message],
) -> Result<ExternalizedImages, ExternalizeError> {
    let mut out: Option<Vec<Message>> = None;
    let mut refs = Vec::new();
    let dir = crate::store::session_blobs_dir(session_path);
    for (i, m) in msgs.iter().enumerate() {
        if !carries_stored_image(m) {
            continue;
        }
        if dir.as_os_str().is_empty() {
            return Err(ExternalizeError::EmptySessionPath);
        }
        let mut slots = message_image_slots(m);
        for slot in &mut slots {
            if !is_data_url(&slot.url) {
                continue;
            }
            let key = write_session_image_blob(&dir, &slot.url)?;
            *slot = SessionImageSlot {
                blob: key,
                url: String::new(),
            };
        }
        let out = out.get_or_insert_with(|| msgs.to_vec());
        out[i].images.clear();
        out[i].unavailable_images.clear();
        refs.push(SessionEventImages {
            message: i as i64,
            slots,
        });
    }
    Ok(ExternalizedImages {
        messages: out,
        image_blobs: refs,
    })
}

/// Publishes url under its hash unless a file there already hashes to it;
/// one that does not is damage and is replaced (Go: writeSessionImageBlob).
pub fn write_session_image_blob(dir: &Path, url: &str) -> Result<String, ExternalizeError> {
    let key = session_image_key(url);
    if read_session_image_blob(dir, &key).is_some() {
        return Ok(key);
    }
    let Some(path) = session_image_blob_path(dir, &key) else {
        return Err(ExternalizeError::EmptyBlobDir);
    };
    // Whether the directory predates the write: a new one needs its entry
    // synced in the parent.
    let sync_parent = matches!(
        std::fs::metadata(dir),
        Err(err) if err.kind() == io::ErrorKind::NotFound
    );
    reasonix_core::fileutil::atomic_write_file_strict(&path, url.as_bytes(), 0o600)
        .map_err(ExternalizeError::WriteBlob)?;
    if sync_parent {
        sync_session_dir(super::parent_or_dot(dir));
    }
    Ok(key)
}

/// Makes a new blob directory's entry durable. Best effort, like the
/// parent sync AtomicWriteFileStrict performs: some platforms cannot (Go:
/// syncSessionDir).
pub fn sync_session_dir(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

/// Every blob key the messages still name (Go: unavailableImageKeys).
pub fn unavailable_image_keys(msgs: &[Message]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for m in msgs {
        for u in &m.unavailable_images {
            keys.insert(u.reference.clone());
        }
    }
    keys
}

/// Puts url back where the image named by reference sat (Go:
/// restoreUnavailableImage). Each still-unavailable image with a smaller
/// index has already shifted the surviving images left, so it moves the
/// insert position back by one.
pub fn restore_unavailable_image(message: &mut Message, reference: &str, url: &str) {
    loop {
        let Some(at) = message
            .unavailable_images
            .iter()
            .position(|u| u.reference == reference)
        else {
            return;
        };
        let u = message.unavailable_images.remove(at);
        let mut pos = u.index;
        for other in &message.unavailable_images {
            if other.index < u.index {
                pos -= 1;
            }
        }
        pos = pos.clamp(0, message.images.len() as i64);
        message.images.insert(pos as usize, url.to_owned());
    }
}

/// Finds the data URLs in the checkpoint whose keys are wanted. It reads
/// raw bytes rather than decoding messages, so a copy past a torn or
/// foreign line is still found: encoding/json writes a data URL verbatim.
/// A missing checkpoint is not an error — there is nothing to find — but
/// one that exists and cannot be read whole returns an error (Go:
/// checkpointImages).
pub fn checkpoint_images(
    checkpoint: &Path,
    wanted: &BTreeSet<String>,
) -> io::Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    let info = match std::fs::metadata(checkpoint) {
        Ok(info) => info,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!("stat checkpoint: {err}"),
            ));
        }
    };
    if !info.is_file() {
        return Err(io::Error::other("checkpoint is not a regular file"));
    }
    if info.len() as i64 > SESSION_EVENT_REPLAY_MAX_BYTES {
        return Err(io::Error::other(format!(
            "checkpoint is {} bytes, past the {}-byte replay ceiling",
            info.len(),
            SESSION_EVENT_REPLAY_MAX_BYTES
        )));
    }
    let raw = std::fs::read(checkpoint)
        .map_err(|err| io::Error::new(err.kind(), format!("read checkpoint: {err}")))?;
    let mut b: &[u8] = &raw;
    while found.len() < wanted.len() {
        let Some(at) = find_subslice(b, b"\"data:") else {
            break;
        };
        b = &b[at + 1..];
        let Some(end) = b.iter().position(|&c| c == b'"') else {
            break;
        };
        let raw_url = &b[..end];
        b = &b[end + 1..];
        let mut url = String::from_utf8_lossy(raw_url).into_owned();
        if raw_url.contains(&b'\\') {
            let quoted = format!("\"{url}\"");
            match serde_json::from_str::<String>(&quoted) {
                Ok(unescaped) => url = unescaped,
                Err(_) => continue,
            }
        }
        let key = session_image_key(&url);
        if wanted.contains(&key) {
            found.insert(key, url);
        }
    }
    Ok(found)
}

/// Restores images whose blob is gone from the copy the checkpoint still
/// inlines, and republishes the blob so the next replay needs no
/// checkpoint (Go: backfillUnavailableImages). Best effort: failures warn
/// and leave the in-memory transcript untouched; a blob that cannot be
/// written still restores the image in memory — the bytes are in hand,
/// and the next save retries the write.
pub fn backfill_unavailable_images(log_path: &Path, msgs: &mut [Message]) {
    let wanted = unavailable_image_keys(msgs);
    if wanted.is_empty() {
        return;
    }
    let checkpoint = crate::store::session_path_for_event_log(log_path);
    let found = match checkpoint_images(&checkpoint, &wanted) {
        Ok(found) => found,
        Err(err) => {
            // Best effort: a checkpoint that could not be read restores
            // nothing, and the caller still needs the blob-recovery
            // warning below.
            tracing::warn!(
                log = %log_path.display(),
                error = %err,
                "session: could not inspect checkpoint for image payloads"
            );
            BTreeMap::new()
        }
    };
    let dir = crate::store::session_blobs_dir_for_event_log(log_path);
    for (key, url) in &found {
        if let Err(err) = write_session_image_blob(&dir, url) {
            tracing::warn!(
                log = %log_path.display(),
                error = %err,
                "session: could not restore image blob from checkpoint"
            );
        }
        for message in msgs.iter_mut() {
            restore_unavailable_image(message, key, url);
        }
    }
    let missing = wanted.len().saturating_sub(found.len());
    if missing > 0 {
        tracing::warn!(
            log = %log_path.display(),
            missing = missing,
            "session: image payloads unavailable"
        );
    }
}

/// A transcript missing an image may not replace a checkpoint that still
/// inlines it: that file would then be the only copy, and the rewrite
/// would destroy it. A checkpoint that cannot be inspected fails closed —
/// the guard cannot rule it out as that only copy (Go:
/// refuseDegradedCheckpointRewrite).
#[derive(Debug)]
pub enum DegradedCheckpointError {
    /// The checkpoint could not be read, so its images cannot be ruled
    /// out; it was left unchanged.
    Unreadable(io::Error),
    /// The checkpoint holds the only copy of n image(s); it was left
    /// unchanged.
    HoldsOnlyCopy(usize),
}

impl std::fmt::Display for DegradedCheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(err) => write!(
                f,
                "{ERR_SESSION_IMAGES_UNAVAILABLE}: the checkpoint could not be read, so its images cannot be ruled out; it was left unchanged: {err}"
            ),
            Self::HoldsOnlyCopy(n) => write!(
                f,
                "{ERR_SESSION_IMAGES_UNAVAILABLE}: the checkpoint holds the only copy of {n} image(s); it was left unchanged"
            ),
        }
    }
}

impl std::error::Error for DegradedCheckpointError {}

/// The rewrite guard itself; see [DegradedCheckpointError].
pub fn refuse_degraded_checkpoint_rewrite(
    checkpoint: &Path,
    msgs: &[Message],
) -> Result<(), DegradedCheckpointError> {
    let wanted = unavailable_image_keys(msgs);
    if wanted.is_empty() {
        return Ok(());
    }
    let held =
        checkpoint_images(checkpoint, &wanted).map_err(DegradedCheckpointError::Unreadable)?;
    if !held.is_empty() {
        return Err(DegradedCheckpointError::HoldsOnlyCopy(held.len()));
    }
    Ok(())
}

/// Whether m still inlines any of the lost images (Go: holdsAnyImage).
pub fn holds_any_image(message: &Message, lost: &[UnavailableImage]) -> bool {
    message
        .images
        .iter()
        .any(|url| is_data_url(url) && lost.iter().any(|u| u.reference == session_image_key(url)))
}

/// Every blob key the session's salvage and recovery sidecars name (Go:
/// salvagedImageKeys). None means a sidecar could not be read whole, and
/// then nothing may be swept.
pub fn salvaged_image_keys(session_path: &Path) -> Option<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for sidecar in [
        crate::store::session_event_log_damaged(session_path),
        crate::store::session_superseded(session_path),
        crate::store::session_context(session_path),
    ] {
        let info = match std::fs::metadata(&sidecar) {
            Ok(info) => info,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return None,
        };
        if info.len() as i64 > SESSION_EVENT_REPLAY_MAX_BYTES {
            return None;
        }
        let raw = std::fs::read(&sidecar).ok()?;
        scan_sha256_keys(&raw, &mut keys);
    }
    Some(keys)
}

fn scan_sha256_keys(raw: &[u8], keys: &mut BTreeSet<String>) {
    let mut at = 0;
    while let Some(pos) = find_subslice(&raw[at..], b"sha256:") {
        let start = at + pos;
        let hex_start = start + "sha256:".len();
        let hex_end = hex_start + 64;
        if hex_end <= raw.len()
            && raw[hex_start..hex_end]
                .iter()
                .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        {
            keys.insert(String::from_utf8_lossy(&raw[start..hex_end]).into_owned());
            at = hex_end;
        } else {
            at = start + 1;
        }
    }
}

/// Removes blobs that neither refs nor a salvage sidecar names (Go:
/// sweepSessionImageBlobs). Callers run it only after the log naming
/// exactly refs is published.
pub fn sweep_session_image_blobs(session_path: &Path, refs: &[SessionEventImages]) {
    let dir = crate::store::session_blobs_dir(session_path);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let Some(mut keep) = salvaged_image_keys(session_path) else {
        return;
    };
    for group in refs {
        for slot in &group.slots {
            if !slot.blob.is_empty() {
                keep.insert(slot.blob.clone());
            }
        }
    }
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let key = format!("{SESSION_IMAGE_BLOB_SCHEME}{name}");
        if session_image_blob_path(&dir, &key).is_some() {
            if !keep.contains(key.as_str()) {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        // Stale AtomicWriteFile temps from a crashed blob write.
        if name.starts_with(".atomic-")
            && name.ends_with(".tmp")
            && let Ok(info) = entry.metadata()
            && let Ok(modified) = info.modified()
            && let Ok(age) = SystemTime::now().duration_since(modified)
            && age > SESSION_IMAGE_BLOB_TEMP_MAX_AGE
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// How much of a transcript's encoded size is data-URL images, which a
/// folded log holds outside itself (Go: inlineImageBytes).
pub fn inline_image_bytes(msgs: &[Message]) -> i64 {
    let mut n = 0;
    for m in msgs {
        for url in &m.images {
            if is_data_url(url) {
                n += url.len() as i64;
            }
        }
    }
    n
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let first = needle[0];
    let mut at = 0;
    while hay.len() - at >= needle.len() {
        let start = at + hay[at..].iter().position(|&c| c == first)?;
        // A first byte this close to the end can never fit the whole
        // needle (Go bytes.Index returns -1 there).
        if start + needle.len() > hay.len() {
            return None;
        }
        if &hay[start..start + needle.len()] == needle {
            return Some(start);
        }
        at = start + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;
    use reasonix_contract::go_struct;

    fn msg(role: &'static str) -> Message {
        Message {
            role: reasonix_contract::provider::Role::from(role),
            ..Default::default()
        }
    }

    #[test]
    fn key_is_salted_sha256_of_empty() {
        assert_eq!(
            session_image_key(""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn key_hashes_url_bytes() {
        let key = session_image_key("data:image/png;base64,AAAA");
        assert!(key.starts_with("sha256:"));
        assert_eq!(key.len(), "sha256:".len() + 64);
    }

    #[test]
    fn blob_path_validates_key_shape() {
        let dir = PathBuf::from("blobs");
        let key = session_image_key("x");
        let path = session_image_blob_path(&dir, &key).unwrap();
        assert_eq!(path, PathBuf::from("blobs").join(&key["sha256:".len()..]));
        assert!(session_image_blob_path(&dir, "").is_none());
        assert!(session_image_blob_path(&dir, "sha256:xyz").is_none());
        assert!(
            session_image_blob_path(
                &dir,
                "sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855"
            )
            .is_none(),
            "uppercase hex rejected"
        );
        assert!(session_image_blob_path(&PathBuf::new(), &key).is_none());
    }

    #[test]
    fn blob_round_trip_and_corruption() {
        let tmp = test_support::tempdir();
        let dir = tmp.path().join("s.events.jsonl.blobs");
        std::fs::create_dir_all(&dir).unwrap();
        let url = "data:image/png;base64,QUJD";
        let key = session_image_key(url);
        std::fs::write(dir.join(&key["sha256:".len()..]), url).unwrap();
        assert_eq!(read_session_image_blob(&dir, &key).as_deref(), Some(url));
        // Wrong content fails the key check.
        std::fs::write(dir.join(&key["sha256:".len()..]), "data:text/plain,other").unwrap();
        assert!(read_session_image_blob(&dir, &key).is_none());
        // Unknown key reads as absent.
        let missing = session_image_key("data:image/png;base64,nope");
        assert!(read_session_image_blob(&dir, &missing).is_none());
    }

    #[test]
    fn blob_read_rejects_invalid_utf8() {
        let tmp = test_support::tempdir();
        let dir = tmp.path().join("b");
        std::fs::create_dir_all(&dir).unwrap();
        let raw = [0xffu8, 0xfe];
        let mut sum = [0u8; 32];
        let mut hasher = Sha256::new();
        hasher.update(raw);
        sum.copy_from_slice(&hasher.finalize());
        let mut key = String::from("sha256:");
        for byte in sum {
            key.push(HEX[(byte >> 4) as usize] as char);
            key.push(HEX[(byte & 0xf) as usize] as char);
        }
        std::fs::write(dir.join(&key["sha256:".len()..]), raw).unwrap();
        assert!(read_session_image_blob(&dir, &key).is_none());
    }

    go_struct! {
        /// Local shim reusing the wire record's images shape.
        pub struct ShimImages : ShimImagesWire {
            pub message: i64,
            pub slots: Vec<ShimSlot>,
        }
    }

    go_struct! {
        pub struct ShimSlot : ShimSlotWire {
            pub blob: String,
            pub url: String,
        }
    }

    #[test]
    fn resolve_restores_and_marks_unavailable() {
        let tmp = test_support::tempdir();
        let log_path = tmp.path().join("s.jsonl.events.jsonl");
        let log_str = log_path.to_str().unwrap().to_owned();
        let dir = crate::store::session_blobs_dir_for_event_log(&log_path);
        std::fs::create_dir_all(&dir).unwrap();
        let url = "data:image/png;base64,QQ==";
        let key = session_image_key(url);
        std::fs::write(dir.join(&key["sha256:".len()..]), url).unwrap();
        let missing_key = session_image_key("data:image/png;base64,gone");

        let mut msgs = [msg("user")];
        let refs = [SessionEventImages {
            message: 0,
            slots: vec![
                super::super::record::SessionImageSlot {
                    blob: key.clone(),
                    url: String::new(),
                },
                super::super::record::SessionImageSlot {
                    blob: missing_key.clone(),
                    url: String::new(),
                },
                super::super::record::SessionImageSlot {
                    blob: String::new(),
                    url: "data:image/png;base64,inline".to_owned(),
                },
            ],
        }];
        resolve_session_images(&log_str, &mut msgs, &refs).unwrap();
        assert_eq!(msgs[0].images.len(), 2);
        assert_eq!(msgs[0].images[0], url);
        assert_eq!(msgs[0].images[1], "data:image/png;base64,inline");
        assert_eq!(msgs[0].unavailable_images.len(), 1);
        assert_eq!(msgs[0].unavailable_images[0].index, 1);
        assert_eq!(msgs[0].unavailable_images[0].reference, missing_key);
    }

    #[test]
    fn resolve_rejects_out_of_range_message() {
        let mut msgs = [msg("user")];
        let refs = [SessionEventImages {
            message: 3,
            slots: vec![],
        }];
        let err = resolve_session_images("x.events.jsonl", &mut msgs, &refs).unwrap_err();
        assert_eq!(err, "image reference to message 3 outside its record");
        let refs = [SessionEventImages {
            message: -1,
            slots: vec![],
        }];
        let err = resolve_session_images("x.events.jsonl", &mut msgs, &refs).unwrap_err();
        assert_eq!(err, "image reference to message -1 outside its record");
    }

    #[test]
    fn budget_wrapper_counts_groups_and_slots() {
        let mut limits = SessionReplayLimits {
            max_collection_items: 4,
            ..Default::default()
        };
        let mut msgs = [msg("user"), msg("assistant")];
        let refs = [
            SessionEventImages {
                message: 0,
                slots: vec![Default::default(); 2],
            },
            SessionEventImages {
                message: 1,
                slots: vec![Default::default(); 1],
            },
        ];
        // 2 groups + 3 slots + 0 existing = 5 > 4.
        match resolve_session_event_images("x.events.jsonl", &mut msgs, &refs, 0, &limits) {
            Err(DecodeError::Limit(e)) => {
                assert_eq!(e.resource, "message_collection_items");
                assert_eq!(e.value, 5);
            }
            other => panic!("{other:?}"),
        }
        // 5 items at limit 5 passes the gate and resolves inline URLs.
        limits.max_collection_items = 5;
        let items =
            resolve_session_event_images("x.events.jsonl", &mut msgs, &refs, 0, &limits).unwrap();
        assert_eq!(items, 5);
        assert_eq!(msgs[0].images.len(), 2);
        assert_eq!(msgs[1].images.len(), 1);
    }

    #[test]
    fn later_group_replaces_same_message_images() {
        // Go clears Images/UnavailableImages once per reference group, so
        // two groups pointing at one message leave only the last group's
        // slots on it.
        let limits = SessionReplayLimits::default();
        let mut msgs = [msg("user")];
        let refs = [
            SessionEventImages {
                message: 0,
                slots: vec![Default::default(); 2],
            },
            SessionEventImages {
                message: 0,
                slots: vec![Default::default(); 1],
            },
        ];
        resolve_session_event_images("x.events.jsonl", &mut msgs, &refs, 0, &limits).unwrap();
        assert_eq!(msgs[0].images.len(), 1);
    }

    // ---- write path ----

    fn user_msg() -> Message {
        msg("user")
    }

    #[test]
    fn is_data_url_and_carries_stored_image() {
        assert!(is_data_url("data:image/png;base64,QQ=="));
        assert!(!is_data_url("https://example.com/x.png"));
        assert!(!carries_stored_image(&user_msg()));
        let with_inline = Message {
            images: vec!["data:image/png;base64,QQ==".to_owned()],
            ..user_msg()
        };
        assert!(carries_stored_image(&with_inline));
        let with_unavailable = Message {
            unavailable_images: vec![UnavailableImage {
                index: 0,
                reference: "sha256:aa".to_owned(),
            }],
            ..user_msg()
        };
        assert!(carries_stored_image(&with_unavailable));
        let with_remote = Message {
            images: vec!["https://example.com/x.png".to_owned()],
            ..user_msg()
        };
        assert!(!carries_stored_image(&with_remote));
    }

    #[test]
    fn message_image_slots_layout() {
        // Two unavailable entries claim positions 0 and 2; a duplicate
        // index and an out-of-range entry wait as strays; the two
        // readable images fill positions 1 and 3, then strays append.
        let m = Message {
            images: vec!["data:a".to_owned(), "data:b".to_owned()],
            unavailable_images: vec![
                UnavailableImage {
                    index: 0,
                    reference: "sha256:zero".to_owned(),
                },
                UnavailableImage {
                    index: 2,
                    reference: "sha256:two".to_owned(),
                },
                UnavailableImage {
                    index: 2,
                    reference: "sha256:stray1".to_owned(),
                },
                UnavailableImage {
                    index: 9,
                    reference: "sha256:stray2".to_owned(),
                },
            ],
            ..user_msg()
        };
        let slots = message_image_slots(&m);
        assert_eq!(slots.len(), 6);
        assert_eq!(slots[0].blob, "sha256:zero");
        assert_eq!(slots[0].url, "");
        assert_eq!(slots[1].url, "data:a");
        assert_eq!(slots[1].blob, "");
        assert_eq!(slots[2].blob, "sha256:two");
        assert_eq!(slots[3].url, "data:b");
        assert_eq!(slots[4].blob, "sha256:stray1");
        assert_eq!(slots[5].blob, "sha256:stray2");
    }

    #[test]
    fn externalize_writes_blob_and_clears_message() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let url = "data:image/png;base64,QQ==";
        let msgs = vec![Message {
            images: vec![url.to_owned()],
            ..user_msg()
        }];
        let out = externalize_session_images(&session, &msgs).unwrap();
        let out_messages = out.messages.expect("carrying message clones");
        assert!(out_messages[0].images.is_empty());
        assert!(out_messages[0].unavailable_images.is_empty());
        assert_eq!(msgs[0].images, vec![url.to_owned()], "input not modified");
        assert_eq!(out.image_blobs.len(), 1);
        assert_eq!(out.image_blobs[0].message, 0);
        let slots = &out.image_blobs[0].slots;
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].blob, session_image_key(url));
        assert_eq!(slots[0].url, "");
        // The blob is on disk and reads back.
        let dir = crate::store::session_blobs_dir(&session);
        assert_eq!(
            read_session_image_blob(&dir, &slots[0].blob).as_deref(),
            Some(url)
        );
    }

    #[test]
    fn externalize_fast_path_returns_none() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let msgs = vec![user_msg()];
        let out = externalize_session_images(&session, &msgs).unwrap();
        assert!(out.messages.is_none());
        assert!(out.image_blobs.is_empty());
        // No blob directory was created for a text-only transcript.
        assert!(!crate::store::session_blobs_dir(&session).exists());
    }

    #[test]
    fn externalize_rejects_empty_session_path() {
        let msgs = vec![Message {
            images: vec!["data:image/png;base64,QQ==".to_owned()],
            ..user_msg()
        }];
        let err = externalize_session_images(Path::new(""), &msgs).unwrap_err();
        assert_eq!(err.to_string(), "session image blobs: empty session path");
    }

    #[test]
    fn write_blob_replaces_corrupt_content() {
        let tmp = test_support::tempdir();
        let dir = tmp.path().join("blobs");
        std::fs::create_dir_all(&dir).unwrap();
        let url = "data:image/png;base64,QQ==";
        let key = session_image_key(url);
        let name = &key["sha256:".len()..];
        // A file that does not hash to its name is damage and replaced.
        std::fs::write(dir.join(name), "wrong").unwrap();
        assert_eq!(write_session_image_blob(&dir, url).unwrap(), key);
        assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), url);
        // Second write hits the read-back dedupe.
        assert_eq!(write_session_image_blob(&dir, url).unwrap(), key);
    }

    #[test]
    fn checkpoint_images_scans_and_unescapes() {
        let tmp = test_support::tempdir();
        let checkpoint = tmp.path().join("s.jsonl");
        // encoding/json escapes < as a unicode escape; the scanner must
        // unquote the raw bytes it found.
        std::fs::write(
            &checkpoint,
            &b"{\"images\":[\"data:text/plain,a\\u003cb\"]}"[..],
        )
        .unwrap();
        let url = "data:text/plain,a<b";
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        wanted.insert(session_image_key(url));
        let found = checkpoint_images(&checkpoint, &wanted).unwrap();
        assert_eq!(
            found.get(&session_image_key(url)).map(String::as_str),
            Some(url)
        );
        // An unwanted URL is not picked up.
        wanted.clear();
        assert!(checkpoint_images(&checkpoint, &wanted).unwrap().is_empty());
        // A verbatim (unescape-free) URL is found by the raw scan.
        std::fs::write(
            &checkpoint,
            &b"{\"images\":[\"data:image/png;base64,QQ==\"]}"[..],
        )
        .unwrap();
        let url2 = "data:image/png;base64,QQ==";
        let mut wanted2: BTreeSet<String> = BTreeSet::new();
        wanted2.insert(session_image_key(url2));
        let found2 = checkpoint_images(&checkpoint, &wanted2).unwrap();
        assert_eq!(
            found2.get(&session_image_key(url2)).map(String::as_str),
            Some(url2)
        );
    }

    #[test]
    fn checkpoint_images_missing_and_invalid() {
        let tmp = test_support::tempdir();
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        wanted.insert(session_image_key("data:x"));
        // Missing checkpoint: nothing to find, no error.
        let found = checkpoint_images(&tmp.path().join("gone.jsonl"), &wanted).unwrap();
        assert!(found.is_empty());
        // A directory is not a regular file.
        let dir = tmp.path().join("dir");
        std::fs::create_dir_all(&dir).unwrap();
        let err = checkpoint_images(&dir, &wanted).unwrap_err();
        assert_eq!(err.to_string(), "checkpoint is not a regular file");
    }

    #[test]
    fn restore_unavailable_positions() {
        // Slot layout before the loss: a@0, b@1, one@2, two@3.
        let mut m = Message {
            images: vec!["one".to_owned(), "two".to_owned()],
            unavailable_images: vec![
                UnavailableImage {
                    index: 0,
                    reference: "sha256:a".to_owned(),
                },
                UnavailableImage {
                    index: 1,
                    reference: "sha256:b".to_owned(),
                },
            ],
            ..user_msg()
        };
        restore_unavailable_image(&mut m, "sha256:b", "B");
        assert_eq!(
            m.images,
            vec!["B".to_owned(), "one".to_owned(), "two".to_owned()]
        );
        assert_eq!(m.unavailable_images.len(), 1);
        restore_unavailable_image(&mut m, "sha256:a", "A");
        assert_eq!(
            m.images,
            vec![
                "A".to_owned(),
                "B".to_owned(),
                "one".to_owned(),
                "two".to_owned()
            ]
        );
        assert!(m.unavailable_images.is_empty());
        // An out-of-range index clamps to the end.
        let mut m2 = Message {
            images: vec!["one".to_owned()],
            unavailable_images: vec![UnavailableImage {
                index: 9,
                reference: "sha256:far".to_owned(),
            }],
            ..user_msg()
        };
        restore_unavailable_image(&mut m2, "sha256:far", "F");
        assert_eq!(m2.images, vec!["one".to_owned(), "F".to_owned()]);
    }

    #[test]
    fn refuse_degraded_checkpoint_guard() {
        let tmp = test_support::tempdir();
        let url = "data:image/png;base64,QQ==";
        let key = session_image_key(url);
        let msgs = [Message {
            unavailable_images: vec![UnavailableImage {
                index: 0,
                reference: key.clone(),
            }],
            ..user_msg()
        }];
        // A checkpoint that still inlines the image is refused.
        let checkpoint = tmp.path().join("s.jsonl");
        std::fs::write(&checkpoint, format!("{{\"images\":[\"{url}\"]}}")).unwrap();
        let err = refuse_degraded_checkpoint_rewrite(&checkpoint, &msgs).unwrap_err();
        assert_eq!(
            err.to_string(),
            "session image payload unavailable: the checkpoint holds the only copy of 1 image(s); it was left unchanged"
        );
        // A checkpoint without it allows the rewrite.
        std::fs::write(&checkpoint, "{\"images\":[]}").unwrap();
        assert!(refuse_degraded_checkpoint_rewrite(&checkpoint, &msgs).is_ok());
        // Messages without unavailable images never consult it.
        assert!(refuse_degraded_checkpoint_rewrite(&checkpoint, &[user_msg()]).is_ok());
        // A directory for a checkpoint fails closed as unreadable.
        let dir = tmp.path().join("d");
        std::fs::create_dir_all(&dir).unwrap();
        let err = refuse_degraded_checkpoint_rewrite(&dir, &msgs).unwrap_err();
        assert_eq!(
            err.to_string(),
            "session image payload unavailable: the checkpoint could not be read, so its images cannot be ruled out; it was left unchanged: checkpoint is not a regular file"
        );
    }

    #[test]
    fn holds_any_image_matches_only_inline_keys() {
        let url = "data:image/png;base64,QQ==";
        let lost = [UnavailableImage {
            index: 0,
            reference: session_image_key(url),
        }];
        assert!(holds_any_image(
            &Message {
                images: vec![url.to_owned()],
                ..user_msg()
            },
            &lost
        ));
        assert!(!holds_any_image(
            &Message {
                images: vec!["https://example.com/x.png".to_owned()],
                ..user_msg()
            },
            &lost
        ));
        assert!(!holds_any_image(
            &Message {
                images: vec!["data:image/png;base64,OTHER".to_owned()],
                ..user_msg()
            },
            &lost
        ));
    }

    #[test]
    fn backfill_restores_from_checkpoint() {
        let tmp = test_support::tempdir();
        let log_path = tmp.path().join("s.jsonl.events.jsonl");
        let session_path = crate::store::session_path_for_event_log(&log_path);
        let url = "data:image/png;base64,QQ==";
        let key = session_image_key(url);
        // The checkpoint still inlines the image.
        std::fs::write(&session_path, format!("{{\"images\":[\"{url}\"]}}")).unwrap();
        let mut msgs = [Message {
            unavailable_images: vec![UnavailableImage {
                index: 0,
                reference: key.clone(),
            }],
            ..user_msg()
        }];
        backfill_unavailable_images(&log_path, &mut msgs);
        assert_eq!(msgs[0].images, vec![url.to_owned()]);
        assert!(msgs[0].unavailable_images.is_empty());
        // The blob was republished for the next replay.
        let dir = crate::store::session_blobs_dir_for_event_log(&log_path);
        assert_eq!(read_session_image_blob(&dir, &key).as_deref(), Some(url));
    }

    #[test]
    fn salvaged_keys_scan_sidecars() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let key = session_image_key("data:x");
        std::fs::write(
            crate::store::session_superseded(&session),
            format!("before {key} after"),
        )
        .unwrap();
        // A partial key (short hex) does not count.
        std::fs::write(
            crate::store::session_context(&session),
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
        )
        .unwrap();
        let keys = salvaged_image_keys(&session).unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&key));
        // An unreadable sidecar stops all sweeping.
        std::fs::create_dir_all(crate::store::session_event_log_damaged(&session)).unwrap();
        assert!(salvaged_image_keys(&session).is_none());
    }

    #[test]
    fn sweep_removes_unreferenced_blobs_and_stale_temps() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("s.jsonl");
        let dir = crate::store::session_blobs_dir(&session);
        std::fs::create_dir_all(&dir).unwrap();
        let keep_url = "data:image/png;base64,KEEP";
        let keep_key = session_image_key(keep_url);
        std::fs::write(dir.join(&keep_key["sha256:".len()..]), keep_url).unwrap();
        // An orphan blob nothing names.
        let orphan = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(dir.join(orphan), "junk").unwrap();
        // A stale and a fresh atomic temp, and a non-blob file.
        let stale = dir.join(".atomic-stale.tmp");
        std::fs::write(&stale, "junk").unwrap();
        let fresh = dir.join(".atomic-fresh.tmp");
        std::fs::write(&fresh, "junk").unwrap();
        set_mtime_two_hours_past(&stale);
        std::fs::write(dir.join("README"), "x").unwrap();
        let refs = [SessionEventImages {
            message: 0,
            slots: vec![SessionImageSlot {
                blob: keep_key.clone(),
                url: String::new(),
            }],
        }];
        sweep_session_image_blobs(&session, &refs);
        assert!(dir.join(&keep_key["sha256:".len()..]).exists());
        assert!(!dir.join(orphan).exists());
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(dir.join("README").exists());
    }

    fn set_mtime_two_hours_past(path: &Path) {
        let file = File::options().write(true).open(path).unwrap();
        let past = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        file.set_times(std::fs::FileTimes::new().set_modified(past))
            .unwrap();
    }

    #[test]
    fn inline_image_bytes_sums_data_urls() {
        let msgs = [
            Message {
                images: vec![
                    "data:image/png;base64,QQ==".to_owned(),
                    "https://example.com/x.png".to_owned(),
                ],
                ..user_msg()
            },
            Message {
                images: vec!["data:image/gif;base64,R0lGOD".to_owned()],
                ..user_msg()
            },
        ];
        let want = "data:image/png;base64,QQ==".len() + "data:image/gif;base64,R0lGOD".len();
        assert_eq!(inline_image_bytes(&msgs), want as i64);
    }
}
