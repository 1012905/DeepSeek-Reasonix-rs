//! Workspace confinement (confine.go §7.1): path resolution, forbid-read
//! deny checks, read-scope refusals, the write-side boundary check
//! (`confine_write`), and the shared path primitives (`real_path`,
//! `within`, `within_fold`, `real_roots`). The session-data guard,
//! managed-config approval and session-temp manager are other slices'
//! machines; the writers reach them through the hook traits below.

#![forbid(unsafe_code)]

use std::path::Path;

use std::sync::Arc;

use crate::builtin::anchors::{CODE_WRITE_OUTSIDE_SCOPE, ConfineError};
use crate::builtin::pathgo;
use crate::builtin::secretpaths;
use crate::builtin::secretpaths::{
    PROTECT_CREDENTIAL_FILES, credential_read_path, fold_paths, sensitive_file_name,
    sensitive_home_dirs,
};
use crate::refusal::Refusal;

/// `workspace.read_outside_scope` — a read refused for resolving outside
/// every read root, symlinks resolved (confine.go).
pub const CODE_READ_OUTSIDE_SCOPE: &str = "workspace.read_outside_scope";

/// The session-data guard hook (confine.go `SessionDataGuard.Check`): one
/// policy the composition root binds, refusing writes into Reasonix's own
/// session stores. `None` is Go's zero value — unconfined.
pub trait SessionDataGuardHook: Send + Sync {
    fn check(&self, target: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// The managed-config hook (confine.go `ManagedConfigPaths`): names the
/// Reasonix-owned config files writable outside the roots after a fresh
/// per-write human approval. `None` is Go's zero value — no escape hatch.
pub trait ManagedConfigHook: Send + Sync {
    fn matches(&self, target: &str) -> bool;
    fn approve(&self, target: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// The session-temp hook (confine.go `sessiontemp.Manager`, the slice of it
/// the writers need): the directory a run's scratch files live under.
pub trait SessionTempSource: Send + Sync {
    /// The session temp dir if the generation already exists.
    fn dir(&self) -> Option<String>;
    /// Creates the generation when needed (Go `Acquire`/`Release`).
    fn ensure_dir(&self) -> Option<String> {
        self.dir()
    }
}

/// An owned hook set the writers share (Go: the struct fields).
#[derive(Clone, Default)]
pub struct WriteHooks {
    pub guard: Option<Arc<dyn SessionDataGuardHook>>,
    pub managed: Option<Arc<dyn ManagedConfigHook>>,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
}

/// confine.go `confineRead`: target inside a forbidRoot, a credential file,
/// or — with the protect toggle on — a sensitive path. Empty deny surface is
/// unconfined. An unresolvable target passes: the caller's normal error path
/// handles it, mimicking the tmpfs semantics the bubblewrap sandbox provides.
pub fn confine_read(forbid_roots: &[String], target: &str) -> bool {
    let protect = secretpaths::protect_sensitive_files();
    let credentials = PROTECT_CREDENTIAL_FILES;
    if forbid_roots.is_empty() && !protect && !credentials {
        return false;
    }
    let abs = match real_path(target) {
        Ok(a) => a,
        Err(_) => return false,
    };
    confine_resolved(forbid_roots, &abs, protect)
}

/// `confineRead` for a path already free of symlinks (confine.go
/// `confineResolved`).
pub fn confine_resolved(forbid_roots: &[String], abs: &str, protect: bool) -> bool {
    if credential_read_path(abs) {
        return true;
    }
    if protect && sensitive_read_path(abs) {
        return true;
    }
    forbid_roots.iter().any(|r| within_fold(r, abs))
}

fn sensitive_read_path(abs: &str) -> bool {
    let clean = pathgo::clean(abs);
    if sensitive_file_name(&secretpaths::file_base(&clean)) {
        return true;
    }
    sensitive_home_dirs()
        .iter()
        .any(|dir| within_fold(dir, &clean))
}

/// confine.go `readOutsideScope`: resolves outside every scope root. Empty
/// scope unconfined; an unresolvable path counts as outside.
pub fn read_outside_scope(scope: &[String], target: &str) -> bool {
    if scope.is_empty() {
        return false;
    }
    let abs = match real_path(target) {
        Ok(a) => a,
        Err(_) => return true,
    };
    !scope.iter().any(|r| within(r, &abs))
}

/// confine.go `confineScope`: `read_outside_scope` with the typed refusal a
/// read tool returns.
pub fn confine_scope(scope: &[String], target: &str) -> Option<Refusal> {
    if !read_outside_scope(scope, target) {
        return None;
    }
    Some(Refusal::code(
        CODE_READ_OUTSIDE_SCOPE,
        format!(
            "read refused: `{target}` is outside the folders this run may read ({}), after following symlinks. \
             This run is read-only and limited to its workspace; nothing else can be read from it",
            scope.join(", ")
        ),
    ))
}

/// confine.go `realPath`: resolves as the system would when opening — links
/// (and Windows junctions, via canonicalize) followed, a not-yet-existing
/// tail re-appended. A POSIX path is not cleaned first: a `..` after a link
/// names the target's parent.
pub fn real_path(path: &str) -> std::io::Result<String> {
    let abs = if cfg!(windows) || !pathgo::is_abs(path) {
        pathgo::abs_path(path)?
    } else {
        String::from(path)
    };
    let mut tail: Vec<String> = Vec::new();
    let mut cur = abs.clone();
    loop {
        match lookup_existing(&cur) {
            Ok(real) => {
                let mut parts = vec![real];
                parts.extend(tail.iter().cloned());
                return Ok(pathgo::join(
                    &parts.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ));
            }
            Err(_) => {
                let (parent, last) = split_last_component(&cur);
                if parent.is_empty() || parent == cur {
                    return Ok(pathgo::clean(&abs));
                }
                tail.insert(0, last);
                cur = parent;
            }
        }
    }
}

/// The filesystem lookup `real_path` makes (Go `resolveExisting`):
/// `filepath.EvalSymlinks` on POSIX; canonicalize follows junctions and 8.3
/// short names on Windows like Go's handle-based resolver.
fn lookup_existing(cur: &str) -> std::io::Result<String> {
    std::fs::canonicalize(cur).map(|p| {
        let s = p.to_string_lossy().into_owned();
        // canonicalize returns the verbatim \?\ form on Windows; Go's
        // EvalSymlinks-based resolver does not, and the tools print paths.
        s.trim_start_matches(r"\\?\").to_owned()
    })
}

/// confine.go `splitLastComponent`: drops the final element without cleaning
/// what is left, drive letters and roots kept whole.
fn split_last_component(p: &str) -> (String, String) {
    let trimmed = if cfg!(windows) {
        p.trim_end_matches(['/', '\\'])
    } else {
        p.trim_end_matches('/')
    };
    let b = trimmed.as_bytes();
    let mut i: isize = -1;
    for (idx, ch) in b.iter().enumerate() {
        if pathgo::is_path_separator(*ch) {
            i = idx as isize;
        }
    }
    if i < 0 {
        return (String::new(), String::from(trimmed));
    }
    let i = i as usize;
    if i == 0 {
        return (String::from(&trimmed[..1]), String::from(&trimmed[1..]));
    }
    if cfg!(windows) && i == 2 && b[1] == b':' {
        return (String::from(&trimmed[..3]), String::from(&trimmed[3..]));
    }
    (String::from(&trimmed[..i]), String::from(&trimmed[i + 1..]))
}

/// confine.go `within`: path at or below root; both sides absolute, cleaned,
/// symlink-free. `filepath.Rel` so a partial-component prefix ("/work-other"
/// vs "/work") cannot read as containment.
pub fn within(root: &str, path: &str) -> bool {
    match pathgo::rel(root, path) {
        Ok(r) => r == "." || (r != ".." && !r.starts_with(&format!("..{}", pathgo::separator()))),
        Err(_) => false,
    }
}

/// `within` with platform case folding, DENY-side only (confine.go
/// `withinFold`): folding a deny rule only ever refuses more; folding an
/// allow rule would wave a genuinely different directory through.
pub fn within_fold(root: &str, path: &str) -> bool {
    if fold_paths() {
        within(&root.to_lowercase(), &path.to_lowercase())
    } else {
        within(root, path)
    }
}

/// confine.go `realRoots`: each root resolved once to an absolute,
/// symlink-free path; unresolvable roots dropped.
pub fn real_roots(roots: &[String]) -> Vec<String> {
    roots.iter().filter_map(|r| real_path(r).ok()).collect()
}

/// confine.go `confine`: an error when target resolves outside every root.
/// Empty roots are unconfined. The path is printed verbatim, never
/// quoted-escaped: %q doubles every Windows backslash.
pub fn confine(roots: &[String], target: &str) -> Result<(), ConfineError> {
    let root_refs: Vec<&str> = roots.iter().map(|s| s.as_str()).collect();
    if let Some(err) = crate::builtin::network::refuse_network_path(target, &root_refs) {
        return Err(ConfineError::Network(err));
    }
    if roots.is_empty() {
        return Ok(());
    }
    let abs = real_path(target).map_err(|e| ConfineError::Resolve {
        target: target.to_owned(),
        message: e.to_string(),
    })?;
    if roots.iter().any(|r| within(r, &abs)) {
        return Ok(());
    }
    Err(ConfineError::Outside(Refusal::code(
        CODE_WRITE_OUTSIDE_SCOPE,
        format!(
            "write refused: `{target}` is outside the workspace write scope; file tools may write only under {}. \
             This scope is enforced by Reasonix's file tools on every platform: it is not an OS sandbox, and it does not depend on whether shell commands are sandboxed. \
             Write inside those directories, or ask the user to add the target's directory to the extra writable directories \
             (Settings > Sandbox > Also writable, stored as allow_write under [sandbox] in reasonix.toml) or to move workspace_root",
            roots.join(", ")
        ),
    )))
}

/// confine.go `confineOpened` re-checks the actually-opened file so a link
/// swapped in between cannot carry the read outside. Where the platform
/// cannot report an open file's path the earlier check stands alone; the
/// Rust read tools re-resolve the named path, which is that weaker check.
pub fn confine_opened(scope: &[String], path: &str) -> Option<Refusal> {
    if scope.is_empty() {
        return None;
    }
    let real = real_path(path).unwrap_or_else(|_| path.to_owned());
    confine_scope(scope, &real)
}

/// confine.go `confineWrite`, the write-tool boundary check. Order is
/// frozen: workspace confinement first; inside the roots or under the
/// session temp → the session-data guard; a network refusal passes
/// through; a non-managed target gets the confinement error plus the
/// session-temp alternative; a managed config file needs the guard and
/// then a fresh per-write human approval. A write can be inside the roots
/// (a home-directory workspace covering the state root) and still be
/// refused when it targets Reasonix's own session stores.
pub fn confine_write(
    roots: &[String],
    hooks: &WriteHooks,
    target: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let confine_err = confine(roots, target);
    let confine_err = match confine_err {
        Ok(()) => return guard_check(hooks, target),
        Err(err) => err,
    };
    if under_session_temp(hooks.session_temp.as_deref(), target) {
        return guard_check(hooks, target);
    }
    if confine_err.is_network() {
        return Err(Box::new(confine_err));
    }
    let managed_matches = hooks.managed.as_ref().is_some_and(|m| m.matches(target));
    if !managed_matches {
        return Err(with_session_temp_alternative(
            hooks.session_temp.as_deref(),
            confine_err,
        ));
    }
    guard_check(hooks, target)?;
    match hooks
        .managed
        .as_ref()
        .expect("checked above")
        .approve(target)
    {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The session-data guard check with Go's zero-value semantics: no hook is
/// unconfined.
fn guard_check(
    hooks: &WriteHooks,
    target: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match hooks.guard.as_ref() {
        None => Ok(()),
        Some(guard) => guard.check(target),
    }
}

/// confine.go `withSessionTempAlternative` names the one writable place
/// outside the roots, so a refused scratch write has somewhere to go that
/// is not the repository.
fn with_session_temp_alternative(
    temp: Option<&dyn SessionTempSource>,
    err: ConfineError,
) -> Box<dyn std::error::Error + Send + Sync> {
    let Some(dir) = temp.and_then(|t| t.dir()) else {
        return Box::new(err);
    };
    if dir.is_empty() {
        return Box::new(err);
    }
    let suffix = format!("; scratch files may also go under $TMPDIR ({dir})");
    match err {
        ConfineError::Outside(mut refusal) => {
            refusal.message += &suffix;
            Box::new(ConfineError::Outside(refusal))
        }
        ConfineError::Resolve { target, message } => Box::new(ConfineError::Resolve {
            target,
            message: format!("{message}{suffix}"),
        }),
        other => Box::new(other),
    }
}

/// confine.go `underSessionTemp`: whether target sits in the session's own
/// temporary directory. Bash already writes there — it is where $TMPDIR
/// points — so a writer refusing it sends a run that was told to keep
/// scratch out of the repository back into the repository.
pub fn under_session_temp(temp: Option<&dyn SessionTempSource>, target: &str) -> bool {
    let Some(dir) = temp.and_then(|t| t.dir()) else {
        return false;
    };
    if dir.is_empty() {
        return false;
    }
    let roots = real_roots(std::slice::from_ref(&dir));
    !roots.is_empty() && confine(&roots, target).is_ok()
}

/// The stat `missing_root` consumes (roots.rs convention).
pub fn stat_path(path: &str) -> std::io::Result<std::fs::Metadata> {
    std::fs::metadata(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> String {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(&base)
            .unwrap();
        dir.path().to_string_lossy().into_owned()
    }

    #[test]
    fn within_separates_prefixes() {
        let (root, sep) = if cfg!(windows) {
            ("C:\\work\\proj", "\\")
        } else {
            ("/work/proj", "/")
        };
        assert!(within(root, root));
        assert!(within(
            root,
            &pathgo::clean(&format!("{root}{sep}a{sep}b.go"))
        ));
        assert!(within(
            root,
            &pathgo::clean(&format!("{root}{sep}..{sep}proj{sep}x"))
        ));
        let sibling = if cfg!(windows) {
            "C:\\work\\other"
        } else {
            "/work/other"
        };
        assert!(!within(root, sibling));
        assert!(!within(root, &format!("{root}-2")));
        let elsewhere = if cfg!(windows) {
            "C:\\etc\\passwd"
        } else {
            "/etc/passwd"
        };
        assert!(!within(root, elsewhere));
        let parent = if cfg!(windows) { "C:\\work" } else { "/work" };
        assert!(!within(root, parent));
    }

    #[test]
    fn real_roots_resolves_deep_missing_paths_lexically() {
        let roots = real_roots(&[String::from(if cfg!(windows) {
            "C:\\no\\such\\dir\\here"
        } else {
            "/no/such/dir/here"
        })]);
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn confine_read_empty_is_unconfined() {
        assert!(!confine_read(&[], "/anywhere"));
    }

    #[test]
    fn confine_read_inside_and_outside() {
        let root = tmp("confine-read");
        let forbid = real_roots(std::slice::from_ref(&root));
        assert!(confine_read(
            &forbid,
            &pathgo::join(&[&root, "secret", "key.pem"])
        ));
        let other = tmp("confine-other");
        assert!(!confine_read(&forbid, &pathgo::join(&[&other, "ok.txt"])));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn confine_refusal_prints_target_verbatim() {
        let root = tmp("confine-refusal");
        let target = if cfg!(windows) {
            String::from("C:\\Users\\EXAMPLE~1\\AppData\\Local\\Temp\\notes.md")
        } else {
            String::from("/tmp/notes.md")
        };
        let err = confine(std::slice::from_ref(&root), &target).unwrap_err();
        let ConfineError::Outside(refusal) = err else {
            panic!("want outside refusal");
        };
        assert_eq!(refusal.code, CODE_WRITE_OUTSIDE_SCOPE);
        let msg = refusal.message.clone();
        assert!(msg.contains(&format!("`{target}`")), "{msg}");
        assert!(!msg.contains("\\\\"), "escaped backslashes: {msg}");
        assert!(msg.contains(&format!("only under {root}.")), "{msg}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
