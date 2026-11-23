//! User-global config candidate set, membership test, and managed repair set
//! (paths.md §2.1/§2.3, config.md §12.5).
//!
//! Which directories resolve to primary/legacy/XDG is reasonix-config's
//! selection chain; this slice composes and compares the already-resolved
//! locations the caller injects.

use std::path::{Path, PathBuf};

use crate::path_clean::clean_lexical;

/// Candidate locations in precedence order, empties dropped and duplicates
/// collapsed (Go `userConfigCandidatePaths`).
pub fn candidate_paths(
    primary: Option<&Path>,
    legacy: &[PathBuf],
    xdg: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_unique(&mut out, primary);
    for path in legacy {
        push_unique(&mut out, Some(path.as_path()));
    }
    for path in xdg {
        push_unique(&mut out, Some(path.as_path()));
    }
    out
}

/// Managed config FILES a repair may touch: primary + legacy TOMLs + legacy
/// v0.x json, deduped (Go `managedConfigPaths`). Directories and the sensitive
/// home siblings never ride along.
pub fn managed_config_paths(
    primary: Option<&Path>,
    legacy: &[PathBuf],
    legacy_json: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_unique(&mut out, primary);
    for path in legacy {
        push_unique(&mut out, Some(path.as_path()));
    }
    push_unique(&mut out, legacy_json);
    out
}

/// Abs+Clean equality, falling back to Clean equality when Abs cannot resolve
/// (Go `samePath`).
pub fn same_path(a: &Path, b: &Path) -> bool {
    if a.as_os_str().is_empty() || b.as_os_str().is_empty() {
        return false;
    }
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(abs_a), Ok(abs_b)) => clean_lexical(&abs_a) == clean_lexical(&abs_b),
        _ => clean_lexical(a) == clean_lexical(b),
    }
}

/// Whether `path` names one of the user config candidates (Go
/// `isUserConfigPath`).
pub fn is_user_config_path(candidates: &[PathBuf], path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    candidates
        .iter()
        .any(|candidate| same_path(candidate, path))
}

fn push_unique(out: &mut Vec<PathBuf>, path: Option<&Path>) {
    let Some(path) = path else {
        return;
    };
    if path.as_os_str().is_empty() {
        return;
    }
    let cleaned = clean_lexical(path);
    if out.iter().any(|existing| same_path(existing, &cleaned)) {
        return;
    }
    out.push(cleaned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn candidate_paths_preserve_precedence_and_drop_duplicates() {
        let tmp = test_support::tempdir();
        let primary = tmp.path().join("config.toml");
        let legacy = tmp.path().join("legacy").join("config.toml");
        let xdg = tmp.path().join("xdg").join("config.toml");

        let paths = candidate_paths(
            Some(&primary),
            std::slice::from_ref(&legacy),
            &[xdg.clone(), primary.clone()],
        );

        assert_eq!(paths.len(), 3);
        assert!(same_path(&paths[0], &primary));
        assert!(same_path(&paths[1], &legacy));
        assert!(same_path(&paths[2], &xdg));
    }

    #[test]
    fn candidate_paths_drop_empty_locations() {
        let paths = candidate_paths(Some(Path::new("")), &[PathBuf::from("")], &[]);
        assert!(paths.is_empty());
    }

    #[test]
    fn same_path_compares_abs_then_clean() {
        let tmp = test_support::tempdir();
        let a = tmp.path().join("config.toml");
        let b = tmp.path().join(".").join("config.toml");
        assert!(same_path(&a, &b));

        let other = tmp.path().join("other.toml");
        assert!(!same_path(&a, &other));
        assert!(!same_path(Path::new(""), &a));
    }

    #[test]
    fn membership_matches_candidates_by_identity() {
        let tmp = test_support::tempdir();
        let primary = tmp.path().join("config.toml");
        let legacy = tmp.path().join("legacy").join("config.toml");
        let candidates = candidate_paths(Some(&primary), std::slice::from_ref(&legacy), &[]);

        assert!(is_user_config_path(&candidates, &primary));
        assert!(is_user_config_path(&candidates, &legacy));
        assert!(!is_user_config_path(
            &candidates,
            &tmp.path().join("settings.json")
        ));
        assert!(!is_user_config_path(&candidates, Path::new("")));
    }

    #[test]
    fn managed_paths_collect_only_config_files_deduped() {
        let tmp = test_support::tempdir();
        let primary = tmp.path().join("config.toml");
        let legacy = tmp.path().join("legacy").join("config.toml");
        let legacy_json = tmp.path().join(".reasonix").join("config.json");

        let managed = managed_config_paths(
            Some(&primary),
            &[legacy.clone(), primary.clone()],
            Some(&legacy_json),
        );

        assert_eq!(managed.len(), 3);
        assert!(same_path(&managed[0], &primary));
        assert!(same_path(&managed[1], &legacy));
        assert!(same_path(&managed[2], &legacy_json));
        for path in &managed {
            let base = path.file_name().unwrap();
            assert!(base == "config.toml" || base == "config.json");
        }
    }
}
