//! TOOL-047 jobs wait/watch/echo/peek/views/progress decision tables.
//!
//! Covers rune-bounded trimming, peek tail slices and the `Truncated` rule,
//! the bash+Done exit backfill, watch carry/seed/fire semantics, the echo
//! whole-line/named-prefix/64 KiB cap behavior, snapshot deltas and target
//! resolution. The merged wait channel, timer and echo pump goroutine are
//! WIP; every cutoff and split rule here runs against in-memory buffers.

use std::sync::{Arc, Mutex};

use reasonix_tools::builtin::jobs::*;
use regex::Regex;

// ---------------------------------------------------------------------------
// View text trimming
// ---------------------------------------------------------------------------

#[test]
fn bounded_trims_and_appends_an_ellipsis_only_over_the_rune_limit() {
    assert_eq!(bounded("  hello  ", 200), "hello");
    let exact = "ä".repeat(200);
    assert_eq!(bounded(&exact, 200), exact, "exactly 200 runes stay intact");
    let over = "ä".repeat(201);
    let out = bounded(&over, 200);
    assert_eq!(out.chars().count(), 201);
    assert!(out.ends_with('…'));
}

#[test]
fn last_line_picks_the_final_non_empty_line_and_caps_at_200_runes() {
    assert_eq!(last_line(b"first\nsecond\nthird line\n"), "third line");
    assert_eq!(last_line(b"first\n\n  \n"), "first");
    let long = "x".repeat(250);
    let out = last_line(long.as_bytes());
    assert_eq!(out.chars().count(), 201);
}

#[test]
fn one_line_collapses_whitespace_and_cuts_with_three_dots() {
    assert_eq!(one_line("a  b\t c", 80), "a b c");
    assert_eq!(one_line("abcdefghij", 4), "abcd...");
}

// ---------------------------------------------------------------------------
// Peek
// ---------------------------------------------------------------------------

#[test]
fn peek_takes_the_tail_and_reports_truncation_by_byte_length() {
    let tail = b"0123456789";
    let peek = peek_from_tail(tail, 4, Status::Running);
    assert_eq!(peek.text, "6789");
    assert_eq!(peek.bytes, 10);
    assert!(peek.truncated);

    let almost = peek_from_tail(tail, 9, Status::Running);
    assert!(almost.truncated, "one byte short is still truncated");
    let full = peek_from_tail(tail, 20, Status::Done);
    assert_eq!(full.text, "0123456789");
    assert!(!full.truncated);

    let empty = peek_from_tail(tail, 0, Status::Running);
    assert!(empty.text.is_empty());
    assert_eq!(empty.bytes, 10);
}

#[test]
fn peek_skips_split_continuation_bytes_at_the_head() {
    let raw = [0xB8, 0xAD, b'x'];
    assert_eq!(first_rune_start(&raw), 2);
}

// ---------------------------------------------------------------------------
// Exit code backfill
// ---------------------------------------------------------------------------

