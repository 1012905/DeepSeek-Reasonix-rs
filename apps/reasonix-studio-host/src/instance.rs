//! The Studio instance id (docs/contracts/host.md §5.1): one data home is one
//! Studio, and the identity is the canonicalized home, not the spelling.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "io.reasonix.studio";

pub fn instance_id_for(home: &str) -> String {
    let trimmed = home.trim();
    if trimmed.is_empty() {
        return PREFIX.to_string();
    }
    let mut root = trimmed.to_string();
    let marker = canonical_existing(
        &Path::new(&root)
            .join(".reasonix-home.identity")
            .to_string_lossy(),
    );
    if let Some(parent) = marker.and_then(|m| Path::new(&m).parent().map(|p| p.to_path_buf())) {
        root = parent.to_string_lossy().into_owned();
    }
    let cleaned = clean_path(&root);
    let sum = Sha256::digest(cleaned.as_bytes());
    format!("{}.{}", PREFIX, hex(&sum[..8]))
}

/// Physical path of an existing file, symlinks resolved, the Windows `\\?\`
/// long-path prefix stripped — the shape the identity hashes. `None` when the
/// path does not exist; the full walk-through-nearest-existing-ancestor of the
/// reference is a state-crate concern this binary must not reach (frozen
/// dependency policy), so a not-yet-created home hashes as spelled — recorded
/// in OPEN_QUESTIONS.md.
fn canonical_existing(path: &str) -> Option<String> {
    let resolved = std::fs::canonicalize(path).ok()?;
    let mut key = resolved.to_string_lossy().into_owned();
    if cfg!(windows) {
        if let Some(stripped) = key.strip_prefix("\\\\?\\UNC\\") {
            key = format!("\\\\{stripped}");
        } else if let Some(stripped) = key.strip_prefix("\\\\?\\") {
            key = stripped.to_string();
        }
    }
    Some(key)
}

/// Lexical `filepath.Clean`: collapses separators, resolves `.` and `..`,
/// drops a trailing separator.
fn clean_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().map(|p| p == "..").unwrap_or(true) && !absolute && parts.is_empty() {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().map(|b| {
        [HEX[(b >> 4) as usize] as char, HEX[(b & 0x0f) as usize] as char]
    }).flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_stays_the_lock_existing_installs_hold() {
        assert_eq!(PREFIX, "io.reasonix.studio");
    }

    #[test]
    fn an_empty_home_names_the_bare_prefix() {
        assert_eq!(instance_id_for(""), PREFIX);
        assert_eq!(instance_id_for("   "), PREFIX);
    }

    #[test]
    fn the_id_is_prefix_plus_eight_bytes_hex() {
        let id = instance_id_for("/some/home");
        assert!(id.starts_with("io.reasonix.studio."));
        let suffix = &id["io.reasonix.studio.".len()..];
        assert_eq!(suffix.len(), 16);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn an_existing_marker_canonicalizes_the_home() {
        // Directories live under target/ because the system Temp is not
        // writable under this environment's antivirus policy.
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/instance");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join(".reasonix-home.identity"), "x").unwrap();
        let id = instance_id_for(&base.to_string_lossy());
        let expected_root = clean_path(&base.to_string_lossy());
        let suffix = hex(&Sha256::digest(expected_root.as_bytes())[..8]);
        assert_eq!(id, format!("{}.{}", PREFIX, suffix));
    }

    #[test]
    fn clean_collapses_dot_segments() {
        assert_eq!(clean_path("/a/./b/../c/"), "/a/c");
        assert_eq!(clean_path("a//b"), "a/b");
        assert_eq!(clean_path(""), "");
    }
}
