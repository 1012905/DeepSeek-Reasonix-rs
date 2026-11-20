//! Network-path refusals by spelling and the resolved-path projection the
//! read tools share (network_scope.go + internal/base/fileutil netpath.go /
//! netscope.go / workspace.go ResolvedPath).

#![forbid(unsafe_code)]

use super::pathgo;
use crate::refusal::Refusal;

/// `workspace.network_path_outside_scope` — Windows connects to another
/// machine to resolve the name, so the refusal is made on the spelling,
/// before any lookup.
pub const CODE_NETWORK_PATH_OUTSIDE_SCOPE: &str = "workspace.network_path_outside_scope";

/// The host's path rule (network_scope.go `windowsPaths`).
pub fn host_is_windows() -> bool {
    cfg!(windows)
}

/// Go `fileutil.IsNetworkPath`: a Windows network or device path by spelling
/// alone — two leading separators or an NT `\??\` prefix, unless a drive
/// follows. Callers apply it where Windows rules hold.
pub fn is_network_path(path: &str) -> bool {
    if let Some(rest) = nt_namespace(path) {
        return !starts_with_drive(rest);
    }
    let b = path.as_bytes();
    if b.len() < 2 || !is_sep(b[0]) || !is_sep(b[1]) {
        return false;
    }
    let rest = &b[2..];
    if rest.len() >= 2 && (rest[0] == b'?' || rest[0] == b'.') && is_sep(rest[1]) {
        return !starts_with_drive_bytes(&rest[2..]);
    }
    true
}

fn nt_namespace(path: &str) -> Option<&str> {
    let b = path.as_bytes();
    if b.len() >= 4 && is_sep(b[0]) && b[1] == b'?' && b[2] == b'?' && is_sep(b[3]) {
        return Some(&path[4..]);
    }
    None
}

fn starts_with_drive(s: &str) -> bool {
    starts_with_drive_bytes(s.as_bytes())
}

fn starts_with_drive_bytes(s: &[u8]) -> bool {
    if s.len() < 2 || s[1] != b':' || !(s[0] | 0x20).is_ascii_lowercase() {
        return false;
    }
    s.len() == 2 || is_sep(s[2])
}

fn is_sep(b: u8) -> bool {
    b == b'\\' || b == b'/'
}

/// Go `fileutil.NetworkScopeOn`: a network path passes only below a root
/// that is itself a network path; the verdict is lexical, since resolving a
/// share is itself a connection. `None` on Windows-host paths means the
/// lexical check does not apply and everything passes.
pub fn network_scope_on(windows: bool, path: &str, roots: &[String]) -> Option<String> {
    if !windows {
        return None;
    }
    let mut path = String::from(path);
    if !is_network_path(&path) {
        let abs = abs_on_windows(&path)?;
        if !is_network_path(&abs) {
            return None;
        }
        path = abs;
    }
    if let Some(target) = network_parts(&path) {
        for root in roots {
            if !is_network_path(root) {
                continue;
            }
            if let Some(base) = network_parts(root)
                && base.len() <= target.len()
                && target[..base.len()] == base[..]
            {
                return None;
            }
        }
    }
    Some(path)
}

fn abs_on_windows(path: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    pathgo::abs_path(path).ok()
}

/// Go `networkParts`: `\\host\share\rest` into folded components, host
/// first, trailing dots/spaces dropped, `..` clamped at the share. A
/// spelling that cannot be placed under a share reports None.
fn network_parts(path: &str) -> Option<Vec<String>> {
    let mut p = path.replace('/', "\\");
    let lower_starts =
        |prefix: &str| p.len() >= prefix.len() && p[..prefix.len()].eq_ignore_ascii_case(prefix);
    if lower_starts("\\\\?\\UNC\\") || lower_starts("\\\\.\\UNC\\") || lower_starts("\\??\\UNC\\") {
        p = p["\\\\?\\UNC\\".len()..].to_owned();
    } else if p.starts_with("\\\\") && !p.starts_with("\\\\\\") && !is_namespace_prefix(&p) {
        p = p[2..].to_owned();
    } else {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for c in p.split('\\') {
        match c {
            "" | "." => continue,
            ".." => {
                if parts.len() > 2 {
                    parts.pop();
                }
                continue;
            }
            _ => {}
        }
        let c = c.trim_end_matches(['.', ' ']);
        if c.is_empty() {
            return None;
        }
        parts.push(c.to_lowercase());
    }
    if parts.len() >= 2 { Some(parts) } else { None }
}

fn is_namespace_prefix(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 4 && (b[2] == b'?' || b[2] == b'.') && b[3] == b'\\'
}

/// network_scope.go `refuseNetworkPath`: the one decision every file tool
/// asks before it resolves a model-supplied path.
pub fn refuse_network_path(target: &str, roots: &[&str]) -> Option<Refusal> {
    let all: Vec<String> = roots.iter().map(|s| String::from(*s)).collect();
    network_scope_on(host_is_windows(), target, &all)?;
    Some(Refusal::code(
        CODE_NETWORK_PATH_OUTSIDE_SCOPE,
        format!(
            "refused: `{target}` is a network path, a file on another machine, outside this workspace. \
             It was refused by its spelling and never looked up; file tools reach a network path only \
             below a workspace folder that is itself on the network"
        ),
    ))
}

/// network_scope.go `networkRoots`: the configured spellings of the folders a
/// run works in, unresolved — resolving a root that names a share would
/// itself be a lookup.
pub fn network_roots(dir: &str, groups: &[&[String]]) -> Vec<String> {
    let mut roots = vec![String::from(dir)];
    for g in groups {
        roots.extend(g.iter().cloned());
    }
    roots
}

/// network_scope.go `networkSpellings`: roots already spelled as network
/// paths, kept unresolved for prefix comparison.
pub fn network_spellings(roots: &[String]) -> Vec<String> {
    roots
        .iter()
        .filter(|r| !r.is_empty() && host_is_windows() && is_network_path(r))
        .cloned()
        .collect()
}

/// workspace.go `resolveIn`: a tool path argument into a working directory.
/// Empty workDir passes through; a defaulted "." targets the workspace root;
/// an absolute path is honored verbatim.
pub fn resolve_in(work_dir: &str, p: &str) -> String {
    if work_dir.is_empty() {
        return String::from(p);
    }
    if p.is_empty() || p == "." {
        return String::from(work_dir);
    }
    if pathgo::is_abs(p) {
        return String::from(p);
    }
    pathgo::join(&[work_dir, p])
}

/// workspace.go `ResolvedPath`: the local path used for I/O plus the token
/// spelling for output. The token-alias resolver (workspace.go
/// PathResolver, §7.7) is anchored in mod.rs — this constructor covers the
/// plain work-dir resolution the read tools perform today.
pub fn resolve_readable_path(work_dir: &str, path: &str) -> ResolvedPath {
    let p = resolve_in(work_dir, path);
    ResolvedPath {
        path: p.clone(),
        display_path: p.clone(),
        root: p,
        display_root: String::new(),
        external: false,
    }
}

/// A resolved read target (workspace.go `ResolvedPath`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPath {
    pub path: String,
    pub display_path: String,
    pub root: String,
    pub display_root: String,
    pub external: bool,
}

