//! The glob tool (glob.go): filepath.Glob fast path, doublestar recursive
//! walk, simple-filename fallback, forbid-read filtering, result cap.

#![forbid(unsafe_code)]

use std::time::Duration;

use serde_json::Value;

use super::confine::{confine_read, confine_scope, read_outside_scope};
use super::globengine::{match_doublestar, match_filepath, split_pattern};
use super::grep::timeout;
use super::network::{ResolvedPath, resolve_readable_path};
use super::pathgo;
use super::walk::{WalkConfine, WalkEntry, WalkFlow, walk_dir};
use crate::abi::{CallContext, Tool, ToolResult};
use crate::shell::NO_MATCHES;

pub const GLOB_MAX_RESULTS: usize = 1000;
pub const GLOB_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const GLOB_MAX_TIMEOUT: Duration = Duration::from_secs(300);

/// The glob tool. Scopes mirror Go's globTool fields; `paths` (token
/// aliases, §7.7) is anchored, not migrated.
pub struct GlobTool {
    pub net_roots: Vec<String>,
    pub work_dir: String,
    pub forbid_roots: Vec<String>,
    pub read_roots: Vec<String>,
}

impl GlobTool {
    fn execute_inner(
        &self,
        ctx: &CallContext,
        args: &Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let pattern_arg = args.get("pattern").and_then(Value::as_str).unwrap_or("");
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if pattern_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("pattern is required")));
        }
        let to = timeout(timeout_seconds, GLOB_DEFAULT_TIMEOUT, GLOB_MAX_TIMEOUT);
        let started = std::time::Instant::now();
        // The raw pattern is kept for the simple-filename fallback: an
        // already-joined absolute path always contains separators.
        let raw_pattern = String::from(pattern_arg);
        let rp = resolve_readable_path(&self.work_dir, pattern_arg);
        let pattern_resolved = rp.path.clone();
        if let Some(r) = rp.refuse_network(&self.net_roots) {
            return Err(Box::new(r));
        }
        if let Some(r) = confine_scope(&self.read_roots, &glob_base(&pattern_resolved)) {
            return Err(Box::new(r));
        }
        let pattern_native = pathgo::from_slash(&pattern_resolved);
        let display_pattern = rp.display_path.clone();

        if pattern_native.contains("**") {
            return self.glob_recursive(ctx, &pattern_native, &display_pattern, &rp, to, started);
        }

        match filepath_glob(&pattern_native) {
            Ok(mut matches) => {
                matches = filter_forbid_matches(matches, &self.forbid_roots, &self.read_roots);
                if matches.is_empty() && !raw_pattern.contains(['/', '\\']) {
                    let fallback = pathgo::join(&[&self.work_dir, "**", &raw_pattern]);
                    let empty = ResolvedPath::default();
                    return self.glob_recursive(
                        ctx,
                        &pathgo::from_slash(&fallback),
                        &pathgo::from_slash(&fallback),
                        &empty,
                        to,
                        started,
                    );
                }
                if matches.is_empty() {
                    return Ok(String::from(NO_MATCHES));
                }
                let matches = display_glob_matches(&matches, &rp);
                if matches.len() > GLOB_MAX_RESULTS {
                    let mut out = matches[..GLOB_MAX_RESULTS].join("\n");
                    out.push_str(&format!("\n... (truncated at {GLOB_MAX_RESULTS} results)"));
                    return Ok(out);
                }
                Ok(matches.join("\n"))
            }
            Err(err) => {
                if rp.external {
                    Err(Box::new(std::io::Error::other(format!(
                        "glob {display_pattern:?}: {}",
                        rp.error_text(&err)
                    )))
                        as Box<dyn std::error::Error + Send + Sync>)
                } else {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("glob {display_pattern:?}: {err}"),
                    ))
                        as Box<dyn std::error::Error + Send + Sync>)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn glob_recursive(
        &self,
        _ctx: &CallContext,
        pattern: &str,
        display_pattern: &str,
        rp: &ResolvedPath,
        to: Duration,
        started: std::time::Instant,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (root, rel_pattern) = glob_walk_root(pattern);
        let expired = || started.elapsed() >= to;

        let info = match std::fs::metadata(&root) {
            Ok(i) => i,
            Err(err) => {
                return if rp.external {
                    Err(Box::new(std::io::Error::other(format!(
                        "glob {display_pattern:?}: {}",
                        rp.error_text(&err)
                    ))))
                } else {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("glob {display_pattern:?}: {err}"),
                    )))
                };
            }
        };
        if !info.is_dir() {
            return Ok(String::from(NO_MATCHES));
        }
        if let Some(r) = confine_scope(&self.read_roots, &root) {
            return Err(Box::new(r));
        }
        let mut matches: Vec<String> = Vec::new();
        let mut truncated = false;
        let confine_walker = WalkConfine::new(&self.forbid_roots, &self.read_roots, &root);
        let mut timed_out = false;
        let walk_err = walk_dir(&root, &mut |e: &WalkEntry| {
            if e.err.is_some() {
                return Ok(WalkFlow::Continue);
            }
            if expired() {
                timed_out = true;
                return Ok(WalkFlow::SkipAll);
            }
            if e.is_dir {
                if skip_walk_dir(&root, e)
                    || confine_walker.blocked(&e.path, Some((&e.name, e.is_dir, e.is_regular)))
                {
                    return Ok(WalkFlow::SkipDir);
                }
                return Ok(WalkFlow::Continue);
            }
            if confine_walker.blocked(&e.path, Some((&e.name, e.is_dir, e.is_regular))) {
                return Ok(WalkFlow::Continue);
            }
            let rel = match pathgo::rel(&root, &e.path) {
                Ok(r) => r,
                Err(_) => return Ok(WalkFlow::Continue),
            };
            let matched = match_doublestar(
                &pathgo::to_slash(&pathgo::from_slash(&rel_pattern)),
                &pathgo::to_slash(&rel),
            );
            if matched {
                matches.push(e.path.clone());
            }
            if matches.len() >= GLOB_MAX_RESULTS {
                truncated = true;
                return Ok(WalkFlow::SkipAll);
            }
            Ok(WalkFlow::Continue)
        });
        if let Err(err) = walk_err {
            return if rp.external {
                Err(Box::new(std::io::Error::other(format!(
                    "glob {display_pattern:?}: {}",
                    rp.error_text(&err)
                ))))
            } else {
                Err(Box::new(std::io::Error::new(
                    err.kind(),
                    format!("glob {display_pattern:?}: {err}"),
                )))
            };
        }

        if matches.is_empty() {
            if timed_out {
                return Ok(format!(
                    "{NO_MATCHES}; timed out after {} — narrow the pattern or raise timeout_seconds",
                    pathgo::go_duration_string(to.as_secs())
                ));
            }
            return Ok(String::from(NO_MATCHES));
        }
        matches.sort();
        let matches = display_glob_matches(&matches, rp);
        let mut result = matches.join("\n");
        if truncated {
            result.push_str(&format!("\n... (truncated at {GLOB_MAX_RESULTS} results)"));
        } else if timed_out {
            result.push_str(&format!(
                "\n... (timed out after {}; results incomplete — narrow the pattern or raise timeout_seconds)",
                pathgo::go_duration_string(to.as_secs())
            ));
        }
        Ok(result)
    }
}

