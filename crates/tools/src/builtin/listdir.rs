//! The ls tool (ls.go): a directory listing with sizes, and the recursive
//! depth-first walk with the noise-dir pruning.

#![forbid(unsafe_code)]

use serde_json::Value;

use super::confine::{confine_read, confine_scope};
use super::network::resolve_readable_path;
use super::pathgo;
use super::walk::{WalkEntry, WalkFlow, walk_dir};
use crate::abi::{CallContext, Tool, ToolResult};

/// The ls tool. Scopes mirror Go's listDir fields.
pub struct ListDirTool {
    pub net_roots: Vec<String>,
    pub work_dir: String,
    pub forbid_roots: Vec<String>,
    pub read_roots: Vec<String>,
}

impl ListDirTool {
    fn execute_inner(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let mut path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_owned();
        let recursive = args
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.is_empty() {
            path = String::from(".");
        }
        let rp = resolve_readable_path(&self.work_dir, &path);
        if let Some(r) = rp.refuse_network(&self.net_roots) {
            return Err(Box::new(r));
        }
        if let Some(r) = confine_scope(&self.read_roots, &rp.path) {
            return Err(Box::new(r));
        }
        if confine_read(&self.forbid_roots, &rp.path) {
            return Ok(String::from("(empty directory)"));
        }
        if recursive {
            return self.list_recursive(&rp.path, &rp);
        }
        let entries = match std::fs::read_dir(&rp.path) {
            Ok(e) => e,
            Err(err) => {
                return if rp.external {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("ls {}: {}", rp.display_path, rp.error_text(&err)),
                    )))
                } else {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("ls {}: {err}", rp.display_path),
                    )))
                };
            }
        };
        let mut listed: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        listed.sort();
        let mut b = String::new();
        for name in listed {
            let full = pathgo::join(&[&rp.path, &name]);
            // os.ReadDir entries judge IsDir/Info from lstat: a link to a
            // directory is listed as a file, like Go.
            match std::fs::symlink_metadata(&full) {
                Ok(m) if m.is_dir() => {
                    b.push_str(&format!("{name}/\n"));
                }
                Ok(m) => {
                    b.push_str(&format!("{name}\t{}\n", m.len()));
                }
                Err(_) => {
                    b.push_str(&format!("{name}\t-1\n"));
                }
            }
        }
        if b.is_empty() {
            return Ok(String::from("(empty directory)"));
        }
        Ok(b)
    }

    /// Recursive mode: the whole tree depth-first, noise dirs skipped, depth
    /// capped to guard against symlink loops.
    fn list_recursive(&self, root: &str, rp: &super::network::ResolvedPath) -> ToolResult {
        let mut b = String::new();
        let result = walk_dir(root, &mut |e: &WalkEntry| {
            if let Some(err) = &e.err {
                return Err(std::io::Error::new(err.kind(), err.to_string()));
            }
            if e.path == root {
                return Ok(WalkFlow::Continue);
            }
            if e.is_dir {
                match e.name.as_str() {
                    ".git" | "node_modules" | ".DS_Store" | "__pycache__" | ".idea" | ".vscode" => {
                        return Ok(WalkFlow::SkipDir);
                    }
                    _ => {}
                }
                if super::walk::skip_forbid_dir(&e.path, &self.forbid_roots) {
                    return Ok(WalkFlow::SkipDir);
                }
            }
            let rel = match pathgo::rel(root, &e.path) {
                Ok(r) => r,
                Err(_) => e.path.clone(),
            };
            if rel.matches(pathgo::separator()).count() > 50 {
                return if e.is_dir {
                    Ok(WalkFlow::SkipDir)
                } else {
                    Ok(WalkFlow::Continue)
                };
            }
            let rel = pathgo::to_slash(&rel);
            let line = if e.is_dir {
                format!("{rel}/")
            } else {
                match std::fs::metadata(&e.path) {
                    Ok(m) => format!("{rel}\t{}", m.len()),
                    Err(_) => rel.clone(),
                }
            };
            b.push_str(&line);
            b.push('\n');
            Ok(WalkFlow::Continue)
        });
        if let Err(err) = result {
            return if rp.external {
                Err(Box::new(std::io::Error::new(
                    err.kind(),
                    format!("ls -R {}: {}", rp.display_path, rp.error_text(&err)),
                )))
            } else {
                Err(Box::new(std::io::Error::new(
                    err.kind(),
                    format!("ls -R {}: {err}", rp.display_path),
                )))
            };
        }
        if b.is_empty() {
            return Ok(String::from("(empty directory tree)"));
        }
        Ok(b)
    }
}

impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List the entries of a directory. Directories are shown with a trailing slash; files show their byte size. Set recursive=true to list all nested files depth-first (skips .git/node_modules)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory path (default \".\")"},
                "recursive": {"type": "boolean", "description": "When true, recursively list all nested files (default false)"}
            }
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.execute_inner(ctx, args)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