impl ResolvedPath {
    /// Rewrites a local path under the root back to the token spelling; a
    /// raw path falls back when it escapes the root.
    pub fn display_for(&self, path: &str) -> String {
        if !self.external {
            return String::from(path);
        }
        match pathgo::rel(&self.root, path) {
            Ok(rel) if is_local(&rel) => {
                if rel == "." {
                    return self.display_root.clone();
                }
                format!("{}/{}", self.display_root, pathgo::to_slash(&rel))
            }
            _ => String::from(path),
        }
    }

    /// External error text never leaks the local root (workspace.go
    /// `ErrorText`).
    pub fn error_text(&self, err: &std::io::Error) -> String {
        let msg = err.to_string();
        if !self.external {
            return msg;
        }
        msg.replace(&self.root, &self.display_root)
    }

    /// network_scope.go `ResolvedPath.refuseNetwork`.
    pub fn refuse_network(&self, roots: &[String]) -> Option<Refusal> {
        if self.external {
            let mut all: Vec<String> = roots.to_vec();
            all.push(self.root.clone());
            refuse_network_path(
                &self.path,
                &all.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )
        } else {
            refuse_network_path(
                &self.path,
                &roots.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )
        }
    }
}

/// Go `filepath.IsLocal`: not empty, not rooted, no volume, no `..`
/// traversal.
pub fn is_local(path: &str) -> bool {
    if path.is_empty() || path == "." {
        return path == ".";
    }
    if pathgo::is_abs(path) {
        return false;
    }
    if pathgo::volume_name_len(path) > 0 {
        return false;
    }
    let sep = pathgo::separator();
    let has_sep = path
        .chars()
        .any(|c| c == sep || (cfg!(windows) && c == '/'));
    let mut rel = if has_sep {
        path.replace('\\', "/")
    } else {
        String::from(path)
    };
    if cfg!(windows) {
        rel = rel.replace('/', "\\");
    }
    let cleaned = pathgo::clean(&rel);
    if cleaned == ".." || cleaned.starts_with(&format!("..{sep}")) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_path_spelling_verdicts() {
        assert!(is_network_path("\\\\host\\share\\file"));
        assert!(is_network_path("//host/share/file"));
        assert!(is_network_path("\\\\?\\UNC\\host\\share"));
        assert!(!is_network_path("\\\\?\\C:\\drive"));
        assert!(!is_network_path("C:\\x"));
        assert!(!is_network_path("relative\\path"));
    }

    #[test]
    fn resolve_in_defaults_and_absolutes() {
        let sep = pathgo::separator();
        assert_eq!(resolve_in("", "x"), "x");
        assert_eq!(resolve_in("w", "."), "w");
        assert_eq!(resolve_in("w", ""), "w");
        let abs = if cfg!(windows) { "C:\\a" } else { "/a" };
        assert_eq!(resolve_in("w", abs), abs);
        assert_eq!(resolve_in("w", "sub"), format!("w{sep}sub"));
    }

    #[test]
    fn display_for_projects_external_paths() {
        let sep = pathgo::separator();
        let rp = ResolvedPath {
            path: format!("C:{sep}physical{sep}sub"),
            display_path: String::new(),
            root: format!("C:{sep}physical"),
            display_root: String::from("token"),
            external: true,
        };
        assert_eq!(
            rp.display_for(&format!("C:{sep}physical{sep}sub{sep}f.txt")),
            "token/sub/f.txt"
        );
    }
}
