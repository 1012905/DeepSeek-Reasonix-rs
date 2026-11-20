//! The move_file tool (movefile.go, movefile_windows.go): rename with the
//! same-file-destination and cross-filesystem fallbacks, both endpoints
//! confined and the source under the read policy.

#![forbid(unsafe_code)]

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use super::confine::{
    ManagedConfigHook, SessionDataGuardHook, SessionTempSource, WriteHooks, confine_write,
};
use super::editsource::check_read_target_access;
use super::writepaths::{resolve_in, resolve_session_temp};
use crate::abi::{CallContext, Capability, CapabilitySet, PathWriter, Tool, ToolResult};
use reasonix_core::fileutil::{create_temp_file, mkdir_all_mode};

/// movefile.go's `var renameFile = os.Rename` seam: tests replace it to
/// force rename failures (same-file destination, cross-volume).
pub type RenameFn = Arc<dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync>;

pub fn std_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// movefile.go `moveFile`. roots, when non-empty, confine both the source
/// and destination to the workspace; guard rejects Reasonix session-data
/// endpoints on either side (a move out of the store mutates it too);
/// workDir resolves relative paths.
pub struct MoveFileTool {
    pub roots: Vec<String>,
    pub read_roots: Vec<String>,
    pub forbid_roots: Vec<String>,
    pub guard: Option<Arc<dyn SessionDataGuardHook>>,
    pub managed: Option<Arc<dyn ManagedConfigHook>>,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
    pub work_dir: String,
    pub rename: RenameFn,
}

impl Default for MoveFileTool {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            read_roots: Vec::new(),
            forbid_roots: Vec::new(),
            guard: None,
            managed: None,
            session_temp: None,
            work_dir: String::new(),
            rename: Arc::new(std_rename),
        }
    }
}

impl MoveFileTool {
    fn hooks(&self) -> WriteHooks {
        WriteHooks {
            guard: self.guard.clone(),
            managed: self.managed.clone(),
            session_temp: self.session_temp.clone(),
        }
    }

    fn move_file(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let src_arg = args
            .get("source_path")
            .and_then(Value::as_str)
            .unwrap_or("");
        let dst_arg = args
            .get("destination_path")
            .and_then(Value::as_str)
            .unwrap_or("");
        if src_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("source_path is required")));
        }
        if dst_arg.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "destination_path is required",
            )));
        }
        let src = resolve_in(
            &self.work_dir,
            &resolve_session_temp(self.session_temp.as_deref(), src_arg),
        );
        let dst = resolve_in(
            &self.work_dir,
            &resolve_session_temp(self.session_temp.as_deref(), dst_arg),
        );
        confine_write(&self.roots, &self.hooks(), &src)?;
        confine_write(&self.roots, &self.hooks(), &dst)?;
        // A move hands its content to the destination, so a source under a
        // forbidden-read root is refused whether the filesystem renames it
        // or has to copy it. The destination is only written, never read.
        if let Err(r) = check_read_target_access(&self.read_roots, &self.forbid_roots, &src) {
            return Err(Box::new(r));
        }
        let info =
            fs::metadata(&src).map_err(|e| std::io::Error::other(format!("stat {src}: {e}")))?;
        if info.is_dir() {
            return Err(Box::new(std::io::Error::other(format!(
                "{src} is a directory; move_file only moves files"
            ))));
        }
        if super::pathgo::clean(&src) == super::pathgo::clean(&dst) {
            return Ok(format!("{src} is already at {dst}; no changes made"));
        }
        let same_file_destination = match fs::metadata(&dst) {
            Ok(dst_info) if !same_file(&src, &dst, &info, &dst_info) => {
                return Err(Box::new(std::io::Error::other(format!(
                    "destination {dst} already exists"
                ))));
            }
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
            Err(e) => {
                return Err(Box::new(std::io::Error::other(format!("stat {dst}: {e}"))));
            }
        };
        if let Some(dir) = Path::new(&dst)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
        {
            mkdir_all_mode(dir, 0o755)
                .map_err(|e| std::io::Error::other(format!("mkdir {}: {e}", dir.display())))?;
        }
        match (self.rename)(Path::new(&src), Path::new(&dst)) {
            Ok(()) => Ok(format!("moved {src} to {dst}")),
            Err(err) => {
                if same_file_destination {
                    return match rename_same_file_destination(
                        Path::new(&src),
                        Path::new(&dst),
                        self.rename.clone(),
                    ) {
                        Ok(()) => Ok(format!("moved {src} to {dst}")),
                        Err(e) => Err(move_error(&src, &dst, e)),
                    };
                }
                if is_cross_device(&err) {
                    return match copy_regular_file_and_remove_source(
                        Path::new(&src),
                        Path::new(&dst),
                        &info,
                    ) {
                        Ok(()) => Ok(format!("moved {src} to {dst}")),
                        Err(e) => Err(move_error(&src, &dst, e)),
                    };
                }
                Err(move_error(&src, &dst, err))
            }
        }
    }
}

