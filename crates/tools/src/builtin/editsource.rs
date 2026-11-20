//! Read-modify-write plumbing for the file tools (editsource.go,
//! edit_read_policy.go, writefile_source.go, target_access.go): the source
//! route a tool reads from and writes back to, and the read-policy
//! refusals both the edit tools and write_file's overwrite path enforce.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use reasonix_core::fileencoding::{Kind, detect_and_decode};

use super::confine::{CODE_READ_OUTSIDE_SCOPE, confine_read, read_outside_scope};
use super::fileencode;
use super::writeatomic::atomic_overwrite_file;
use crate::refusal::Refusal;

/// The host overlay trait (editsource.go `FileOverlay`): serves a file's
/// unsaved editor buffer. Text-only — non-UTF-8 files stay on the disk
/// route. `None` is Go nil, and the tools then read and write the disk.
pub trait FileOverlay: Send + Sync {
    /// The host's unsaved buffer for path, when it holds one.
    fn read_text_file(&self, path: &str) -> Option<String>;
    /// Applies the write to the buffer and the file in one step.
    /// `Ok(false)` declines (the caller falls back to disk); `Ok(true)`
    /// means the buffer, and the file, hold the new content.
    fn write_text_file(&self, path: &str, content: &str) -> std::io::Result<bool>;
}

