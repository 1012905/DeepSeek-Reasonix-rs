//! Read-side storage-path derivation: the subset of Go
//! `internal/contract/config` (`storage_roots.go`, `paths.go`) the hub
//! reads need — state root, session dirs, workspace slugs, root names.
//!
//! Anchored to move into `reasonix-config`/`reasonix-contract` when that
//! crate lands its roots table (PATHS-001/003); keep the semantics
//! identical to the reference rather than growing a third path language.
//! Session-path identity itself is never re-derived here — canonical keys
//! always come from `reasonix_state::sessionstore::canonical_session_path`
//! (OPT-007).

use std::path::{Path, PathBuf};

/// Go `Roots.Dir(home)` for the process binding: `REASONIX_HOME` when set,
/// else the OS default home. Empty when the OS cannot name one.
pub fn home_root() -> Option<PathBuf> {
    let pinned = clean_env_dir("REASONIX_HOME");
    if !pinned.is_empty() {
        return Some(PathBuf::from(pinned));
    }
    default_home_dir()
}

/// Go `Roots.Dir(state)` for the process binding: `REASONIX_STATE_HOME`,
/// else the home root (state shares home unless relocated).
pub fn state_root() -> Option<PathBuf> {
    let pinned = clean_env_dir("REASONIX_STATE_HOME");
    if !pinned.is_empty() {
        return Some(PathBuf::from(pinned));
    }
    home_root()
}

/// Go `defaultHomeDir`: the roaming-profile `reasonix` folder on Windows,
/// `~/.reasonix` elsewhere.
fn default_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(dir) = os_user_config_dir() {
            return Some(dir.join("reasonix"));
        }
        if let Some(home) = os_user_home_dir() {
            return Some(home.join("AppData").join("Roaming").join("reasonix"));
        }
        return None;
    }
    if let Some(home) = os_user_home_dir() {
        return Some(home.join(".reasonix"));
    }
    os_user_config_dir().map(|dir| dir.join("reasonix"))
}

/// Go `os.UserConfigDir`: `%AppData%` on Windows, XDG else.
fn os_user_config_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_dir("AppData");
    }
    if let Some(dir) = env_dir("XDG_CONFIG_HOME") {
        return Some(dir);
    }
    os_user_home_dir().map(|home| home.join(".config"))
}

/// Go `os.UserHomeDir`: `%UserProfile%` on Windows, `$HOME` elsewhere.
fn os_user_home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return env_dir("USERPROFILE").or_else(|| env_dir("HOME"));
    }
    env_dir("HOME")
}

fn env_dir(name: &str) -> Option<PathBuf> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v)),
        _ => None,
    }
}

/// Go `config.cleanEnvDir`: read a directory someone wrote in the
/// environment, expanded.
fn clean_env_dir(name: &str) -> String {
    expand_dir_value(&std::env::var(name).unwrap_or_default())
}

/// Go `config.expandDirValue`: trim, expand `${VAR}` / `${VAR:-default}`
/// (an unset variable with no default expands to empty, matching the MCP
/// convention), honor `~`, and land absolute and cleaned.
pub fn expand_dir_value(dir: &str) -> String {
    let mut dir = dir.trim().to_string();
    if dir.is_empty() {
        return String::new();
    }
    dir = expand_vars(&dir);
    if dir == "~" {
        if let Some(home) = os_user_home_dir() {
            dir = home.to_string_lossy().into_owned();
        }
    } else if let Some(rest) = dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\"))
        && let Some(home) = os_user_home_dir()
    {
        dir = home.join(rest).to_string_lossy().into_owned();
    }
    if !Path::new(&dir).is_absolute()
        && let Ok(abs) = std::path::absolute(&dir)
    {
        dir = abs.to_string_lossy().into_owned();
    }
    clean(&dir)
}

