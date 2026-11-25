//! FileViews (fileviews.go, tools.md §7.5): what one run's agent last saw of
//! each file — the content a read returned it from, or what one of its own
//! writes left there. `write_file` checks it so a whole-file overwrite cannot
//! discard a change made since. A nil record (the hook absent) records and
//! checks nothing.
//!
//! The read-renderer wrapper that captures a complete read (`captureRead`)
//! binds at the file-tool call sites; this module keeps the observation
//! record, the hash comparison, and the overwrite refusals.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::editsource::FileViewsHook;
use super::pathgo;

/// `file changed since the agent last saw it` — a write refused because the
/// file no longer holds what the agent last read or wrote there.
pub const ERR_FILE_CHANGED_SINCE_SEEN: &str = "file changed since the agent last saw it";
/// `...: not read in full` — a whole-file overwrite after an incomplete read.
pub const ERR_FILE_NOT_FULLY_SEEN: &str =
    "file changed since the agent last saw it: not read in full";

/// maxViewBytes: bounds decoded content retained while rendering a read.
pub const MAX_VIEW_BYTES: usize = 8 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileView {
    Complete { hash: [u8; 32] },
    Partial,
}

/// What one run's agent last saw of each file (Go `FileViews`).
#[derive(Debug, Default)]
pub struct FileViews {
    seen: Mutex<BTreeMap<String, FileView>>,
}

/// The overwrite refusals, carrying the Go sentinel identity in the message.
#[derive(Debug, thiserror::Error)]
pub enum FileViewsError {
    /// A partial read was followed by a whole-file overwrite.
    #[error("{0}")]
    NotFullySeen(String),
    /// The current content differs from what the agent last saw.
    #[error("{0}")]
    ChangedSinceSeen(String),
}

impl FileViews {
    /// An empty record.
    pub fn new() -> Self {
        Self::default()
    }

    fn clean(path: &str) -> String {
        pathgo::clean(path)
    }

    /// sha256 of the observed content (Go `sha256.Sum256`).
    fn hash(content: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(content);
        hasher.finalize().into()
    }

    /// Records a complete observation of `content` at `path`.
    pub fn saw(&self, path: &str, content: &[u8]) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.insert(
            Self::clean(path),
            FileView::Complete {
                hash: Self::hash(content),
            },
        );
    }

    /// Records that `path` was only partially observed; an earlier complete
    /// observation is never downgraded by a page or failed read.
    pub fn saw_partial(&self, path: &str) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let clean = Self::clean(path);
        seen.entry(clean).or_insert(FileView::Partial);
    }

    /// captureRead's complete-read criteria: an offset-0 read that reached
    /// EOF, stayed within [`MAX_VIEW_BYTES`], and whose line count fits the
    /// requested limit. Anything else records a partial observation only — a
    /// page, failed read or oversize input must not certify content the caller
    /// did not see.
    pub fn record_read(&self, path: &str, content: &[u8], offset: usize, eof: bool, limit: usize) {
        if offset != 0 || !eof || content.len() > MAX_VIEW_BYTES {
            self.saw_partial(path);
            return;
        }
        let lines = count_lines(content);
        if limit > 0 && lines > limit {
            self.saw_partial(path);
            return;
        }
        self.saw(path, content);
    }

    /// checkOverwrite: refuses replacing `current` when the agent saw different
    /// content there. A file it has never seen is not refused: nothing it
    /// holds can be stale.
    pub fn check_overwrite(&self, path: &str, current: &[u8]) -> Result<(), FileViewsError> {
        let Some(want) = self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&Self::clean(path))
            .copied()
        else {
            return Ok(());
        };
        match want {
            FileView::Partial => {
                let lines = count_lines(current);
                Err(FileViewsError::NotFullySeen(format!(
                    "{ERR_FILE_NOT_FULLY_SEEN}: write_file replaces all {lines} lines of {path}; read it whole first (read_file with offset 0 and limit >= {lines}), or use edit_file for a targeted change"
                )))
            }
            FileView::Complete { hash } if hash != Self::hash(current) => {
                Err(FileViewsError::ChangedSinceSeen(format!(
                    "{ERR_FILE_CHANGED_SINCE_SEEN}: {path} was modified after you last read or wrote it, by the user or another process; read it again and apply your change to what it holds now"
                )))
            }
            FileView::Complete { .. } => Ok(()),
        }
    }
}

/// Go `bytes.Count(data, '\n')` plus the trailing unterminated-line rule.
fn count_lines(content: &[u8]) -> usize {
    let mut lines = content.iter().filter(|&&b| b == b'\n').count();
    if !content.is_empty() && content[content.len() - 1] != b'\n' {
        lines += 1;
    }
    lines
}

impl FileViewsHook for FileViews {
    fn saw(&self, path: &str, content: &[u8]) {
        self.saw(path, content);
    }

    fn check_overwrite(
        &self,
        path: &str,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.check_overwrite(path, content)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_never_seen_file_is_not_refused() {
        let views = FileViews::new();
        assert!(views.check_overwrite("a.txt", b"new").is_ok());
    }

    #[test]
    fn a_partial_read_blocks_a_whole_file_overwrite() {
        let views = FileViews::new();
        views.saw_partial("a.txt");
        let err = views
            .check_overwrite("a.txt", b"line one\nline two\n")
            .unwrap_err();
        assert!(err.to_string().contains(ERR_FILE_NOT_FULLY_SEEN));
        assert!(err.to_string().contains("2 lines of a.txt"));
    }

    #[test]
    fn a_matching_hash_passes_and_a_drifted_file_is_refused() {
        let views = FileViews::new();
        views.saw("a.txt", b"stable");
        assert!(views.check_overwrite("a.txt", b"stable").is_ok());
        let err = views.check_overwrite("a.txt", b"changed").unwrap_err();
        assert!(err.to_string().contains(ERR_FILE_CHANGED_SINCE_SEEN));
        assert!(
            err.to_string()
                .contains("modified after you last read or wrote it")
        );
    }

    #[test]
    fn only_a_complete_read_certifies_content() {
        let views = FileViews::new();
        views.record_read("paged.txt", b"chunk", 128, true, 10);
        assert!(
            views
                .check_overwrite("paged.txt", b"other")
                .unwrap_err()
                .to_string()
                .contains(ERR_FILE_NOT_FULLY_SEEN)
        );

        views.record_read("whole.txt", b"one\ntwo\n", 0, true, 10);
        assert!(views.check_overwrite("whole.txt", b"one\ntwo\n").is_ok());

        // Oversize content must not certify the observation.
        let big = vec![b'x'; MAX_VIEW_BYTES + 1];
        views.record_read("big.txt", &big, 0, true, 100000);
        assert!(views.check_overwrite("big.txt", b"y").is_err());
    }

    #[test]
    fn a_partial_read_does_not_downgrade_an_earlier_complete_observation() {
        let views = FileViews::new();
        views.saw("a.txt", b"full");
        views.saw_partial("a.txt");
        assert!(views.check_overwrite("a.txt", b"full").is_ok());
    }
}