fn skip_walk_dir(root: &str, e: &WalkEntry) -> bool {
    super::walk::skip_walk_dir(root, &e.path, &e.name)
}

/// workspace.go `filterForbidMatches`.
fn filter_forbid_matches(
    matches: Vec<String>,
    forbid_roots: &[String],
    scope: &[String],
) -> Vec<String> {
    if matches.is_empty()
        || (forbid_roots.is_empty()
            && scope.is_empty()
            && !super::secretpaths::protect_sensitive_files()
            && !super::secretpaths::PROTECT_CREDENTIAL_FILES)
    {
        return matches;
    }
    matches
        .into_iter()
        .filter(|m| !read_outside_scope(scope, m) && !confine_read(forbid_roots, m))
        .collect()
}

/// glob.go `globWalkRoot`: the tree to walk and the pattern to match
/// relative paths against, repairing the one root doublestar hands back that
/// does not name what it looks like — a pattern rooted at a drive splits to
/// the bare volume, and "D:" is that drive's current directory, not its root.
pub fn glob_walk_root(pattern: &str) -> (String, String) {
    let (root_slash, rel) = split_pattern(&pathgo::to_slash(&pathgo::clean(pattern)));
    let mut root = pathgo::from_slash(&root_slash);
    let vol = pathgo::volume_name(&root);
    if !vol.is_empty() && vol == root {
        root.push(pathgo::separator());
    }
    let rel = if rel.is_empty() {
        String::from("**")
    } else {
        rel
    };
    (root, rel)
}

fn display_glob_matches(matches: &[String], rp: &ResolvedPath) -> Vec<String> {
    if !rp.external {
        return matches.to_vec();
    }
    matches.iter().map(|m| rp.display_for(m)).collect()
}

/// glob.go `globBase`: the literal directory a pattern is anchored at —
/// everything before its first metacharacter, cut back to a path separator.
pub fn glob_base(pattern: &str) -> String {
    let i = match pattern.find(['*', '?', '[', '{']) {
        Some(i) => i,
        None => return String::from(pattern),
    };
    let base = &pattern[..i];
    match base.rfind(['/', '\\']) {
        Some(j) => String::from(&base[..j + 1]),
        None => String::from("."),
    }
}

/// Go `filepath.Glob`: matches against directory listings, lexicographical,
/// OS-separator aware, ignoring unreadable directories, with the Lstat
/// existence check for meta-free patterns and the recursive descent when the
/// directory part itself carries metacharacters.
pub fn filepath_glob(pattern: &str) -> std::io::Result<Vec<String>> {
    glob_with_limit(pattern, 0)
}