/// Go `config.ExpandVars`: `${VAR}` and `${VAR:-default}` references from
/// the process environment.
fn expand_vars(s: &str) -> String {
    expand_vars_with(s, |name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

/// Go `config.expandVarsWithLookup`: the expansion core with the lookup
/// injected, so tests exercise it without touching the environment.
fn expand_vars_with(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    if !s.contains("${") {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"${")
            && let Some(end) = s[i + 2..].find('}').map(|at| i + 2 + at)
            && let Some((name, default)) = split_var_ref(&s[i + 2..end])
        {
            out.push_str(&lookup(name).unwrap_or_else(|| default.unwrap_or_default().to_string()));
            i = end + 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Splits `${NAME}` / `${NAME:-default}` bodies; `None` when the body does
/// not name a variable.
fn split_var_ref(inner: &str) -> Option<(&str, Option<&str>)> {
    let (name, default) = match inner.split_once(":-") {
        Some((name, default)) => (name, Some(default)),
        None => (inner, None),
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return None,
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
        return None;
    }
    Some((name, default))
}

/// Go `filepath.Clean`, lexically, for the shapes this build reads.
pub fn clean(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let volume = volume_prefix(path);
    let rest = &path[volume.len()..];
    let rooted = rest.starts_with('\\') || rest.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in rest.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => match out.last() {
                Some(top) if *top != ".." => {
                    out.pop();
                }
                _ if !rooted && volume.is_empty() => out.push(".."),
                _ => {}
            },
            other => out.push(other),
        }
    }
    let joined = out.join(&separator().to_string());
    if !volume.is_empty() {
        if joined.is_empty() {
            format!("{volume}{sep}", sep = separator())
        } else {
            format!("{volume}{sep}{joined}", sep = separator())
        }
    } else if rooted {
        if joined.is_empty() {
            separator().to_string()
        } else {
            format!("{sep}{joined}", sep = separator())
        }
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// `filepath.Separator` for this OS. Comparisons accept both spellings,
/// the way Go's `filepath` does on Windows.
fn separator() -> char {
    if cfg!(windows) { '\\' } else { '/' }
}

/// Go `filepath.VolumeName`: the `C:` of a drive path, `\\server\share` of
/// a UNC path, else empty. Windows-only, as in Go.
fn volume_prefix(path: &str) -> String {
    if !cfg!(windows) {
        return String::new();
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return path[..2].to_string();
    }
    if let Some(rest) = path.strip_prefix("\\\\")
        && let Some(at) = rest.find(['\\', '/'])
    {
        let share = &rest[at + 1..];
        let share_end = share.find(['\\', '/']).unwrap_or(share.len());
        return format!("\\\\{}\\{}", &rest[..at], &share[..share_end]);
    }
    String::new()
}

/// Go `fileutil.RootName`: what to call a workspace root in a list a
/// person reads. `filepath.Base` answers a separator for a volume root, so
/// a drive letter names the folder instead.
pub fn root_name(root: &str) -> String {
    let root = root.trim();
    if root.is_empty() {
        return String::new();
    }
    let cleaned = clean(root);
    let base = cleaned
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_string();
    if !base.is_empty() && base != "." && base != ".." && base != separator().to_string() {
        return base;
    }
    let volume = volume_prefix(&cleaned);
    if !volume.is_empty() {
        return format!("{volume}{sep}", sep = separator());
    }
    separator().to_string()
}

/// Go `config.WorkspaceSlug`: flatten an absolute workspace path into the
/// directory name used under `<state root>/projects`. Windows spells one
/// folder with varying case, so the slug folds case there — matching the
/// canonical session-path key form.
pub fn workspace_slug(abs_path: &str) -> String {
    let mut path = abs_path.to_string();
    if cfg!(windows) {
        path = path.to_lowercase();
    }
    let slug = path.replace(['\\', '/', ':'], "-");
    bound_filename_component(&slug, 255)
}

/// Go `config.boundFilenameComponent`: cap a derived component at the
/// per-component filesystem limit, appending an FNV-1a hash of the full
/// input so distinct deep paths cannot collapse to one name.
pub fn bound_filename_component(s: &str, max_len: usize) -> String {
    if max_len == 0 || s.len() <= max_len {
        return s.to_string();
    }
    let hash = fnv1a64(s.as_bytes());
    let budget = max_len - 17; // room for "-" + 16 hex digits
    let mut prefix = &s[..budget.min(s.len())];
    while !prefix.is_char_boundary(prefix.len()) {
        prefix = &prefix[..prefix.len() - 1];
    }
    format!("{prefix}-{hash:016x}")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Go `config.ProjectSessionDir`: the per-workspace session directory the
/// desktop sidebar lists — `<state root>/projects/<slug>/sessions`. `None`
/// when either the state root or the workspace root does not resolve.
pub fn project_session_dir(state_root: &Path, workspace_root: &str) -> Option<PathBuf> {
    if state_root.as_os_str().is_empty() {
        return None;
    }
    let root = workspace_root.trim();
    if root.is_empty() {
        return None;
    }
    let abs = match std::path::absolute(root) {
        Ok(abs) => abs.to_string_lossy().into_owned(),
        Err(_) => root.to_string(),
    };
    let slug = workspace_slug(&abs);
    if !is_local_component(&slug) {
        return None;
    }
    Some(state_root.join("projects").join(slug).join("sessions"))
}

/// Go `filepath.IsLocal` for an already-flattened slug: non-empty and
/// neither "." nor "..".
fn is_local_component(slug: &str) -> bool {
    !slug.is_empty() && slug != "." && slug != ".."
}

/// Go serve `SessionDirFor`: where root's transcripts live — the project's
/// own directory when the slug resolves, else the global store.
pub fn session_dir_for(state_root: &Path, workspace_root: &str) -> PathBuf {
    if let Some(dir) = project_session_dir(state_root, workspace_root) {
        return dir;
    }
    state_root.join("sessions")
}

/// Go `config.AutoWorkspaceDir`: the folder a first launch opens when
/// nobody chose a project — one the app owns under the state root.
pub fn auto_workspace_dir(state_root: &Path) -> PathBuf {
    state_root.join("workspaces").join("auto")
}

/// Go `config.DeliveryWorktreeDir` fallback chain (read half): a pinned
/// state root takes the worktrees along; Windows keeps them out of the
/// roaming profile otherwise.
pub fn delivery_worktree_dir(state_root: &Path) -> PathBuf {
    if !clean_env_dir("REASONIX_STATE_HOME").is_empty()
        || !clean_env_dir("REASONIX_HOME").is_empty()
    {
        return state_root.join("worktrees");
    }
    if cfg!(windows)
        && let Some(base) = env_dir("LocalAppData")
    {
        return base.join("reasonix").join("worktrees");
    }
    state_root.join("worktrees")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_name_names_a_drive_root_by_its_letter() {
        if cfg!(windows) {
            assert_eq!(root_name(r"C:\"), "C:\\");
            assert_eq!(root_name(r"C:\projects\demo"), "demo");
            assert_eq!(root_name("/"), "\\");
        } else {
            assert_eq!(root_name("/home/u/demo"), "demo");
            assert_eq!(root_name("/"), "/");
        }
        assert_eq!(root_name("  "), "");
    }

    #[test]
    fn slug_replaces_separators_and_colons() {
        // The drive colon and the separator each become a dash, as in Go;
        // the case fold rides the same compile-time flag as the canonical
        // key form.
        assert_eq!(workspace_slug("/home/u/demo"), "-home-u-demo");
        if cfg!(windows) {
            assert_eq!(workspace_slug(r"C:\Projects\Demo"), "c--projects-demo");
        } else {
            assert_eq!(workspace_slug(r"C:\Projects\Demo"), "C--Projects-Demo");
        }
    }

    #[test]
    fn slug_hashes_only_overlong_components() {
        let short = workspace_slug("C:\\a");
        assert_eq!(short, if cfg!(windows) { "c--a" } else { "C--a" });
        let long = "x".repeat(300);
        let hashed = bound_filename_component(&long, 255);
        assert_eq!(hashed.len(), 255);
        // Distinct deep paths never collapse to one name.
        let other = format!("{}!", "x".repeat(300));
        assert_ne!(bound_filename_component(&other, 255), hashed);
    }

    #[test]
    fn project_session_dir_lands_under_projects() {
        let tmp = crate::test_support::tempdir();
        let dir = project_session_dir(tmp.path(), r"C:\Projects\Demo").unwrap();
        let text = dir.to_string_lossy().into_owned();
        let slug = if cfg!(windows) {
            "c--projects-demo"
        } else {
            "C--Projects-Demo"
        };
        assert!(text.contains("projects"), "{text}");
        assert!(text.contains(slug), "{text}");
        assert!(text.ends_with("sessions"), "{text}");
    }

    #[test]
    fn project_session_dir_refuses_blank_inputs() {
        let tmp = crate::test_support::tempdir();
        assert!(project_session_dir(tmp.path(), "   ").is_none());
        assert!(project_session_dir(Path::new(""), "C:\\a").is_none());
    }

    #[test]
    fn session_dir_for_falls_back_to_the_global_store() {
        let tmp = crate::test_support::tempdir();
        assert_eq!(
            session_dir_for(tmp.path(), "  "),
            tmp.path().join("sessions")
        );
    }

    #[test]
    fn expand_vars_honors_defaults_and_unset() {
        let lookup = |name: &str| (name == "KNOWN").then(|| "v".to_string());
        assert_eq!(expand_vars_with("${KNOWN:-d}/x", lookup), "v/x");
        assert_eq!(expand_vars_with("${MISSING:-d}/x", lookup), "d/x");
        assert_eq!(expand_vars_with("${MISSING}/x", lookup), "/x");
        assert_eq!(expand_vars_with("${not-a-var}/x", lookup), "${not-a-var}/x");
    }

    #[test]
    fn clean_lexically_resolves_dotdot() {
        assert_eq!(
            clean("a/b/../c"),
            if cfg!(windows) { "a\\c" } else { "a/c" }
        );
        assert_eq!(clean(""), ".");
        assert_eq!(clean("./x/."), "x");
        if cfg!(windows) {
            assert_eq!(clean(r"C:\"), "C:\\");
            assert_eq!(clean(r"C:\a\..\b"), r"C:\b");
        }
    }
}
