//! The grep tool (grep.go): native scanner answering in ripgrep's two
//! phases — tracked files first, then the paths the ignore rules exclude,
//! labeled so "absent" and "filtered" stay distinguishable. The ripgrep
//! subprocess engine (rg != "") is anchored: it needs the sandbox spec and
//! session-temp machines from unmigrated slices, and its window/execute
//! behavior is a host-level composition, not this crate's.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;

use super::confine::{confine_read, confine_scope, real_roots};
use super::gitignore::Rules;
use super::network::{ResolvedPath, resolve_readable_path};
use super::pathgo;
use super::walk::{
    WalkConfine, WalkEntry, WalkFlow, abs_clean, is_hidden_name, is_vcs_store_dir, walk_dir,
};
use crate::abi::{CallContext, Tool, ToolResult};
use crate::shell::NO_MATCHES;

pub const GREP_MAX_MATCHES: usize = 200;
pub const GREP_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const GREP_MAX_TIMEOUT: Duration = Duration::from_secs(300);
const PEEK: usize = 8 * 1024;

/// Which tree produced an answer (grep.go `grepScope`): "not in the tracked
/// files" and "not here at all" are different, and a bare "no matches" reads
/// as the second while meaning the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrepScope {
    /// The ignore rules were in force.
    Tracked,
    /// Nothing tracked matched; these came from ignored paths.
    Ignored,
    /// Nothing matched in either.
    Neither,
}

/// grep.go `formatGrep`.
pub fn format_grep(
    timed_out: bool,
    out: &[String],
    truncated: bool,
    to: Duration,
    scope: GrepScope,
) -> String {
    if out.is_empty() {
        if timed_out {
            return format!(
                "{NO_MATCHES}; timed out after {} — narrow the path/pattern or raise timeout_seconds",
                pathgo::go_duration_string(to.as_secs())
            );
        }
        if scope == GrepScope::Neither {
            return format!(
                "{NO_MATCHES}; the ignored paths were searched too, so this is absent rather than filtered"
            );
        }
        return String::from(NO_MATCHES);
    }
    let mut res = out.join("\n");
    if scope == GrepScope::Ignored {
        res = format!(
            "(no match in the tracked files; these are from paths the ignore rules exclude — build output, dependencies, generated code)\n{res}"
        );
    }
    if truncated {
        res.push_str(&format!("\n... (truncated at {GREP_MAX_MATCHES} matches)"));
    } else if timed_out {
        res.push_str(&format!(
            "\n... (timed out after {}; results incomplete — narrow the path/pattern or raise timeout_seconds)",
            pathgo::go_duration_string(to.as_secs())
        ));
    }
    res
}

/// grep.go `toolTimeout` (timeout.go): zero means default, a ceiling caps.
pub fn timeout(sec: i64, def: Duration, ceiling: Duration) -> Duration {
    let to = if sec <= 0 {
        def
    } else {
        Duration::from_secs(sec.max(0) as u64)
    };
    if !ceiling.is_zero() && to > ceiling {
        return ceiling;
    }
    to
}

/// The grep tool. `rg` is anchored (see module docs) and must stay empty on
/// this slice; `session_temp` likewise. ConfineSearch's binding of the rg
/// path is the composition root's job once the sandbox slice lands.
pub struct GrepTool {
    pub net_roots: Vec<String>,
    pub work_dir: String,
    pub rg: String,
    pub forbid_roots: Vec<String>,
    pub read_roots: Vec<String>,
}