fn move_error(src: &str, dst: &str, e: io::Error) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(format!("move {src} to {dst}: {e}")))
}

/// The OS code a cross-volume rename returns; the message Windows pairs
/// with it is localized, so the move is recognizable only by its code.
#[cfg(windows)]
const CROSS_DEVICE_ERRNO: i32 = 17; // ERROR_NOT_SAME_DEVICE
#[cfg(not(windows))]
const CROSS_DEVICE_ERRNO: i32 = 18; // EXDEV

pub fn is_cross_device(err: &io::Error) -> bool {
    err.raw_os_error() == Some(CROSS_DEVICE_ERRNO)
}

/// Go `os.SameFile`. Unix compares device and inode. Windows std exposes
/// no stable file index, so equal canonical paths stand in — that covers
/// the case-only rename this check guards.
fn same_file(src: &str, dst: &str, a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(windows)]
    {
        let _ = (a, b);
        let canon = |p: &str| {
            fs::canonicalize(p).ok().map(|c| {
                c.to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .to_lowercase()
            })
        };
        match (canon(src), canon(dst)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

/// movefile.go `renameSameFileDestination`: when rename onto an existing
/// same-file destination fails, hop through a sibling temp name, drop the
/// destination, and rename into place, restoring the source on failure.
fn rename_same_file_destination(src: &Path, dst: &Path, rename: RenameFn) -> io::Result<()> {
    let dir = src
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (tmp, tmp_name) = create_temp_file(dir, ".reasonix-move-", "")?;
    drop(tmp);
    fs::remove_file(&tmp_name)?;
    rename(src, &tmp_name)?;
    let restore = |e: io::Error| -> io::Error {
        if let Err(re) = rename(&tmp_name, src) {
            return io::Error::other(format!("{e}; restore {}: {re}", src.display()));
        }
        e
    };
    match fs::remove_file(dst) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(restore(e)),
    }
    rename(&tmp_name, dst).map_err(restore)
}

/// movefile.go `copyRegularFileAndRemoveSource`: the cross-filesystem
/// fallback. Regular files only; the destination is created exclusively so
/// a concurrent writer wins, and removed again when anything fails.
fn copy_regular_file_and_remove_source(
    src: &Path,
    dst: &Path,
    info: &fs::Metadata,
) -> io::Result<()> {
    if !info.is_file() {
        return Err(io::Error::other(
            "cross-filesystem fallback only supports regular files",
        ));
    }
    let mut input = fs::File::open(src)?;
    let result = copy_to_exclusive(dst, info, &mut input);
    match result {
        Ok(()) => fs::remove_file(src),
        Err(e) => {
            let _ = fs::remove_file(dst);
            Err(e)
        }
    }
}

fn copy_to_exclusive(dst: &Path, info: &fs::Metadata, input: &mut fs::File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let perm = info.permissions().mode() & 0o777;
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(perm)
            .open(dst)?;
        io::copy(input, &mut out)?;
        // Go checks the writer's Close error; std has no fallible close,
        // so the equivalent check is a final sync.
        out.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = info;
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst)?;
        io::copy(input, &mut out)?;
        out.sync_all()?;
        Ok(())
    }
}

struct PathWriterCap;

impl Capability for PathWriterCap {}

impl PathWriter for PathWriterCap {
    fn writes_named_paths(&self) -> bool {
        true
    }
}

impl Tool for MoveFileTool {
    fn name(&self) -> &str {
        "move_file"
    }

    fn description(&self) -> &str {
        "Move or rename a file from source_path to destination_path. Creates the destination parent directory as needed. Use instead of shell mv, Move-Item, or ren for file moves so workspace confinement and file-edit permissions apply."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_path": {"type": "string", "description": "Existing file path to move"},
                "destination_path": {"type": "string", "description": "Destination file path; must not already exist"}
            },
            "required": ["source_path", "destination_path"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.move_file(ctx, args)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with::<dyn PathWriter>(
            "path_writer",
            Arc::new(PathWriterCap) as Arc<dyn PathWriter>,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
