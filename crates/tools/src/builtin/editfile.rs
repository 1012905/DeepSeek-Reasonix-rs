//! The edit_file tool (editfile.go): one exact-string replacement with the
//! shared edit engine, the read policy, and the post-write receipt.

#![forbid(unsafe_code)]

use std::sync::Arc;

use serde_json::Value;

use super::confine::{
    ManagedConfigHook, SessionDataGuardHook, SessionTempSource, WriteHooks, confine_write,
};
use super::editengine::apply_old_string_edit;
use super::editsource::{
    EditReadPolicy, FileOverlay, FileViewsHook, wrap_read_error, wrap_write_error,
};
use super::fuzzyedit::{old_string_not_found_error, old_string_not_unique_error};
use super::receipt::with_actual_post_write_receipts;
use super::writepaths::{resolve_in, resolve_session_temp};
use crate::abi::{CallContext, Capability, CapabilitySet, PathWriter, Tool, ToolResult};

/// editfile.go `editFile`. roots confines the target to the workspace when
/// non-empty; guard rejects Reasonix session-data targets; workDir, when
/// non-empty, is the directory a relative path resolves against.
#[derive(Default)]
pub struct EditFileTool {
    pub read_policy: EditReadPolicy,
    pub roots: Vec<String>,
    pub guard: Option<Arc<dyn SessionDataGuardHook>>,
    pub managed: Option<Arc<dyn ManagedConfigHook>>,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
    pub work_dir: String,
    pub overlay: Option<Arc<dyn FileOverlay>>,
    pub views: Option<Arc<dyn FileViewsHook>>,
}

impl EditFileTool {
    fn hooks(&self) -> WriteHooks {
        WriteHooks {
            guard: self.guard.clone(),
            managed: self.managed.clone(),
            session_temp: self.session_temp.clone(),
        }
    }

    fn edit(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let path_arg = args.get("path").and_then(Value::as_str).unwrap_or("");
        let old = args.get("old_string").and_then(Value::as_str).unwrap_or("");
        let new = args.get("new_string").and_then(Value::as_str).unwrap_or("");
        if path_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("path is required")));
        }
        if old.is_empty() {
            return Err(Box::new(std::io::Error::other("old_string is required")));
        }
        let path = resolve_in(
            &self.work_dir,
            &resolve_session_temp(self.session_temp.as_deref(), path_arg),
        );
        confine_write(&self.roots, &self.hooks(), &path)?;

        let src = self
            .read_policy
            .source(self.overlay.as_deref(), &path)
            .map_err(|e| wrap_read_error(&path, e))?;

        let applied = apply_old_string_edit(&src.content, old.as_bytes(), new.as_bytes(), false);
        if applied.applied != 1 {
            if applied.matches == 0 {
                return Err(Box::new(std::io::Error::other(old_string_not_found_error(
                    &path,
                    old.as_bytes(),
                    &src.content,
                ))));
            }
            return Err(Box::new(std::io::Error::other(
                old_string_not_unique_error(
                    &path,
                    old.as_bytes(),
                    &src.content,
                    applied.matches,
                    false,
                ),
            )));
        }

        src.write(self.overlay.as_deref(), &path, &applied.updated)
            .map_err(|e| wrap_write_error(&path, e))?;
        if let Some(v) = self.views.as_ref() {
            v.saw(&path, &applied.updated);
        }
        let mut summary = format!("edited {path}");
        if applied.fuzzy {
            summary.push_str(" (fuzzy match)");
        }
        Ok(with_actual_post_write_receipts(
            &summary,
            &[applied.receipt],
        ))
    }
}

struct PathWriterCap;

impl Capability for PathWriterCap {}

impl PathWriter for PathWriterCap {
    fn writes_named_paths(&self) -> bool {
        true
    }
}

impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace an exact string in a file with another. old_string must occur exactly once; add surrounding context to disambiguate. Use for targeted edits instead of rewriting the whole file."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "old_string": {"type": "string", "description": "Exact text to replace (must be unique in the file)"},
                "new_string": {"type": "string", "description": "Replacement text (may be empty to delete)"}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.edit(ctx, args)
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