impl GrepTool {
    fn execute_inner(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let pattern = args.get("pattern").and_then(Value::as_str).unwrap_or("");
        let mut path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let glob = args
            .get("glob")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if pattern.is_empty() {
            return Err(Box::new(std::io::Error::other("pattern is required")));
        }
        if path.is_empty() {
            path = String::from(".");
        }
        if glob.starts_with('!') {
            // A negated glob would re-admit the sensitive-file excludes
            // below it.
            return Err(Box::new(std::io::Error::other(
                "glob must select files, not exclude them",
            )));
        }
        let rp = resolve_readable_path(&self.work_dir, &path);
        if let Some(r) = rp.refuse_network(&self.net_roots) {
            return Err(Box::new(r));
        }
        let to = timeout(timeout_seconds, GREP_DEFAULT_TIMEOUT, GREP_MAX_TIMEOUT);
        if let Some(r) = confine_scope(&self.read_roots, &rp.path) {
            return Err(Box::new(r));
        }
        let info = match std::fs::symlink_metadata(&rp.path) {
            Ok(i) => i,
            Err(err) => {
                return if rp.external {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("grep {}: {}", rp.display_path, rp.error_text(&err)),
                    )))
                } else {
                    Err(Box::new(std::io::Error::new(
                        err.kind(),
                        format!("grep {}: {err}", rp.display_path),
                    )))
                };
            }
        };
        if confine_read(&self.forbid_roots, &rp.path) {
            if info.is_dir() {
                return Ok(format_grep(false, &[], false, to, GrepScope::Tracked));
            }
            let err = std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("stat {}: file does not exist", rp.path),
            );
            if rp.external {
                return Err(Box::new(std::io::Error::new(
                    err.kind(),
                    format!("grep {}: {}", rp.display_path, rp.error_text(&err)),
                )));
            }
            return Err(Box::new(err));
        }
        if !self.rg.is_empty() {
            // Anchor: the ripgrep delegation (grep.go runRipgrep) needs
            // sandbox.Spec + sessiontemp machines from unmigrated slices;
            // until they land, a configured rg falls back to the native
            // scanner rather than forging the subprocess behavior.
        }
        self.run_native(pattern, &rp.path, &glob, &info, to, &rp)
    }

    /// grep.go `runNative`: two phases. A single file is searched once —
    /// nothing was filtered.
    fn run_native(
        &self,
        pattern: &str,
        path: &str,
        glob: &str,
        info: &std::fs::Metadata,
        to: Duration,
        rp: &ResolvedPath,
    ) -> ToolResult {
        let started = Instant::now();
        let timed_out = || started.elapsed() >= to;
        let (out, truncated, err) =
            self.native_pass(pattern, path, glob, info, rp, false, &timed_out);
        if let Some(err) = err {
            return Err(err);
        }
        if !out.is_empty() || timed_out() || !info.is_dir() {
            return Ok(format_grep(
                timed_out(),
                &out,
                truncated,
                to,
                GrepScope::Tracked,
            ));
        }
        let (wide, wide_truncated, wide_err) =
            self.native_pass(pattern, path, glob, info, rp, true, &timed_out);
        if wide_err.is_some() || wide.is_empty() {
            return Ok(format_grep(false, &[], false, to, GrepScope::Neither));
        }
        Ok(format_grep(
            timed_out(),
            &wide,
            wide_truncated,
            to,
            GrepScope::Ignored,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn native_pass(
        &self,
        pattern: &str,
        path: &str,
        glob: &str,
        info: &std::fs::Metadata,
        rp: &ResolvedPath,
        wide: bool,
        timed_out: &dyn Fn() -> bool,
    ) -> (
        Vec<String>,
        bool,
        Option<Box<dyn std::error::Error + Send + Sync>>,
    ) {
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => {
                return (
                    Vec::new(),
                    false,
                    Some(Box::new(std::io::Error::other(format!(
                        "invalid pattern: {e}"
                    )))),
                );
            }
        };
        let mut out: Vec<String> = Vec::new();
        let mut truncated = false;

        if info.is_dir() {
            let root = String::from(path);
            let mut ig = WalkIgnorer::new(path, &self.forbid_roots, &self.read_roots, wide);
            let walk_result = walk_dir(path, &mut |e: &WalkEntry| {
                if e.err.is_some() {
                    return Ok(WalkFlow::Continue);
                }
                if timed_out() {
                    return Ok(WalkFlow::SkipAll);
                }
                if e.is_dir {
                    if ig.skip(&e.path, e) {
                        return Ok(WalkFlow::SkipDir);
                    }
                    ig.enter(&e.path);
                    return Ok(WalkFlow::Continue);
                }
                if ig.skip(&e.path, e) || !grep_glob_matches(glob, &root, &e.path) {
                    return Ok(WalkFlow::Continue);
                }
                if search_file(&re, &e.path, rp, &mut out, &mut truncated) {
                    return Ok(WalkFlow::SkipAll);
                }
                Ok(WalkFlow::Continue)
            });
            if let Err(err) = walk_result {
                return (out, truncated, Some(Box::new(err)));
            }
        } else if !confine_read(&self.forbid_roots, path) {
            let _ = search_file(&re, path, rp, &mut out, &mut truncated);
        }
        (out, truncated, None)
    }
}

/// grep.go `grepGlobMatches`: ripgrep's rule — a glob with no slash matches
/// the file name at any depth, one with a slash the path below the root.
pub fn grep_glob_matches(glob: &str, root: &str, file: &str) -> bool {
    if glob.is_empty() {
        return true;
    }
    if !glob.contains('/') {
        let slashed = pathgo::to_slash(file);
        let base = slashed.rsplit('/').next().unwrap_or(file);
        return super::globengine::match_slash_glob(base, glob);
    }
    let rel = match pathgo::rel(root, file) {
        Ok(r) => r,
        Err(_) => String::from(file),
    };
    super::globengine::match_slash_glob(&pathgo::to_slash(&rel), glob)
}

/// searchFile returns whether the match cap ended the walk (Go: io.EOF
/// sentinel).
fn search_file(
    re: &Regex,
    file: &str,
    rp: &ResolvedPath,
    out: &mut Vec<String>,
    truncated: &mut bool,
) -> bool {
    let mut f = match std::fs::File::open(file) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut peek = vec![0u8; PEEK];
    let n = read_full(&mut f, &mut peek);
    peek.truncate(n);

    let bom = reasonix_core::fileencoding::detect_quick(&peek);
    let bom_16 = matches!(
        bom,
        reasonix_core::fileencoding::Kind::Utf16Le
            | reasonix_core::fileencoding::Kind::Utf16Be
            | reasonix_core::fileencoding::Kind::Utf8Bom
    );
    if !bom_16 && peek.contains(&0) {
        return false;
    }

    let text: Vec<u8> = if matches!(
        bom,
        reasonix_core::fileencoding::Kind::Utf16Le | reasonix_core::fileencoding::Kind::Utf16Be
    ) {
        let mut rest = Vec::new();
        if f.read_to_end(&mut rest).is_err() {
            return false;
        }
        let mut all = peek.clone();
        all.extend_from_slice(&rest);
        reasonix_core::fileencoding::decode(&all, bom).into_owned()
    } else {
        let kind = if n == PEEK {
            reasonix_core::fileencoding::detect_fragment(&peek).0
        } else {
            reasonix_core::fileencoding::detect_and_decode(&peek).0
        };
        match kind {
            reasonix_core::fileencoding::Kind::Gb18030
            | reasonix_core::fileencoding::Kind::Gbk
            | reasonix_core::fileencoding::Kind::Utf16LeNoBom
            | reasonix_core::fileencoding::Kind::Utf16BeNoBom => {
                let mut rest = Vec::new();
                if f.read_to_end(&mut rest).is_err() {
                    return false;
                }
                let mut all = peek.clone();
                all.extend_from_slice(&rest);
                reasonix_core::fileencoding::decode(&all, kind).into_owned()
            }
            _ => peek,
        }
    };

    for (ln, line) in LineIter::new(&text).enumerate() {
        let ln = ln + 1;
        if line.contains('\0') {
            return false;
        }
        if re.is_match(&line) {
            out.push(format!("{}:{ln}:{}", rp.display_for(file), line));
            if out.len() >= GREP_MAX_MATCHES {
                *truncated = true;
                return true;
            }
        }
    }
    false
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> usize {
    let mut n = 0usize;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    n
}

use std::io::Read as _;

/// Splits decoded text on \n, dropping a trailing empty segment the way a
/// scanner over text does, and strips a trailing \r per line.
struct LineIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> LineIter<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

impl Iterator for LineIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if self.pos >= self.data.len() {
            return None;
        }
        let rest = &self.data[self.pos..];
        let i = rest.iter().position(|&b| b == b'\n');
        let (line, next_pos) = match i {
            Some(i) => (&rest[..i], self.pos + i + 1),
            None => (rest, self.data.len()),
        };
        self.pos = next_pos;
        let line = if line.last() == Some(&b'\r') {
            &line[..line.len() - 1]
        } else {
            line
        };
        Some(String::from_utf8_lossy(line).into_owned())
    }
}

