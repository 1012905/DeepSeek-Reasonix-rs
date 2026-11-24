//! ACP-039: additional-directory vetting and its refusal codes
//! (`additional_dirs.go:10-88`; `workspace_roots.go:23-31`).
//!
//! The count and per-path length caps reuse [`crate::session_new`]'s existing
//! constants. Real `config.VetExtraRoot` resolution, symlink evaluation, and
//! grant-store writes are runtime wiring; this module owns the refusal-code
//! vocabulary, the whole-list rejection rule, the deduplicated resolved-root
//! result, and the open-time directory comparison.

use crate::session_new::{
    DIRS_DISABLED, DIRS_TOO_MANY, MAX_ADDITIONAL_DIR_LEN, MAX_ADDITIONAL_DIRS,
};

/// Whole-request refusal: more than thirty-two folders
/// (`additional_dirs.go:17,42-44`).
pub const CODE_TOO_MANY: &str = DIRS_TOO_MANY;
/// Open with a different folder set — close the existing session first
/// (`additional_dirs.go:18`).
pub const CODE_SESSION_OPEN: &str = "acp.additional_directories.session_open";
/// Factory started write-only inside the session cwd (`additional_dirs.go:19`).
pub const CODE_DISABLED: &str = DIRS_DISABLED;

/// Per-folder workspace-root refusal identities (`workspace_roots.go:24-30`).
pub const ROOT_NOT_DIRECTORY: &str = "workspace_roots.not_directory";
pub const ROOT_COVERED_ALREADY: &str = "workspace_roots.covered_by_primary";
pub const ROOT_TOO_BROAD: &str = "workspace_roots.too_broad";
pub const ROOT_PROTECTED: &str = "workspace_roots.protected";
pub const ROOT_SKILL_STORE: &str = "workspace_roots.skill_store";
pub const ROOT_NOT_OWNED: &str = "workspace_roots.not_owned";
pub const ROOT_OVERLAPS_WRITE: &str = "workspace_roots.overlaps_write_root";

/// The factory's write-only-cwd sentinel message (`additional_dirs.go:24`).
pub const ERR_DISABLED_MESSAGE: &str = "this agent writes only inside the session's cwd";

/// One additional-directory refusal: stable code, the folder when one does,
/// and the kernel English message (`additional_dirs.go:28-31`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirsRefusal {
    pub code: String,
    pub path: Option<String>,
    pub message: String,
}

/// A workspace-root vetting failure the runtime config layer reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRefusal {
    pub code: String,
}

/// Resolves and vets one declared extra root against the roots already kept
/// for this session. The runtime backs this with `config.VetExtraRoot`.
pub trait ExtraRootVetter {
    /// Returns the resolved real path, or the workspace-root refusal code.
    fn vet_extra_root(
        &self,
        cwd: &str,
        dir: &str,
        existing: &[String],
    ) -> Result<String, RootRefusal>;
}

/// Vets the declared folders: count cap, per-path length cap, per-folder root
/// vet, then dedup on the resolved real path. One refusal rejects the whole
/// request; nothing is written to the user's grants (`additional_dirs.go:38-67`).
pub fn vet_additional_dirs(
    cwd: &str,
    dirs: &[String],
    vetter: &dyn ExtraRootVetter,
) -> Result<Vec<String>, DirsRefusal> {
    if dirs.is_empty() {
        return Ok(Vec::new());
    }
    if dirs.len() > MAX_ADDITIONAL_DIRS {
        return Err(DirsRefusal {
            code: CODE_TOO_MANY.to_owned(),
            path: None,
            message: "too many additional directories".to_owned(),
        });
    }

    let mut resolved: Vec<String> = Vec::new();
    for dir in dirs {
        if dir.chars().count() > MAX_ADDITIONAL_DIR_LEN {
            // The reference maps an over-long path onto the not-directory root
            // code (`additional_dirs.go:51-53`).
            return Err(DirsRefusal {
                code: ROOT_NOT_DIRECTORY.to_owned(),
                path: Some(dir.clone()),
                message: "additional directory path is too long".to_owned(),
            });
        }
        let real = vetter
            .vet_extra_root(cwd, dir, &resolved)
            .map_err(|r| DirsRefusal {
                code: r.code,
                path: Some(dir.clone()),
                message: "this folder cannot be added".to_owned(),
            })?;
        if !resolved.contains(&real) {
            resolved.push(real);
        }
    }
    Ok(resolved)
}