#[test]
fn exit_code_backfills_zero_only_for_bash_done_without_a_recorded_code() {
    assert_eq!(backfill_exit_code("bash", Status::Done, None), Some(0));
    assert_eq!(backfill_exit_code("bash", Status::Failed, None), None);
    assert_eq!(backfill_exit_code("task", Status::Done, None), None);
    assert_eq!(backfill_exit_code("bash", Status::Done, Some(3)), Some(3));
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

#[test]
fn byte_count_picks_buckets_at_the_thresholds() {
    assert_eq!(byte_count(1023), "1023 B");
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

// ---------------------------------------------------------------------------
// Watch
// ---------------------------------------------------------------------------

#[test]
fn wait_outcome_maps_the_four_states() {
    assert_eq!(WaitOutcome::Finished.as_str(), "finished");
    assert_eq!(WaitOutcome::Matched.as_str(), "matched");
    assert_eq!(WaitOutcome::TimedOut.as_str(), "timeout");
    assert_eq!(WaitOutcome::Cancelled.as_str(), "cancelled");
    assert_eq!(
        WaitOutcome::from_str("timeout"),
        Some(WaitOutcome::TimedOut)
    );
    assert!(WaitOutcome::from_str("nope").is_none());
}

#[test]
fn a_match_already_in_the_tail_fires_on_registration() {
    let re = Regex::new("needle").unwrap();
    let (_watch, fired) = OutputWatch::new(Some(re), None, b"the needle here");
    assert!(fired);
}

#[test]
fn carry_bridges_a_match_split_across_two_writes() {
    let re = Regex::new("needle").unwrap();
    let (watch, fired) = OutputWatch::new(Some(re), None, b"");
    assert!(!fired);
    assert!(!watch.notify(b"nee"));
    assert!(watch.notify(b"dle"), "split match crosses the carry");
    assert!(watch.fired());
}

#[test]
fn a_fired_watch_ignores_later_writes() {
    let re = Regex::new("ab").unwrap();
    let (watch, _) = OutputWatch::new(Some(re), None, b"");
    assert!(watch.notify(b"ab"));
    assert!(!watch.notify(b"ab"), "one match per watch");
}

#[test]
fn echo_only_watches_stream_bytes_and_never_hit() {
    let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_clone = seen.clone();
    let echo: ByteSink = Arc::new(move |p| {
        seen_clone.lock().expect("seen lock").push(p.to_vec());
    });
    let (watch, fired) = OutputWatch::new(None, Some(echo), b"seed");
    assert!(!fired);
    assert!(!watch.notify(b"hello"));
    assert_eq!(seen.lock().expect("seen lock").len(), 1);
}

// ---------------------------------------------------------------------------
// Snapshot deltas
// ---------------------------------------------------------------------------

#[test]
fn delta_clamps_negative_differences_to_zero() {
    assert_eq!(delta(120, 90), 30);
    assert_eq!(delta(90, 120), 0);
    assert_eq!(delta(120, 120), 0);
}

#[test]
fn resolve_lists_running_in_session_or_matches_named_ids() {
    let jobs = vec![
        ResolveRef {
            id: "a".into(),
            session: "s1".into(),
            running: true,
        },
        ResolveRef {
            id: "b".into(),
            session: "s1".into(),
            running: false,
        },
        ResolveRef {
            id: "c".into(),
            session: "s2".into(),
            running: true,
        },
        ResolveRef {
            id: "d".into(),
            session: String::new(),
            running: true,
        },
    ];
    let all_s1 = resolve(&jobs, "s1", &[]);
    assert_eq!(
        all_s1.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "d"]
    );
    let named = resolve(&jobs, "s1", &["b".into()]);
    assert_eq!(named.len(), 1);
    let unscoped = resolve(&jobs, "", &["d".into()]);
    assert_eq!(unscoped.len(), 1);
    let unscoped_all = resolve(&jobs, "", &[]);
    assert_eq!(unscoped_all.len(), 3);
}

// ---------------------------------------------------------------------------
// Echo aggregator
// ---------------------------------------------------------------------------

type EchoEmit = Arc<dyn Fn(String) + Send + Sync>;
type ByteSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

fn recording() -> (Arc<Mutex<Vec<String>>>, EchoEmit) {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emit: EchoEmit = {
        let seen = seen.clone();
        Arc::new(move |s| seen.lock().expect("seen lock").push(s))
    };
    (seen, emit)
}

#[test]
fn whole_lines_are_queued_and_flushed_without_a_prefix_for_one_job() {
    let (seen, emit) = recording();
    let echo = Echo::new(emit, false);
    echo.take("bash-1", b"hello\nworld\n");
    echo.flush();
    assert_eq!(seen.lock().expect("seen lock").join(""), "hello\nworld\n");
}

#[test]
fn named_mode_prefixes_every_line_with_the_job_id() {
    let (seen, emit) = recording();
    let echo = Echo::new(emit, true);
    echo.take("bash-1", b"one\n");
    echo.take("bash-2", b"two\n");
    echo.finish(&["bash-1".into(), "bash-2".into()]);
    assert_eq!(
        seen.lock().expect("seen lock").join(""),
        "[bash-1] one\n[bash-2] two\n"
    );
}

#[test]
fn a_partial_line_waits_until_finish() {
    let (seen, emit) = recording();
    let echo = Echo::new(emit, false);
    echo.take("bash-1", b"partial");
    echo.flush();
    assert!(seen.lock().expect("seen lock").is_empty());
    echo.finish(&["bash-1".into()]);
    assert_eq!(seen.lock().expect("seen lock").join(""), "partial\n");
}

#[test]
fn the_64kib_cap_marks_once_and_stops_enqueueing() {
    let (seen, emit) = recording();
    let echo = Echo::new(emit, false);
    let big = vec![b'a'; ECHO_LIMIT + 10];
    echo.take("bash-1", &big);
    echo.finish(&["bash-1".into()]);
    let out = seen.lock().expect("seen lock").join("");
    assert!(out.contains(ECHO_CAPPED));
    assert!(echo.is_capped());
    assert!(out.len() <= ECHO_LIMIT + ECHO_CAPPED.len());
}

#[test]
fn open_lines_over_the_block_limit_are_sliced_at_a_rune_boundary() {
    let (seen, emit) = recording();
    let echo = Echo::new(emit, false);
    let chunk = vec![b'x'; ECHO_OPEN_LIMIT + 1];
    echo.take("bash-1", &chunk);
    echo.flush();
    let out = seen.lock().expect("seen lock").join("");
    assert!(out.ends_with('\n'));
    assert!(!out.is_empty());
}

#[test]
fn seed_from_tail_skips_a_split_line_and_leading_continuations() {
    let seed = Echo::seed_from_tail(b"first line\nsecond line tail");
    assert_eq!(seed, b"second line tail");
    let mid = b"\xe4\xb8\xad rest";
    let seed = Echo::seed_from_tail(mid);
    assert_eq!(seed, b"\xe4\xb8\xad rest");
}