/// builtin/gitignore.go `walkIgnorer`: prunes a recursive grep walk to
/// mirror ripgrep — hidden entries, the fixed vendor dirs, and anything the
/// repository's ignore rules match. The walk root is never pruned, and
/// pointing grep straight at a hidden or ignored path searches it in full.
/// `wide` drops the ignore rules and the build-output directories, and only
/// those: confinement and the VCS store stay pruned above it.
pub struct WalkIgnorer {
    root: String,
    disabled: bool,
    wide: bool,
    frames: Vec<(String, Arc<Rules>)>,
    confine: WalkConfine,
}

use std::sync::Arc;

impl WalkIgnorer {
    pub fn new(root: &str, forbid_roots: &[String], scope: &[String], wide: bool) -> Self {
        let root_abs = abs_clean(root);
        let rules = Rules::at(&root_abs);
        let repo_root = String::from(rules.repo_root());
        let mut frames: Vec<(String, Arc<Rules>)> = Vec::new();
        let mut disabled = false;
        if repo_root.is_empty() {
            // Go's gitignore.At yields no rules outside a repository; the
            // observed Go oracle behavior (grep_ignored_test) still excludes
            // the walk root's own .gitignore there, so the walk root acts as
            // its own anchor. Recorded as an open question in the report.
            let root_rules = Rules::at_rootless(&root_abs);
            if let Some(r) = root_rules {
                frames.push((root_abs.clone(), r));
                let base = pathgo::clean(&root_abs)
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let top = &frames[0].1;
                if is_hidden_name(&base) || top.ignored(&root_abs, true) {
                    disabled = true;
                }
            }
        } else {
            frames.push((root_abs.clone(), rules));
            let base = pathgo::clean(&root_abs)
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_owned();
            let top = &frames[0].1;
            if is_hidden_name(&base) || top.ignored(&root_abs, true) {
                disabled = true;
            }
        }
        Self {
            root: root_abs,
            disabled,
            wide,
            frames,
            confine: WalkConfine::new(forbid_roots, scope, root),
        }
    }

