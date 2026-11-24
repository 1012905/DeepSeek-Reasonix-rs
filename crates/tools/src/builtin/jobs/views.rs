//! TOOL-047 view snapshots, peek tail reads and the wrapper render helpers
//! (Go `internal/tools/jobs/jobs_view.go`, `peek.go`, `progress.go`,
//! `builtin/bgjobs.go` §343-370).
//!
//! Done here: the `View`/`Peek` shapes, rune-bounded description/detail/tail
//! trimming, the peek tail slice and `Truncated` rule, the bash+Done exit
//! backfill, and the `stillRunning`/`byteCount`/`oneLine` render functions.
//! File-end seek reads and the live panel refresh are host WIP.

#![forbid(unsafe_code)]

use super::Status;
use super::wait::last_bytes;

/// `MaxDescriptionRunes` (`jobs_view.go:108`).
pub const MAX_DESCRIPTION_RUNES: usize = 200;
/// `MaxDetailRunes` (`jobs_view.go:109`).
pub const MAX_DETAIL_RUNES: usize = 4096;
/// `maxTailRunes` (`jobs_view.go:84`).
pub const MAX_TAIL_RUNES: usize = 200;

/// Presentation attached once at start (`Presentation`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presentation {
    pub description: String,
    pub detail: String,
}

/// One read-only running or ended job snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub description: String,
    pub detail: String,
    pub status: Status,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub tail: String,
}

/// A peek at the current tail; it does not advance the reader offset
/// (`peek.go:11-16`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Peek {
    pub text: String,
    pub status: Status,
    /// Total bytes produced so far.
    pub bytes: i64,
    /// Whether `text` is only a trailing fragment.
    pub truncated: bool,
}

fn is_continuation(b: u8) -> bool {
    (b & 0xC0) == 0x80
}

/// `max0`: clamp a possibly-negative byte count to zero (`peek.go:42`).
pub fn max0(n: i64) -> i64 {
    n.max(0)
}

/// `firstRuneStart`: skip continuation bytes preceding a rune lead inside
/// the first UTF-8 max bytes (`peek.go:65-72`).
pub fn first_rune_start(b: &[u8]) -> usize {
    let mut i = 0;
    while i < b.len() && i < 3 && is_continuation(b[i]) {
        i += 1;
    }
    i
}

/// `bounded`: trim, then cut to `limit` runes with an ellipsis when trimmed
/// (`jobs_view.go:113-119`).
pub fn bounded(s: &str, limit: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(limit).collect();
    out.push('…');
    out
}

/// `lastLine`: last non-empty line, trimmed, bounded to [`MAX_TAIL_RUNES`]
/// runes (`jobs_view.go:87-94`).
pub fn last_line(tail: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(tail);
    let trimmed = lossy.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\r' || c == '\n');
    let last = trimmed.rsplit('\n').next().unwrap_or("").trim();
    bounded(last, MAX_TAIL_RUNES)
}

/// Peek assembly over an in-memory tail. The file-end branch mirrors this
/// slice-for-slice once artifact IO lands (`peek.go:22-40`).
pub fn peek_from_tail(tail: &[u8], max: i64, status: Status) -> Peek {
    let max = max0(max) as usize;
    let bytes = tail.len() as i64;
    let raw = if max == 0 {
        Vec::new()
    } else {
        last_bytes(tail, max)
    };
    let truncated = (raw.len() as i64) < bytes;
    let start = first_rune_start(&raw);
    let text = String::from_utf8_lossy(&raw[start..]).into_owned();
    Peek {
        text,
        status,
        bytes,
        truncated,
    }
}

/// Bash jobs that finished cleanly without a recorded exit code read back as
/// exit 0; task jobs carry no exit code (`jobs_view.go:74-77`).
pub fn backfill_exit_code(kind: &str, status: Status, exit: Option<i32>) -> Option<i32> {
    match exit {
        Some(code) => Some(code),
        None if kind == "bash" && status == Status::Done => Some(0),
        None => None,
    }
}

/// `byteCount`: human byte size the way the wait/list rendering words it
/// (`bgjobs.go:359-370`).
pub fn byte_count(n: i64) -> String {
    let n = n.max(0);
    const MB: f64 = (1 << 20) as f64;
    const KB: f64 = (1 << 10) as f64;
    let f = n as f64;
    if f >= MB - 51.0 {
        format!("{:.1} MB", f / MB)
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{n} B")
    }
}

