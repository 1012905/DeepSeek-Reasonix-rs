//! The write_file tool (writefile.go): full-content writes with
//! encoding preservation, parent-directory creation, the same-content
//! no-op, and the write-side confinement boundary.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use reasonix_core::fileencoding::Kind;
use reasonix_core::fileutil::mkdir_all_mode;
use serde_json::Value;

use super::confine::{
    ManagedConfigHook, SessionDataGuardHook, SessionTempSource, WriteHooks, confine_write,
};
use super::editsource::{
    FileOverlay, FileViewsHook, WriteReceipt, WriteWrapError, source_for_write, write_file_encoded,
};
use super::writeatomic::atomic_create_file;
use super::writepaths::{resolve_in, resolve_session_temp};
use crate::abi::{CallContext, Capability, CapabilitySet, PathWriter, Tool, ToolResult};

/// writefile.go `writeFile`. roots, when non-empty, confines the target to
/// the workspace; guard rejects Reasonix session-data targets even inside
/// the roots; workDir, when non-empty, is the directory a relative path
/// resolves against. The zero-value instance is unconfined and is
/// overridden per run by confine_writers.
#[derive(Default)]
pub struct WriteFileTool {
    pub roots: Vec<String>,
    pub read_roots: Vec<String>,
    pub forbid_roots: Vec<String>,
    pub guard: Option<Arc<dyn SessionDataGuardHook>>,
    pub managed: Option<Arc<dyn ManagedConfigHook>>,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
    pub work_dir: String,
    /// Routes the write through the host transport so an open editor
    /// buffer updates too. Consulted only after write confinement, and
    /// only for plain-UTF-8 targets (the overlay is text-only, so
    /// non-UTF-8 files keep the local encoding-preserving path).
    pub overlay: Option<Arc<dyn FileOverlay>>,
    /// Per-runtime effect hook: hadPrior means an existing file was
    /// overwritten; prior is its previous content.
    pub receipt: Option<WriteReceipt>,
    pub views: Option<Arc<dyn FileViewsHook>>,
}

impl WriteFileTool {
    fn hooks(&self) -> WriteHooks {
        WriteHooks {
            guard: self.guard.clone(),
            managed: self.managed.clone(),
            session_temp: self.session_temp.clone(),
        }
    }

    fn write(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let path_arg = args.get("path").and_then(Value::as_str).unwrap_or("");
        let content = args.get("content").and_then(Value::as_str).unwrap_or("");
        if path_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("path is required")));
        }
        let path = resolve_in(
            &self.work_dir,
            &resolve_session_temp(self.session_temp.as_deref(), path_arg),
        );
        confine_write(&self.roots, &self.hooks(), &path)?;

        // Preserve the existing file's encoding (GBK/UTF-16/BOM) on
        // overwrite instead of always writing UTF-8, which would silently
        // corrupt a non-UTF-8 file. A missing file yields enc=UTF-8 — the
        // right default for a new one.
        let (src, had_prior) = source_for_write(
            &self.read_roots,
            &self.forbid_roots,
            self.overlay.as_deref(),
            &path,
        )
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        if had_prior && src.content == content.as_bytes() {
            return Ok(format!(
                "{path} already contains the exact content; no changes made"
            ));
        }
        if had_prior
            && let Some(views) = self.views.as_ref()
            && let Err(e) = views.check_overwrite(&path, &src.content)
        {
            return Err(e);
        }

        // The host overlay applies the write to the editor buffer and the
        // file in one step. Text-only, so it handles plain UTF-8 targets
        // (and new files); non-UTF-8 files stay on the local
        // encoding-preserving path below.
        if let Some(o) = self.overlay.as_deref()
            && super::pathgo::is_abs(&path)
            && src.enc == Kind::Utf8
        {
            match o.write_text_file(&path, content) {
                Ok(true) => {
                    if let Some(r) = self.receipt.as_ref() {
                        r(&path, had_prior, &src.content);
                    }
                    if let Some(v) = self.views.as_ref() {
                        v.saw(&path, content.as_bytes());
                    }
                    return Ok(format!("wrote {} bytes to {}", content.len(), path));
                }
                Ok(false) => {}
                Err(e) => {
                    return Err(Box::new(std::io::Error::other(format!(
                        "write {path}: {e}"
                    ))));
                }
            }
        }

        if let Some(dir) = parent_dir(&path) {
            mkdir_all_mode(Path::new(&dir), 0o755)
                .map_err(|e| std::io::Error::other(format!("mkdir {dir}: {e}")))?;
        }
        if had_prior {
            write_file_encoded(&path, content.as_bytes(), src.enc).map_err(|e| {
                Box::new(WriteWrapError {
                    path: path.clone(),
                    source: e,
                }) as Box<dyn std::error::Error + Send + Sync>
            })?;
        } else {
            atomic_create_file(Path::new(&path), content.as_bytes(), 0o644).map_err(|e| {
                Box::new(std::io::Error::other(format!("write {path}: {e}")))
                    as Box<dyn std::error::Error + Send + Sync>
            })?;
        }
        let prior: &[u8] = if had_prior { &src.content } else { &[] };
        if let Some(r) = self.receipt.as_ref() {
            r(&path, had_prior, prior);
        }
        if let Some(v) = self.views.as_ref() {
            v.saw(&path, content.as_bytes());
        }
        Ok(format!("wrote {} bytes to {}", content.len(), path))
    }
}

/// Go `filepath.Dir`, empty and "." excluded.
fn parent_dir(path: &str) -> Option<String> {
    let (parent, _) = super::pathgo::split(path);
    if parent.is_empty() || parent == "." {
        return None;
    }
    Some(parent)
}

struct PathWriterCap;

impl Capability for PathWriterCap {}

impl PathWriter for PathWriterCap {
    fn writes_named_paths(&self) -> bool {
        true
    }
}

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write full file content, overwriting and creating parent directories. Prefer edit_file/multi_edit for partial changes."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.write(ctx, args)
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
