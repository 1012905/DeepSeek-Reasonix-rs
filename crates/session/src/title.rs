//! Session title read side (Go serve `titlecache.go` + `titlefill.go`):
//! titles come from sidecars, never transcript decoding (http.md §7).
//!
//! The generated-title cache is a per-directory `.session-titles.json`
//! written by the title model machine. This build reads it but never
//! generates: a miss falls back to the preview, exactly as a Go tree read
//! does while background generation is still filling the cache. A listing
//! loads one snapshot per directory and titles every row from it — Go's
//! `titleCacheFor(dir).snapshot()` is likewise one read per directory.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use reasonix_state::sessionstore::preview::strip_paste_display_label;
use sha2::{Digest, Sha256};

/// The per-project title cache file (Go `titlecache.go`).
const TITLES_FILE: &str = ".session-titles.json";

/// One entry of the generated-title sidecar (Go `titleEntry`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct TitleEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    r#mod: i64,
    #[serde(default, rename = "source_hash")]
    source_hash: String,
    #[serde(default)]
    explicit: bool,
}

/// One directory's title sidecar, loaded once per listing (Go
/// `titleSnapshot`).
#[derive(Debug, Clone, Default)]
pub struct TitleSnapshot {
    entries: HashMap<String, TitleEntry>,
}

impl TitleSnapshot {
    /// Go `titleSnapshot.display`: a title the user typed outranks a
    /// generated one; a generated one only answers while its source still
    /// matches what it was generated from.
    pub fn display(&self, name: &str, source: &str, modified_nanos: i64, custom: &str) -> String {
        let custom = custom.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        let entry = match self.entries.get(name) {
            Some(entry) => entry,
            None => return String::new(),
        };
        if entry.explicit {
            return entry.title.clone();
        }
        if !entry.source_hash.is_empty() {
            if entry.source_hash == source_hash(source) {
                return entry.title.clone();
            }
        } else if entry.r#mod == modified_nanos {
            return entry.title.clone();
        }
        String::new()
    }
}

/// Read handle for one project directory's title cache (Go `titleCache`).
#[derive(Debug, Clone)]
pub struct TitleCache {
    dir: PathBuf,
}

impl TitleCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Go `titleCache.snapshot`: reload the sidecar. A missing or
    /// unparsable file reads as an empty cache — a damaged sidecar must
    /// not blank a sidebar that has previews to show.
    pub fn snapshot(&self) -> TitleSnapshot {
        TitleSnapshot {
            entries: self.load(),
        }
    }

    fn load(&self) -> HashMap<String, TitleEntry> {
        let Ok(data) = fs::read(self.dir.join(TITLES_FILE)) else {
            return HashMap::new();
        };
        // Go reads through ReadFileUTF8 and answers an empty map on any
        // decode failure.
        let text = String::from_utf8_lossy(&data);
        serde_json::from_str::<HashMap<String, TitleEntry>>(&text).unwrap_or_default()
    }

    /// One-shot display for callers titling a single row.
    pub fn display(&self, name: &str, source: &str, modified_nanos: i64, custom: &str) -> String {
        self.snapshot()
            .display(name, source, modified_nanos, custom)
    }
}

/// The title a row shows: cached/custom title when one answers, else the
/// preview stands in (Go `sessionTitleFrom` minus the background
/// generation, which is the title-model machine).
pub fn session_title(
    titles: &TitleSnapshot,
    name: &str,
    preview: &str,
    modified_nanos: i64,
    custom: &str,
) -> String {
    let source = title_source(preview);
    let cached = titles.display(name, &source, modified_nanos, custom);
    if !cached.is_empty() {
        return cached;
    }
    preview_title(&source)
}

/// Go `titleSource`: the first message, minus paste display chrome.
pub fn title_source(first: &str) -> String {
    strip_paste_display_label(first).trim().to_string()
}

/// Go `previewTitle`: the first 50 runes of the source, cut at 47 with an
/// ellipsis.
pub fn preview_title(source: &str) -> String {
    let runes: Vec<char> = source.chars().collect();
    if runes.len() > 50 {
        let head: String = runes[..47].iter().collect();
        return format!("{head}...");
    }
    source.to_string()
}

/// Go `titleSourceHash`: what ties a cached title to the message it was
/// generated from.
pub fn source_hash(source: &str) -> String {
    let sum = Sha256::digest(source.as_bytes());
    let mut hex = String::with_capacity(64);
    for b in sum {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn custom_title_outranks_everything() {
        let tmp = crate::test_support::tempdir();
        let cache = TitleCache::new(tmp.path());
        assert_eq!(cache.display("s.jsonl", "src", 5, "  mine  "), "mine");
    }

    #[test]
    fn explicit_entry_outranks_source_match() {
        let tmp = crate::test_support::tempdir();
        fs::write(
            tmp.path().join(TITLES_FILE),
            r#"{"s.jsonl":{"title":"kept","explicit":true,"source_hash":"deadbeef"}}"#,
        )
        .unwrap();
        let cache = TitleCache::new(tmp.path());
        assert_eq!(cache.display("s.jsonl", "src", 0, ""), "kept");
    }

    #[test]
    fn generated_title_answers_only_while_the_source_matches() {
        let tmp = crate::test_support::tempdir();
        let hash = source_hash("find me");
        fs::write(
            tmp.path().join(TITLES_FILE),
            format!(r#"{{"s.jsonl":{{"title":"Find me","source_hash":"{hash}"}}}}"#),
        )
        .unwrap();
        let cache = TitleCache::new(tmp.path());
        assert_eq!(cache.display("s.jsonl", "find me", 0, ""), "Find me");
        assert_eq!(cache.display("s.jsonl", "other", 0, ""), "");
    }

    #[test]
    fn mod_match_answers_when_no_hash_was_recorded() {
        let tmp = crate::test_support::tempdir();
        fs::write(
            tmp.path().join(TITLES_FILE),
            r#"{"s.jsonl":{"title":"Old","mod":42}}"#,
        )
        .unwrap();
        let cache = TitleCache::new(tmp.path());
        assert_eq!(cache.display("s.jsonl", "src", 42, ""), "Old");
        assert_eq!(cache.display("s.jsonl", "src", 43, ""), "");
    }

    #[test]
    fn damaged_sidecar_reads_as_empty() {
        let tmp = crate::test_support::tempdir();
        fs::write(tmp.path().join(TITLES_FILE), "{broken").unwrap();
        let cache = TitleCache::new(tmp.path());
        assert_eq!(cache.display("s.jsonl", "src", 0, ""), "");
    }

    #[test]
    fn preview_title_cuts_at_forty_seven_runes() {
        assert_eq!(preview_title("short"), "short");
        let long = "字".repeat(60);
        assert_eq!(preview_title(&long), format!("{}...", "字".repeat(47)));
    }
}
