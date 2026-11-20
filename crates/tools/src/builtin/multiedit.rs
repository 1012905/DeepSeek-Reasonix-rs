//! The multi_edit tool (multiedit.go): a batch of edits applied against
//! each other in memory, the file rewritten only if every edit lands — a
//! failure in step 3 leaves the file untouched instead of half-edited.

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
use super::fuzzyedit::{
    multi_edit_failure, old_string_not_found_error, old_string_not_unique_error,
};
use super::receipt::with_actual_post_write_receipts;
use super::writepaths::{resolve_in, resolve_session_temp};
use crate::abi::{CallContext, Capability, CapabilitySet, PathWriter, Tool, ToolResult};

/// multiedit.go `editStep`: one edit in a multi_edit operation. Mirrors
/// edit_file's args plus a per-step replace_all toggle so a single call
/// can mix targeted and sweep replacements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditStep {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

impl EditStep {
    fn parse(step: &Value) -> EditStep {
        EditStep {
            old_string: step
                .get("old_string")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            new_string: step
                .get("new_string")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            replace_all: step
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

/// multiedit.go `multiEdit`.
#[derive(Default)]
pub struct MultiEditTool {
    pub read_policy: EditReadPolicy,
    pub roots: Vec<String>,
    pub guard: Option<Arc<dyn SessionDataGuardHook>>,
    pub managed: Option<Arc<dyn ManagedConfigHook>>,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
    pub work_dir: String,
    pub overlay: Option<Arc<dyn FileOverlay>>,
    pub views: Option<Arc<dyn FileViewsHook>>,
}

impl MultiEditTool {
    fn hooks(&self) -> WriteHooks {
        WriteHooks {
            guard: self.guard.clone(),
            managed: self.managed.clone(),
            session_temp: self.session_temp.clone(),
        }
    }

    fn multi_edit(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let path_arg = args.get("path").and_then(Value::as_str).unwrap_or("");
        let edits: Vec<EditStep> = args
            .get("edits")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().map(EditStep::parse).collect())
            .unwrap_or_default();
        if path_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("path is required")));
        }
        if edits.is_empty() {
            return Err(Box::new(std::io::Error::other("edits must not be empty")));
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
        let mut content = src.content.clone();

        // A failed step leaves the buffer as it was and the sweep goes on,
        // so one call reports every bad step; any failure still returns
        // before the write.
        let mut applied = 0usize;
        let mut used_fuzzy = false;
        let mut receipts = Vec::with_capacity(edits.len());
        let mut failures: Vec<String> = Vec::new();
        for (i, step) in edits.iter().enumerate() {
            if step.old_string.is_empty() {
                failures.push(format!("edit {}: old_string is required", i + 1));
                continue;
            }
            let result = apply_old_string_edit(
                &content,
                step.old_string.as_bytes(),
                step.new_string.as_bytes(),
                step.replace_all,
            );
            if result.applied > 0 {
                content = result.updated;
                applied += result.applied;
                used_fuzzy = used_fuzzy || result.fuzzy;
                receipts.push(result.receipt);
            } else if result.matches == 0 {
                failures.push(format!(
                    "edit {}: {}",
                    i + 1,
                    old_string_not_found_error(&path, step.old_string.as_bytes(), &content)
                ));
            } else {
                failures.push(format!(
                    "edit {}: {}",
                    i + 1,
                    old_string_not_unique_error(
                        &path,
                        step.old_string.as_bytes(),
                        &content,
                        result.matches,
                        true
                    )
                ));
            }
        }
        if let Some(msg) = multi_edit_failure(&path, edits.len(), &failures) {
            return Err(Box::new(std::io::Error::other(msg)));
        }

        src.write(self.overlay.as_deref(), &path, &content)
            .map_err(|e| wrap_write_error(&path, e))?;
        if let Some(v) = self.views.as_ref() {
            v.saw(&path, &content);
        }
        let mut summary = format!(
            "multi_edit {path}: {} edits applied ({applied} total replacements)",
            edits.len()
        );
        if used_fuzzy {
            summary.push_str(" (fuzzy match)");
        }
        Ok(with_actual_post_write_receipts(&summary, &receipts))
    }
}

struct PathWriterCap;

impl Capability for PathWriterCap {}

impl PathWriter for PathWriterCap {
    fn writes_named_paths(&self) -> bool {
        true
    }
}

impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn description(&self) -> &str {
        "Apply a list of edits to a single file atomically: each edit runs against the result of the previous one, all in memory; the file is rewritten only if every edit succeeds. Cheaper and safer than chaining edit_file calls — a failure in step 3 leaves the file untouched instead of half-edited."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ordered edits. Each step sees the file as left by the previous step.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": {"type": "string", "description": "Exact text to find. Without replace_all, must match exactly once."},
                            "new_string": {"type": "string", "description": "Replacement text (empty deletes)."},
                            "replace_all": {"type": "boolean", "description": "Replace every occurrence instead of requiring uniqueness."}
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.multi_edit(ctx, args)
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
