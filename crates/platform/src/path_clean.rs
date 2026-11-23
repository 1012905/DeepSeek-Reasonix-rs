use std::path::{Path, PathBuf};

/// Lexical cleanup equivalent to Go `filepath.Clean`: collapses repeated
/// separators and resolves `.` / `..` without touching the filesystem. Used for
/// path identity comparison only; the resolved absolute spelling a reader sees
/// is produced by the config side.
pub(crate) fn clean_lexical(path: &Path) -> PathBuf {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    let absolute = normalized.starts_with('/');
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut out = if absolute {
        "/".to_string()
    } else {
        String::new()
    };
    out.push_str(&parts.join("/"));
    if out.is_empty() {
        out = ".".to_string();
    }
    PathBuf::from(out)
}