    /// Loads a kept directory's own .gitignore as a cumulative frame
    /// governing its children; called after skip clears the directory.
    pub fn enter(&mut self, path: &str) {
        if self.disabled || self.frames.is_empty() {
            return;
        }
        let abs = abs_clean(path);
        if abs == self.root {
            return;
        }
        let top = self.frames.last().unwrap().1.clone();
        let next = top.descend(&abs);
        self.frames.push((abs, next));
    }

    /// Whether a walked entry should be pruned, popping frames the walk has
    /// moved past.
    pub fn skip(&mut self, path: &str, d: &WalkEntry) -> bool {
        let name = d.name.as_str();
        let is_dir = d.is_dir;
        let abs = abs_clean(path);
        while self.frames.len() > 1 && !super::walk::under_dir(&self.frames.last().unwrap().0, &abs)
        {
            self.frames.pop();
        }
        if abs == self.root || self.disabled {
            return false;
        }
        // Confinement first, and above the wide switch: what a caller may
        // not read is not an ignore rule, and widening a search must not
        // widen that.
        if is_dir
            && (super::walk::is_vendor_dir(name)
                || self
                    .confine
                    .blocked(path, Some((&d.name, d.is_dir, d.is_regular))))
        {
            return true;
        }
        if is_dir && is_vcs_store_dir(name) {
            return true;
        }
        if self.wide {
            return false;
        }
        if is_hidden_name(name) {
            return true;
        }
        if is_dir && super::walk::is_vendor_dir(name) {
            return true;
        }
        self.ignored(&abs, is_dir)
    }

    fn ignored(&self, abs: &str, is_dir: bool) -> bool {
        match self.frames.last() {
            Some((_, rules)) => rules.ignored(abs, is_dir),
            None => false,
        }
    }
}

/// grep.go `ResolveSearch`: picks the engine from config. "native" forces
/// the native scanner; "rg" requires ripgrep (warns and falls back when
/// absent); "auto"/"" uses ripgrep when found. `rg_path` overrides PATH.
pub fn resolve_search(
    engine: &str,
    rg_path: &str,
    warn: &mut dyn std::io::Write,
) -> Option<String> {
    let find = || -> Option<String> {
        if !rg_path.is_empty() {
            if let Ok(fi) = std::fs::metadata(rg_path)
                && !fi.is_dir()
            {
                return Some(String::from(rg_path));
            }
            return None;
        }
        find_rg_on_path()
    };
    match engine.trim().to_lowercase().as_str() {
        "native" => None,
        "rg" => match find() {
            Some(p) => Some(p),
            None => {
                let _ = writeln!(
                    warn,
                    "warning: [tools.search] engine=\"rg\" but ripgrep (rg) was not found; using the native search engine"
                );
                None
            }
        },
        _ => find(),
    }
}

/// Go `exec.LookPath("rg")` over PATH.
fn find_rg_on_path() -> Option<String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    for dir in path_var.split(if cfg!(windows) { ';' } else { ':' }) {
        if dir.is_empty() {
            continue;
        }
        let candidate = format!("{}{}rg{}", dir, pathgo::separator(), ext);
        if let Ok(fi) = std::fs::metadata(&candidate)
            && !fi.is_dir()
        {
            return Some(candidate);
        }
    }
    None
}

