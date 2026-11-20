//! Roots: the directories a call acts under, and the typed cause when one
//! is gone (tools.md §1.12).

use std::path::Path;

/// Which kind of directory a call needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// The session's project folder.
    Workspace,
    /// A folder the person granted from outside the project.
    External,
}

/// `tool.root_missing` — the call was not run because a directory it needs
/// no longer exists. It is the host's own finding, not a permission or
/// argument problem.
pub const CODE_ROOT_MISSING: &str = "tool.root_missing";

/// The error identity every root-missing failure carries (Go
/// `ErrRootMissing`); callers classify by downcast, never by wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootMissing {
    Missing,
}

/// One directory a call acts under. `path` is where the host looks;
/// `display` is what the model and the UI are told, so an external folder
/// is named by the token it was granted under and never by the real path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub kind: RootKind,
    pub path: String,
    pub display: String,
}

/// The typed cause carried to the model and every frontend when a directory
/// a call needs does not exist.
#[derive(Debug, Clone)]
pub struct RootMissingError {
    pub root: Root,
}

impl RootMissingError {
    /// Which kind of directory is gone.
    pub fn kind(&self) -> RootKind {
        self.root.kind
    }

    /// The refusal identity a frontend classifies without reading the
    /// sentence.
    pub fn refusal_code(&self) -> &'static str {
        CODE_ROOT_MISSING
    }
}

impl std::fmt::Display for RootMissingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.root.kind {
            RootKind::Workspace => "project folder",
            RootKind::External => "external folder",
        };
        write!(
            f,
            "root missing: the {} {} does not exist. This is not a permission or argument problem: it was moved, renamed or deleted, or its drive is not available. Nothing was run.",
            what, self.root.display
        )
    }
}

impl std::error::Error for RootMissingError {}

/// Whether `err` is the root-missing cause (Go `errors.Is(err, ErrRootMissing)`).
pub fn is_root_missing(err: &(dyn std::error::Error + 'static)) -> bool {
    err.downcast_ref::<RootMissingError>().is_some()
}

/// The first of roots that does not exist as a directory, as a
/// [`RootMissingError`]. A root that cannot be read for another reason (a
/// denied stat) is not missing: the tool reports that itself.
pub fn missing_root(
    roots: &[Root],
    stat: impl Fn(&str) -> Result<std::fs::Metadata, std::io::Error>,
) -> Option<RootMissingError> {
    for r in roots {
        if r.path.is_empty() {
            continue;
        }
        match stat(&r.path) {
            Ok(meta) => {
                if !meta.is_dir() {
                    return Some(RootMissingError { root: r.clone() });
                }
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    return Some(RootMissingError { root: r.clone() });
                }
            }
        }
    }
    None
}

/// Convenience stat over the real filesystem.
pub fn stat_path(path: &str) -> Result<std::fs::Metadata, std::io::Error> {
    std::fs::metadata(Path::new(path))
}