fn glob_with_limit(pattern: &str, depth: usize) -> std::io::Result<Vec<String>> {
    const PATH_SEPARATORS_LIMIT: usize = 10_000;
    if depth == PATH_SEPARATORS_LIMIT {
        return Err(std::io::Error::other("syntax error in pattern"));
    }
    if match_filepath(pattern, "").is_err() {
        return Err(std::io::Error::other("syntax error in pattern"));
    }
    if !pathgo::has_meta(pattern) {
        if std::fs::symlink_metadata(pattern).is_err() {
            return Ok(Vec::new());
        }
        return Ok(vec![String::from(pattern)]);
    }
    let (dir_raw, file) = pathgo::split(pattern);
    let (volume_len, dir) = clean_glob_path_windows(&dir_raw);
    let dir = if cfg!(windows) {
        dir
    } else {
        clean_glob_path(&dir_raw)
    };
    let dir = if cfg!(windows) {
        dir
    } else {
        String::from(&dir)
    };
    if !pathgo::has_meta(if cfg!(windows) {
        &dir[volume_len.min(dir.len())..]
    } else {
        &dir
    }) {
        return glob_dir_files(&dir, &file, Vec::new());
    }
    if dir == pattern {
        return Err(std::io::Error::other("syntax error in pattern"));
    }
    let dirs = glob_with_limit(&dir, depth + 1)?;
    let mut out = Vec::new();
    for d in dirs {
        out = glob_dir_files(&d, &file, out)?;
    }
    Ok(out)
}

fn clean_glob_path(path: &str) -> String {
    match path {
        "" => String::from("."),
        p if p.len() == 1 && pathgo::is_path_separator(p.as_bytes()[0]) => String::from(p),
        p => String::from(&p[..p.len() - 1]),
    }
}

fn clean_glob_path_windows(path: &str) -> (usize, String) {
    let vollen = pathgo::volume_name_len(path);
    if path.is_empty() {
        return (0, String::from("."));
    }
    if vollen + 1 == path.len() && pathgo::is_path_separator(path.as_bytes()[path.len() - 1]) {
        return (vollen + 1, String::from(path));
    }
    if vollen == path.len() && path.len() == 2 {
        return (vollen, format!("{path}."));
    }
    let vollen = if vollen >= path.len() {
        path.len() - 1
    } else {
        vollen
    };
    (vollen, String::from(&path[..path.len() - 1]))
}

fn glob_dir_files(
    dir: &str,
    pattern: &str,
    mut matches: Vec<String>,
) -> std::io::Result<Vec<String>> {
    let info = match std::fs::metadata(dir) {
        Ok(i) => i,
        Err(_) => return Ok(matches),
    };
    if !info.is_dir() {
        return Ok(matches);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(matches),
    };
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => names.push(e.file_name().to_string_lossy().into_owned()),
            Err(_) => return Ok(matches),
        }
    }
    names.sort();
    for n in names {
        match match_filepath(pattern, &n) {
            Ok(true) => matches.push(pathgo::join(&[dir, &n])),
            Ok(false) => {}
            Err(_) => return Err(std::io::Error::other("syntax error in pattern")),
        }
    }
    Ok(matches)
}

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. \"*.go\", \"internal/*/*.go\", \"**/*.test.ts\"). Supports shell metacharacters * ? [] and the recursive ** pattern."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern (supports ** for recursive matching)"},
                "timeout_seconds": {"type": "integer", "description": "Walk timeout in seconds (default 30, max 300); partial results are returned when it expires"}
            },
            "required": ["pattern"]
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

/// The refused-outside-scope construction a glob target carries; re-exported
/// so tests can name the type.
pub use super::confine::CODE_READ_OUTSIDE_SCOPE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_root_repairs_bare_drive() {
        let (root, rel) = glob_walk_root(if cfg!(windows) {
            "D:\\**\\*xray*"
        } else {
            "/**/*xray*"
        });
        if cfg!(windows) {
            assert_eq!(root, "D:\\");
            assert_eq!(rel, "**/*xray*");
            assert!(pathgo::is_abs(&root));
        }
        let (root2, rel2) = glob_walk_root(if cfg!(windows) { "D:\\" } else { "/" });
        assert_eq!(rel2, "**");
        assert!(!root2.is_empty());
    }

    #[test]
    fn glob_base_anchors_before_meta() {
        assert_eq!(glob_base("src/*.go"), "src/");
        assert_eq!(glob_base("**/x"), ".");
        assert_eq!(glob_base("plain/x.go"), "plain/x.go");
    }

    #[test]
    fn filepath_glob_lists_lexically() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("globfp")
            .tempdir_in(&base)
            .unwrap();
        std::fs::write(dir.path().join("a.go"), b"").unwrap();
        std::fs::write(dir.path().join("b.go"), b"").unwrap();
        let pat = pathgo::join(&[dir.path().to_str().unwrap(), "*.go"]);
        let out = filepath_glob(&pat).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn unresolvable_read_roots_refuse_everything() {
        let tool = GlobTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: Vec::new(),
            read_roots: vec![pathgo::join(&[
                &pathgo::separator().to_string(),
                "\x00unresolved",
            ])],
        };
        let args = serde_json::json!({"pattern": "*.rs"});
        let err = Tool::execute(&tool, &CallContext::new(), &args).unwrap_err();
        assert!(
            err.to_string()
                .contains("outside the folders this run may read"),
            "{err}"
        );
    }
}