/// grep.go `ConfineSearch`: binds a resolved search engine and forbid-read
/// roots. The sandbox spec parameter of the Go form is anchored — this
/// slice's tools never launch a subprocess.
pub fn confine_search(_rg_path: Option<String>, forbid_roots: &[String]) -> GrepTool {
    GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: real_roots(forbid_roots),
        read_roots: Vec::new(),
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        const RG: &str = "Search for a regular expression in a file, or recursively under a directory — ripgrep-backed, so it honors .gitignore. Returns matching lines as path:line:text, capped at 200 matches. Ignored paths are searched only when nothing else matched; such matches say so, so an empty answer means absent rather than filtered.";
        const NATIVE: &str = "Search for a regular expression in a file, or recursively under a directory (skips hidden files and files matched by .gitignore). Returns matching lines as path:line:text, capped at 200 matches. Ignored paths are searched only when nothing else matched; such matches say so, so an empty answer means absent rather than filtered.";
        if !self.rg.is_empty() {
            return RG;
        }
        NATIVE
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regular expression (RE2 syntax)"},
                "path": {"type": "string", "description": "File or directory to search (default \".\")"},
                "glob": {"type": "string", "description": "Only search files matching this glob. A glob with no slash matches the file name at any depth (\"*.go\"); one with a slash matches the path below the search root (\"internal/**/*_test.go\")."},
                "timeout_seconds": {"type": "integer", "description": "Abort and return partial matches after this many seconds (default 30, max 300). Raise it for a large tree; lower it for a quick probe.", "minimum": 1}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_clamp() {
        let cases = [
            (0i64, GREP_DEFAULT_TIMEOUT),
            (-5, GREP_DEFAULT_TIMEOUT),
            (5, Duration::from_secs(5)),
            (99999, GREP_MAX_TIMEOUT),
        ];
        for (sec, want) in cases {
            assert_eq!(
                timeout(sec, GREP_DEFAULT_TIMEOUT, GREP_MAX_TIMEOUT),
                want,
                "{sec}"
            );
        }
    }

    #[test]
    fn format_preserves_partial_results() {
        let hit = vec![String::from("a.go:1:match")];
        let got = format_grep(
            true,
            &hit,
            false,
            Duration::from_secs(30),
            GrepScope::Tracked,
        );
        assert!(got.starts_with("a.go:1:match"), "{got}");
        assert!(
            got.contains("timed out") && got.contains("timeout_seconds"),
            "{got}"
        );
        let got = format_grep(
            true,
            &[],
            false,
            Duration::from_secs(30),
            GrepScope::Tracked,
        );
        assert!(got.contains("timed out"), "{got}");
        let got = format_grep(
            false,
            &[],
            false,
            Duration::from_secs(30),
            GrepScope::Tracked,
        );
        assert_eq!(got, NO_MATCHES);
    }

    #[test]
    fn resolve_search_picks_engine() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("rgbin")
            .tempdir_in(&base)
            .unwrap();
        let rg_file = dir.path().join("rg");
        std::fs::write(&rg_file, b"x").unwrap();
        let missing = dir.path().join("absent");
        let rg = rg_file.to_string_lossy().into_owned();
        let miss = missing.to_string_lossy().into_owned();
        let mut warn = Vec::new();
        assert_eq!(resolve_search("native", &rg, &mut warn), None);
        assert_eq!(resolve_search("rg", &rg, &mut warn), Some(rg.clone()));
        assert_eq!(resolve_search("auto", &rg, &mut warn), Some(rg.clone()));
        warn.clear();
        assert_eq!(resolve_search("rg", &miss, &mut warn), None);
        assert!(String::from_utf8_lossy(&warn).contains("ripgrep"));
    }

    #[test]
    fn glob_no_slash_matches_file_name_any_depth() {
        assert!(grep_glob_matches("*.go", "/r", "/r/deep/main.go"));
        assert!(!grep_glob_matches("*.go", "/r", "/r/main.md"));
        assert!(grep_glob_matches(
            "internal/**/*_test.go",
            "/r",
            "/r/internal/deep/x_test.go"
        ));
        assert!(!grep_glob_matches(
            "internal/**/*_test.go",
            "/r",
            "/r/top.go"
        ));
    }
}
