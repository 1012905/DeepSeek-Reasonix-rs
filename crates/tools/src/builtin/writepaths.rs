//! Write-path identity for the file-writing tools (write_paths.go +
//! internal/base/fileutil canonicalpath.go): resolution against a working
//! directory, session-temp spellings, and the lease/grant canonical form.

#![forbid(unsafe_code)]

use super::confine::{SessionTempSource, real_path};
use super::pathgo;
use crate::refusal::Refusal;

pub use super::network::{host_is_windows, refuse_network_path, resolve_in};

/// The standard temporary-directory variables a session-private temp dir
/// is reached through (sandbox.SessionTempEnvKeys): `$KEY`, `${KEY}` or
/// `%KEY%` on the first path segment.
pub const SESSION_TEMP_ENV_KEYS: [&str; 3] = ["TMPDIR", "TMP", "TEMP"];

/// `write path is required` — the resolver's identity for a missing target.
pub const ERR_WRITE_PATH_REQUIRED: &str = "write path is required";

/// workspace.go `resolveSessionTemp`: rewrites a leading session-temp
/// variable into the directory it names. The host exports these spellings
/// to bash and names $TMPDIR in its own refusal text, so a writer that
/// cannot read them would create a literal `$TMPDIR` directory inside the
/// workspace. Only the first segment is expanded. `None` (Go nil) expands
/// nothing.
pub fn resolve_session_temp(temp: Option<&dyn SessionTempSource>, p: &str) -> String {
    let Some(temp) = temp else {
        return String::from(p);
    };
    if p.is_empty() {
        return String::from(p);
    }
    let slashed = pathgo::to_slash(p);
    let (head, rest) = match slashed.split_once('/') {
        Some((head, rest)) => (head, rest),
        None => (slashed.as_str(), ""),
    };
    let named = SESSION_TEMP_ENV_KEYS.iter().any(|key| {
        head == format!("${key}")
            || head == format!("${{{key}}}")
            || head.eq_ignore_ascii_case(&format!("%{key}%"))
    });
    if !named {
        return String::from(p);
    }
    let dir = match temp.dir() {
        Some(dir) if !dir.is_empty() => dir,
        _ => match temp.ensure_dir() {
            Some(dir) if !dir.is_empty() => dir,
            _ => return String::from(p),
        },
    };
    pathgo::join(&[dir.as_str(), &pathgo::from_slash(rest)])
}

/// canonicalpath.go `ErrAmbiguousPath` and the resolve failures
/// `CanonicalWritePath` answers with.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalPathError {
    /// The path's identity is ambiguous (Windows trailing-dot spellings,
    /// symlinks at the leaf or on a missing ancestor).
    #[error("filesystem path identity is ambiguous")]
    Ambiguous,
    /// The operating system rejected a lookup or an absolute conversion.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl CanonicalPathError {
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, CanonicalPathError::Ambiguous)
    }
}

/// canonicalpath.go `CanonicalWritePath`: resolves existing ancestors while
/// preserving leaf rename semantics. Ambiguous missing Windows components
/// cannot establish an identity.
pub fn canonical_write_path(path: &str) -> Result<String, CanonicalPathError> {
    if host_is_windows() {
        for component in pathgo::to_slash(path).split('/') {
            if component != "." && component != ".." && trim_dots(component) != component {
                return Err(CanonicalPathError::Ambiguous);
            }
        }
    }
    let abs = pathgo::abs_path(path)?;
    if std::fs::symlink_metadata(&abs)
        .map(|info| info.is_symlink())
        .unwrap_or(false)
    {
        return Err(CanonicalPathError::Ambiguous);
    }
    let mut parent = abs.clone();
    let mut tail: Vec<String> = Vec::new();
    loop {
        match resolve_existing(&parent) {
            Ok(resolved) => {
                let mut parts = vec![resolved];
                parts.extend(tail.iter().cloned());
                let mut result =
                    pathgo::join(&parts.iter().map(|s| s.as_str()).collect::<Vec<_>>());
                if tail.is_empty() && host_is_windows() {
                    let base_abs = last_component(&pathgo::clean(&abs));
                    let base_result = last_component(&result);
                    if base_abs.eq_ignore_ascii_case(&base_result) {
                        result = pathgo::join(&[parent_of(&result).as_str(), base_abs.as_str()]);
                    }
                }
                return Ok(result);
            }
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(err.into());
            }
            Err(_) => {
                if std::fs::symlink_metadata(&parent)
                    .map(|info| info.is_symlink())
                    .unwrap_or(false)
                {
                    return Err(CanonicalPathError::Ambiguous);
                }
                let component = last_component(&parent);
                if host_is_windows()
                    && (trim_dots(&component) != component
                        || component.contains('~')
                        || component.contains(':'))
                {
                    return Err(CanonicalPathError::Ambiguous);
                }
                let next = parent_of(&parent);
                if next == parent {
                    return Err(CanonicalPathError::Ambiguous);
                }
                tail.insert(0, component);
                parent = next;
            }
        }
    }
}

/// canonicalpath.go's `TrimRight(component, ". ")` on Windows.
fn trim_dots(component: &str) -> &str {
    component.trim_end_matches(['.', ' '])
}

/// Go `filepath.Base`: trailing separators first, then the last element.
fn last_component(p: &str) -> String {
    let trimmed = p.trim_end_matches(['/', '\\']);
    pathgo::split(trimmed).1
}

/// Go `filepath.Dir`: trailing separators dropped, then the cleaned parent.
fn parent_of(p: &str) -> String {
    let trimmed = p.trim_end_matches(['/', '\\']);
    let (parent, _) = pathgo::split(trimmed);
    pathgo::clean(&parent)
}

