//! The Studio instance id (docs/contracts/host.md §5.1): one data home is one
//! Studio, and the identity is the canonicalized home, not the spelling.

use reasonix_state::sessionstore::canonical_session_path;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const PREFIX: &str = "io.reasonix.studio";

pub fn instance_id_for(home: &str) -> String {
    let trimmed = home.trim();
    if trimmed.is_empty() {
        return PREFIX.to_string();
    }
    let mut root = trimmed.to_string();
    let marker = canonical_session_path(
        &Path::new(&root)
            .join(".reasonix-home.identity")
            .to_string_lossy(),
    );
    if !marker.is_empty() {
        if let Some(parent) = Path::new(&marker).parent() {
            root = parent.to_string_lossy().into_owned();
        }
    }
    let cleaned = clean_path(&root);
    let sum = Sha256::digest(cleaned.as_bytes());
    format!("{}.{}", PREFIX, hex(&sum[..8]))
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
    fn clean_collapses_dot_segments() {
        assert_eq!(clean_path("/a/./b/../c/"), "/a/c");
        assert_eq!(clean_path("a//b"), "a/b");
        assert_eq!(clean_path(""), "");
    }
}