/// The file-view hook (fileviews.go `FileViews`, the slice of it the
/// writers need): what the agent last saw of each file. The full machine
/// (hashes, partial-read records) lands with TOOL-054; hooks keep the
/// writers' call sites faithful meanwhile.
pub trait FileViewsHook: Send + Sync {
    fn saw(&self, path: &str, content: &[u8]);
    fn check_overwrite(
        &self,
        path: &str,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// The per-runtime write receipt (writefile.go `BindFileWriteReceipt`):
/// `had_prior` means an existing file was overwritten; `prior` is its
/// previous content.
pub type WriteReceipt = Arc<dyn Fn(&str, bool, &[u8]) + Send + Sync>;

/// editsource.go `editSource`: the file state a read-modify-write tool
/// works against, plus the route its write must take back. Read and write
/// must stay paired: content read from the host's unsaved buffer has to
/// return there, and content read from disk has to return to disk. Mixing
/// the two silently drops one side's changes.
#[derive(Debug, Clone)]
pub struct EditSource {
    pub content: Vec<u8>,
    pub enc: Kind,
    pub overlay: bool,
}

/// editsource.go `readEditSource`: the host's unsaved editor buffer when
/// the overlay can serve it, otherwise the decoded disk content. A
/// non-UTF-8 file always stays on the disk route — the overlay contract is
/// text-only, so routing GBK or UTF-16 through it would rewrite the file
/// as UTF-8.
pub fn read_edit_source(
    overlay: Option<&dyn FileOverlay>,
    path: &str,
) -> std::io::Result<EditSource> {
    let data = std::fs::read(path)?;
    let (enc, text) = detect_and_decode(&data);
    let content = text.into_owned();
    if let Some(o) = overlay
        && enc == Kind::Utf8
        && super::pathgo::is_abs(path)
        && let Some(buffered) = o.read_text_file(path)
    {
        return Ok(EditSource {
            content: buffered.into_bytes(),
            enc,
            overlay: true,
        });
    }
    Ok(EditSource {
        content,
        enc,
        overlay: false,
    })
}

impl EditSource {
    /// editsource.go `write`: persists content on the same route the
    /// source was read from. An overlay that declines the write falls back
    /// to disk, which is safe because the content already carries whatever
    /// the buffer held.
    pub fn write(
        &self,
        overlay: Option<&dyn FileOverlay>,
        path: &str,
        content: &[u8],
    ) -> Result<(), WriteFileError> {
        if self.overlay
            && let Some(o) = overlay
            && let Ok(text) = std::str::from_utf8(content)
            && o.write_text_file(path, text)?
        {
            return Ok(());
        }
        write_file_encoded(path, content, self.enc)
    }
}

/// encoding_helpers.go `writeFileEncoded`: encode content back to the
/// given encoding and write it atomically — a truncating write that fails
/// midway (a Windows filter driver holding a transient lock, a full disk)
/// would leave the user's source file empty or half-written.
pub fn write_file_encoded(path: &str, content: &[u8], enc: Kind) -> Result<(), WriteFileError> {
    let data = fileencode::encode_bytes(content, enc)?;
    atomic_overwrite_file(Path::new(path), &data, 0o644)?;
    Ok(())
}

/// The failures a source read or write answers with. Go wraps them as
/// "read %s: %w" / "write %s: %w"; the typed identity stays downcastable
/// through [`ReadWrapError`] / [`WriteWrapError`].
#[derive(Debug, thiserror::Error)]
pub enum WriteFileError {
    #[error(transparent)]
    Unencodable(#[from] fileencode::UnencodableError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{0}")]
    Policy(#[source] Refusal),
    #[error("{0}")]
    Io(#[source] WriteFileError),
}

/// Go `fmt.Errorf("read %s: %w", path, err)`, keeping the source
/// downcastable.
#[derive(Debug, thiserror::Error)]
#[error("read {path}: {source}")]
pub struct ReadWrapError {
    pub path: String,
    #[source]
    pub source: SourceError,
}

/// Go `fmt.Errorf("write %s: %w", path, err)`, keeping the source
/// downcastable.
#[derive(Debug, thiserror::Error)]
#[error("write {path}: {source}")]
pub struct WriteWrapError {
    pub path: String,
    #[source]
    pub source: WriteFileError,
}

/// target_access.go `workspace.read_forbidden`: a target denied by the
/// workspace read policy.
pub const CODE_READ_FORBIDDEN: &str = "workspace.read_forbidden";

/// Overwrite refusals retain the read-policy cause while naming the write
/// action (target_access.go).
pub const CODE_OVERWRITE_READ_FORBIDDEN: &str = "workspace.overwrite_read_forbidden";
pub const CODE_OVERWRITE_READ_OUTSIDE_SCOPE: &str = "workspace.overwrite_read_outside_scope";

pub const OVERWRITE_READ_ACCESS_DENIED_MESSAGE: &str =
    "Cannot overwrite target file: permission to read existing contents is required.";

/// target_access.go `checkReadTargetAccess`: outside the read roots first,
/// then the deny list.
pub fn check_read_target_access(
    read_roots: &[String],
    forbidden: &[String],
    path: &str,
) -> Result<(), Refusal> {
    if read_outside_scope(read_roots, path) {
        return Err(Refusal::code(
            CODE_READ_OUTSIDE_SCOPE,
            read_outside_scope_message(path, read_roots),
        ));
    }
    if confine_read(forbidden, path) {
        return Err(Refusal::code(
            CODE_READ_FORBIDDEN,
            read_forbidden_message(path),
        ));
    }
    Ok(())
}

/// target_access.go `readOutsideScopeMessage`: names the target and the
/// folders this session may read. It depends only on the policy and the
/// path, never on whether the file exists, so it tells the model what to
/// do without saying anything about the disk.
fn read_outside_scope_message(path: &str, read_roots: &[String]) -> String {
    if read_roots.is_empty() {
        return format!(
            "Cannot read `{path}`: it is outside the folders this session may read. Read a path inside the workspace, or ask the user to add the folder."
        );
    }
    format!(
        "Cannot read `{path}`: it is outside the folders this session may read ({}). Read a path inside one of them, or ask the user to add the folder.",
        read_roots.join(", ")
    )
}

/// target_access.go `readForbiddenMessage`: says the path is on the
/// read-forbidden list without naming which entry matched — that list is a
/// protection the user configured.
fn read_forbidden_message(path: &str) -> String {
    format!(
        "Cannot read `{path}`: it is on this session's forbidden-read list, a protection the user configured, so it cannot be read here."
    )
}

/// target_access.go `overwriteReadRefusal`: a read-policy refusal becomes
/// the overwrite form; anything else passes through.
pub fn overwrite_read_refusal(refusal: Refusal) -> Refusal {
    match refusal.code.as_str() {
        CODE_READ_FORBIDDEN => Refusal::code(
            CODE_OVERWRITE_READ_FORBIDDEN,
            OVERWRITE_READ_ACCESS_DENIED_MESSAGE,
        ),
        CODE_READ_OUTSIDE_SCOPE => Refusal::code(
            CODE_OVERWRITE_READ_OUTSIDE_SCOPE,
            OVERWRITE_READ_ACCESS_DENIED_MESSAGE,
        ),
        _ => refusal,
    }
}

/// target_access.go `checkOverwriteReadAccess`.
pub fn check_overwrite_read_access(
    read_roots: &[String],
    forbidden: &[String],
    path: &str,
) -> Result<(), Refusal> {
    match check_read_target_access(read_roots, forbidden, path) {
        Ok(()) => Ok(()),
        Err(r) => Err(overwrite_read_refusal(r)),
    }
}

/// writefile_source.go `existingWriteSource`.
pub fn existing_write_source(path: &str) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// writefile_source.go `sourceForWrite`: a missing file yields enc=UTF-8 —
/// the right default for a new one. Reading via the overlay makes the
/// no-op check see the same buffer Preview does.
pub fn source_for_write(
    read_roots: &[String],
    forbid_roots: &[String],
    overlay: Option<&dyn FileOverlay>,
    path: &str,
) -> Result<(EditSource, bool), SourceError> {
    let exists = existing_write_source(path)
        .map_err(WriteFileError::Io)
        .map_err(SourceError::Io)?;
    if !exists {
        return Ok((
            EditSource {
                content: Vec::new(),
                enc: Kind::Utf8,
                overlay: false,
            },
            false,
        ));
    }
    check_overwrite_read_access(read_roots, forbid_roots, path).map_err(SourceError::Policy)?;
    read_edit_source(overlay, path)
        .map(|src| (src, true))
        .map_err(|e| SourceError::Io(WriteFileError::Io(e)))
}

/// Go `fmt.Errorf("read %s: %w", ...)` as a value.
pub fn wrap_read_error(path: &str, e: SourceError) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(ReadWrapError {
        path: path.to_owned(),
        source: e,
    })
}

/// Go `fmt.Errorf("write %s: %w", ...)` as a value.
pub fn wrap_write_error(path: &str, e: WriteFileError) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(WriteWrapError {
        path: path.to_owned(),
        source: e,
    })
}

/// edit_read_policy.go `editReadPolicy`.
#[derive(Clone, Default)]
pub struct EditReadPolicy {
    pub read_roots: Vec<String>,
    pub forbid_roots: Vec<String>,
}

impl EditReadPolicy {
    pub fn source(
        &self,
        overlay: Option<&dyn FileOverlay>,
        path: &str,
    ) -> Result<EditSource, SourceError> {
        check_read_target_access(&self.read_roots, &self.forbid_roots, path)
            .map_err(SourceError::Policy)?;
        read_edit_source(overlay, path).map_err(|e| SourceError::Io(WriteFileError::Io(e)))
    }
}