/// Folder sets compare as sorted real paths (`additional_dirs.go:73-79`).
pub fn same_dirs(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut left: Vec<&str> = a.iter().map(String::as_str).collect();
    let mut right: Vec<&str> = b.iter().map(String::as_str).collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

/// Open-time mismatch: the persisted session's folders differ from the
/// request's, so the call is refused rather than merged
/// (`additional_dirs.go:18`; `service.go:486-524`).
pub fn open_dirs_refusal(
    method: &str,
    requested: &[String],
    persisted: &[String],
) -> Option<DirsRefusal> {
    if same_dirs(requested, persisted) {
        return None;
    }
    Some(DirsRefusal {
        code: CODE_SESSION_OPEN.to_owned(),
        path: None,
        message: format!(
            "{method}: the session is open with other additional directories; close it first"
        ),
    })
}

/// Maps a factory failure onto the client-facing refusal
/// (`additional_dirs.go:83-88`). The disabled sentinel is the client's to fix;
/// every other factory failure stays a session-open fault the runtime classifies.
pub fn disabled_refusal(method: &str) -> DirsRefusal {
    DirsRefusal {
        code: CODE_DISABLED.to_owned(),
        path: None,
        message: format!("{method}: {ERR_DISABLED_MESSAGE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeVetter {
        refusals: Vec<(&'static str, &'static str)>,
    }

    impl ExtraRootVetter for FakeVetter {
        fn vet_extra_root(
            &self,
            _cwd: &str,
            dir: &str,
            _existing: &[String],
        ) -> Result<String, RootRefusal> {
            for (needle, code) in &self.refusals {
                if dir.contains(needle) {
                    return Err(RootRefusal {
                        code: (*code).to_owned(),
                    });
                }
            }
            Ok(format!("real:{dir}"))
        }
    }

    #[test]
    fn empty_list_passes_through_without_calling_the_vetter() {
        let vetter = FakeVetter { refusals: vec![] };
        let out = vet_additional_dirs("/ws", &[], &vetter).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn more_than_thirty_two_folders_refuses_whole_with_too_many_code() {
        let vetter = FakeVetter { refusals: vec![] };
        let dirs: Vec<String> = (0..=MAX_ADDITIONAL_DIRS)
            .map(|i| format!("/d/{i}"))
            .collect();
        let err = vet_additional_dirs("/ws", &dirs, &vetter).unwrap_err();
        assert_eq!(err.code, CODE_TOO_MANY);
        assert!(err.path.is_none());
        assert_eq!(err.message, "too many additional directories");
    }

    #[test]
    fn over_long_path_carries_the_not_directory_code_and_the_folder() {
        let vetter = FakeVetter { refusals: vec![] };
        let long = "x".repeat(MAX_ADDITIONAL_DIR_LEN + 1);
        let err = vet_additional_dirs("/ws", std::slice::from_ref(&long), &vetter).unwrap_err();
        assert_eq!(err.code, ROOT_NOT_DIRECTORY);
        assert_eq!(err.path, Some(long));
        assert_eq!(err.message, "additional directory path is too long");
    }

    #[test]
    fn per_folder_refusal_keeps_the_root_identity_and_folder() {
        let vetter = FakeVetter {
            refusals: vec![(".ssh", ROOT_PROTECTED)],
        };
        let err = vet_additional_dirs("/ws", &["/home/u/.ssh".to_owned()], &vetter).unwrap_err();
        assert_eq!(err.code, ROOT_PROTECTED);
        assert_eq!(err.path, Some("/home/u/.ssh".to_owned()));
        assert_eq!(err.message, "this folder cannot be added");
    }

    #[test]
    fn resolved_real_paths_are_deduplicated() {
        let vetter = FakeVetter { refusals: vec![] };
        let out = vet_additional_dirs(
            "/ws",
            &["/sibling".to_owned(), "/sibling/".to_owned()],
            &vetter,
        )
        .unwrap();
        assert_eq!(
            out,
            vec!["real:/sibling".to_owned(), "real:/sibling/".to_owned()]
        );
    }

    #[test]
    fn same_dirs_compares_sorted_regardless_of_order() {
        assert!(same_dirs(
            &["/a".to_owned(), "/b".to_owned()],
            &["/b".to_owned(), "/a".to_owned()]
        ));
        assert!(!same_dirs(
            &["/a".to_owned(), "/b".to_owned()],
            &["/a".to_owned()]
        ));
        assert!(!same_dirs(&["/a".to_owned()], &["/b".to_owned()]));
    }

    #[test]
    fn open_refusal_fires_only_on_a_folder_mismatch() {
        let persisted = vec!["/sibling".to_owned()];
        assert!(open_dirs_refusal("session/resume", &persisted, &persisted).is_none());
        let err = open_dirs_refusal("session/resume", &["/other".to_owned()], &persisted).unwrap();
        assert_eq!(err.code, CODE_SESSION_OPEN);
        assert_eq!(
            err.message,
            "session/resume: the session is open with other additional directories; close it first"
        );
    }

    #[test]
    fn disabled_factory_refusal_carries_the_off_code_and_sentinel_message() {
        let err = disabled_refusal("session/new");
        assert_eq!(err.code, CODE_DISABLED);
        assert_eq!(err.message, format!("session/new: {ERR_DISABLED_MESSAGE}"));
    }
}