/// `stillRunning`: the resume sentence appended to a running job's wait line
/// (`bgjobs.go:343-357`).
pub fn still_running(
    running_ms: i64,
    produced: i64,
    delta: i64,
    steps: i64,
    step_delta: i64,
) -> String {
    let secs = (running_ms.max(0) + 500) / 1000;
    let mut parts = vec![format!("running {secs}s")];
    if produced > 0 {
        parts.push(format!(
            "{} of output, {} of it while waiting",
            byte_count(produced),
            byte_count(delta)
        ));
    } else if steps > 0 {
        parts.push(format!(
            "no output text (it does not write here), {steps} step(s) reported, {step_delta} while waiting"
        ));
    } else {
        parts.push("no output text and no steps reported yet".to_owned());
    }
    format!(" — {}", parts.join(", "))
}

/// `oneLine`: collapse whitespace and cut to `limit` runes with an ellipsis
/// (`bgjobs.go:141-147`).
pub fn one_line(s: &str, limit: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(limit).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_trims_and_appends_an_ellipsis_at_the_rune_limit() {
        assert_eq!(bounded("  hello  ", 200), "hello");
        let long = "ä".repeat(250);
        let out = bounded(&long, 200);
        assert_eq!(out.chars().count(), 201);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn last_line_picks_the_final_non_empty_line_bounded_to_200_runes() {
        let tail = b"first\nsecond\nthird line\n";
        assert_eq!(last_line(tail), "third line");
        let blank = b"first\n\n  \n";
        assert_eq!(last_line(blank), "first");
        let long = "x".repeat(250);
        let out = last_line(long.as_bytes());
        assert_eq!(out.chars().count(), 201);
    }

    #[test]
    fn peek_takes_the_tail_and_reports_truncation_by_length() {
        let tail = b"0123456789";
        let peek = peek_from_tail(tail, 4, Status::Running);
        assert_eq!(peek.text, "6789");
        assert_eq!(peek.bytes, 10);
        assert!(peek.truncated);
        let full = peek_from_tail(tail, 20, Status::Done);
        assert_eq!(full.text, "0123456789");
        assert!(!full.truncated);
    }

    #[test]
    fn peek_skips_a_split_continuation_byte_at_the_head() {
        // A three-byte rune cut after its lead byte leaves two continuation
        // bytes ahead of the following lead.
        let raw = [0xB8, 0xAD, b'x'];
        let start = first_rune_start(&raw);
        assert_eq!(start, 2);
    }

    #[test]
    fn exit_code_backfills_zero_only_for_bash_done() {
        assert_eq!(backfill_exit_code("bash", Status::Done, None), Some(0));
        assert_eq!(backfill_exit_code("bash", Status::Failed, None), None);
        assert_eq!(backfill_exit_code("task", Status::Done, None), None);
        assert_eq!(backfill_exit_code("bash", Status::Done, Some(3)), Some(3));
    }

    #[test]
    fn byte_count_picks_buckets_at_the_thresholds() {
        assert_eq!(byte_count(999), "999 B");
        assert_eq!(byte_count(1024), "1.0 KB");
        assert_eq!(byte_count(1 << 20), "1.0 MB");
        assert_eq!(byte_count(2_560_000), "2.4 MB");
    }

    #[test]
    fn still_running_words_output_then_steps_then_neither() {
        assert_eq!(
            still_running(5_000, 120, 30, 0, 0),
            " — running 5s, 120 B of output, 30 B of it while waiting"
        );
        assert_eq!(
            still_running(5_000, 0, 0, 4, 1),
            " — running 5s, no output text (it does not write here), 4 step(s) reported, 1 while waiting"
        );
        assert_eq!(
            still_running(5_000, 0, 0, 0, 0),
            " — running 5s, no output text and no steps reported yet"
        );
    }

    #[test]
    fn one_line_collapses_whitespace_and_cuts() {
        assert_eq!(one_line("a  b\t c", 80), "a b c");
        let out = one_line("abcdefghij", 4);
        assert_eq!(out, "abcd...");
    }
}