/// realpath_other.go `ResolveExistingPath`: the filesystem lookup
/// `CanonicalWritePath` resolves ancestors through.
fn resolve_existing(path: &str) -> std::io::Result<String> {
    let s = std::fs::canonicalize(path)?.to_string_lossy().into_owned();
    Ok(s.trim_start_matches(r"\\?\").to_owned())
}

/// The failures `canonical_writer_path` answers with (write_paths.go keeps
/// the ambiguous identity as a recoverable case).
#[derive(Debug, thiserror::Error)]
pub enum WriterPathError {
    /// The target names a machine other than one a root lives on.
    #[error("{0}")]
    Network(Refusal),
    /// Windows alias spellings cannot establish identity; the caller falls
    /// back to `real_path`.
    #[error("{1}")]
    Ambiguous(#[source] std::io::Error, String),
    /// A lookup failed for a reason that is not ambiguity.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl WriterPathError {
    /// Whether the identity is merely ambiguous (Go `errors.Is(err,
    /// fileutil.ErrAmbiguousPath)`).
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, WriterPathError::Ambiguous(..))
    }
}

/// write_paths.go `canonicalWriterPath`: the lease and grant identity of a
/// write target. The writers themselves keep the path as the caller spelled
/// it, so receipts, file views and refusals name what the model used.
pub fn canonical_writer_path(
    work_dir: &str,
    temp: Option<&dyn SessionTempSource>,
    roots: &[String],
    path: &str,
) -> Result<String, WriterPathError> {
    let resolved = resolve_in(work_dir, &resolve_session_temp(temp, path));
    let root_refs: Vec<&str> = roots.iter().map(|s| s.as_str()).collect();
    if let Some(r) = refuse_network_path(&resolved, &root_refs) {
        return Err(WriterPathError::Network(r));
    }
    match canonical_write_path(&resolved) {
        Ok(canonical) => Ok(canonical),
        Err(e) if e.is_ambiguous() => Err(WriterPathError::Ambiguous(
            std::io::Error::other("ambiguous path"),
            resolved,
        )),
        Err(CanonicalPathError::Io(io)) => Err(io.into()),
        Err(CanonicalPathError::Ambiguous) => unreachable!("handled above"),
    }
}

/// write_paths.go `ResolveWritePaths`: grant targets even when their narrow
/// lease identity is ambiguous. Lease callers must retain whole-workspace
/// exclusion on error. The move tool reads `source_path` +
/// `destination_path`; the single-path writers read `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWritePaths {
    pub paths: Vec<String>,
    /// Set when at least one target's identity was ambiguous and the
    /// realpath fallback was used.
    pub ambiguous: bool,
}

pub fn resolve_write_paths(
    work_dir: &str,
    temp: Option<&dyn SessionTempSource>,
    roots: &[String],
    path: Option<&str>,
    destination: Option<&str>,
    move_: bool,
) -> Result<ResolvedWritePaths, WriterPathError> {
    let mut paths: Vec<String> = Vec::new();
    if move_ {
        paths.push(path.unwrap_or("").to_owned());
        paths.push(destination.unwrap_or("").to_owned());
    } else {
        paths.push(path.unwrap_or("").to_owned());
    }
    if paths.iter().any(|p| p.is_empty()) {
        return Err(WriterPathError::Io(std::io::Error::other(
            ERR_WRITE_PATH_REQUIRED,
        )));
    }
    let mut ambiguous = false;
    for p in &mut paths {
        match canonical_writer_path(work_dir, temp, roots, p) {
            Ok(c) => *p = c,
            Err(e) if e.is_ambiguous() => {
                ambiguous = true;
                *p = real_path(p).map_err(WriterPathError::Io)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(ResolvedWritePaths { paths, ambiguous })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> (tempfile::TempDir, String) {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(&base)
            .unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        (dir, path)
    }

    struct FixedTemp(String);

    impl SessionTempSource for FixedTemp {
        fn dir(&self) -> Option<String> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn session_temp_spelling_expands_first_segment_only() {
        let (_guard, dir) = tmp("writepaths-temp");
        let temp = FixedTemp(dir.clone());
        let sep = pathgo::separator();
        let got = resolve_session_temp(Some(&temp), &format!("$TMPDIR{sep}scratch{sep}a.txt"));
        assert_eq!(got, pathgo::join(&[dir.as_str(), "scratch", "a.txt"]));
        let braced = format!("${{TMPDIR}}{sep}b.txt");
        assert_eq!(
            resolve_session_temp(Some(&temp), &braced),
            pathgo::join(&[dir.as_str(), "b.txt"])
        );
        // Only the first segment expands; a mid-path spelling stays verbatim.
        let deep = format!("{dir}{sep}$TMPDIR{sep}x");
        assert_eq!(resolve_session_temp(Some(&temp), &deep), deep);
        assert_eq!(resolve_session_temp(None, "$TMPDIR/x"), "$TMPDIR/x");
    }

    #[test]
    fn canonical_write_path_resolves_existing_ancestors() {
        let (_guard, dir) = tmp("writepaths-canonical");
        let link_free = real_path(&dir).unwrap();
        let target = pathgo::join(&[link_free.as_str(), "sub", "new.txt"]);
        assert_eq!(canonical_write_path(&target).unwrap(), target);
    }

    #[test]
    fn write_path_required_is_typed() {
        let err = resolve_write_paths("", None, &[], Some(""), None, false).unwrap_err();
        assert!(err.to_string().contains(ERR_WRITE_PATH_REQUIRED));
    }
}
