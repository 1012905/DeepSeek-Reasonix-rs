//! TOOL-047 live echo aggregator (Go `internal/tools/jobs/echo.go`).
//!
//! Done here: whole-line splitting, per-job held partial lines, the named
//! prefix, the 4 KiB seed and open-line block, the 64 KiB cap with its marker,
//! and the trailing partial-rune trim. The echo pump goroutine, wake channel
//! and stop handshake are runtime WIP; the aggregation itself runs against an
//! injectable emit closure.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::wait::last_bytes;

/// `echoLimit`: per-wait live echo ceiling; the wait result still carries the
/// full output (`echo.go:12`).
pub const ECHO_LIMIT: usize = 64 << 10;
/// String echo closure handed to the aggregator.
type EchoEmit = Arc<dyn Fn(String) + Send + Sync>;

/// Marker appended once the ceiling is crossed.
pub const ECHO_CAPPED: &str =
    "\n...[live output capped at 64 KiB; the result shows the rest when the wait ends]...\n";
/// `echoSeed`: tail of already-written output shown when the wait starts.
pub const ECHO_SEED: usize = 4 << 10;
/// `echoOpenLimit`: retained unfinished-line block size; progress bars and
/// binary streams are sliced in this chunk.
pub const ECHO_OPEN_LIMIT: usize = 4 << 10;

/// Whether a byte is a UTF-8 continuation byte (not a rune lead).
fn is_continuation(b: u8) -> bool {
    (b & 0xC0) == 0x80
}

/// `completeRunes`: drop only a trailing half-encoded multi-byte character;
/// illegal bytes in the middle pass through unchanged (`echo.go:118-128`).
pub fn complete_runes(buf: &mut Vec<u8>) {
    while !buf.is_empty() && std::str::from_utf8(buf).is_err() {
        buf.pop();
    }
}

/// Split complete trailing-newline lines off the buffer; the partial last
/// line stays with the caller.
fn split_complete_lines(buf: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let Some(last_nl) = buf.iter().rposition(|b| *b == b'\n') else {
        return (Vec::new(), buf.to_vec());
    };
    let ready: Vec<Vec<u8>> = buf[..=last_nl]
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut out = line.to_vec();
            out.push(b'\n');
            out
        })
        .collect();
    (ready, buf[last_nl + 1..].to_vec())
}

/// The in-memory echo aggregator for one wait across one or many jobs.
pub struct Echo {
    emit: EchoEmit,
    named: bool,
    queued: Mutex<Vec<u8>>,
    open: Mutex<HashMap<String, Vec<u8>>>,
    sent: Mutex<usize>,
    capped: Mutex<bool>,
}

impl Echo {
    pub fn new(emit: Arc<dyn Fn(String) + Send + Sync>, named: bool) -> Arc<Self> {
        Arc::new(Self {
            emit,
            named,
            queued: Mutex::new(Vec::new()),
            open: Mutex::new(HashMap::new()),
            sent: Mutex::new(0),
            capped: Mutex::new(false),
        })
    }

    /// Whether the ceiling has been crossed.
    pub fn is_capped(&self) -> bool {
        *self.capped.lock().expect("echo capped lock")
    }

    /// `take`: route one write from a job into the held partial line and the
    /// ready line queue. Called under the job lock; the emit itself is not.
    pub fn take(&self, id: &str, p: &[u8]) {
        let mut open = self.open.lock().expect("echo open lock");
        let buf = open.entry(id.to_owned()).or_default();
        buf.extend_from_slice(p);

        let mut ready: Vec<Vec<u8>> = Vec::new();
        if p.contains(&b'\n') {
            let (lines, rest) = split_complete_lines(buf);
            ready = lines;
            *buf = rest;
        } else if buf.len() > ECHO_OPEN_LIMIT {
            let mut block = buf.clone();
            complete_runes(&mut block);
            block.push(b'\n');
            ready.push(block);
            buf.drain(..buf.len().min(ECHO_OPEN_LIMIT));
        }
        drop(open);

        if !ready.is_empty() {
            self.queue(id, ready);
        }
    }

    fn queue(&self, id: &str, lines: Vec<Vec<u8>>) {
        let mut queued = self.queued.lock().expect("echo queued lock");
        let mut sent = self.sent.lock().expect("echo sent lock");
        let mut capped = self.capped.lock().expect("echo capped lock");
        if *capped || lines.is_empty() {
            return;
        }
        for mut line in lines {
            if self.named {
                let prefix = format!("[{id}] ");
                let mut prefixed = Vec::with_capacity(prefix.len() + line.len());
                prefixed.extend_from_slice(prefix.as_bytes());
                prefixed.append(&mut line);
                line = prefixed;
            }
            let space = ECHO_LIMIT.saturating_sub(*sent);
            if line.len() > space {
                // Cut back to the last newline so the marker lands on its own
                // line; after the cap, nothing else is enqueued.
                if let Some(nl) = queued.iter().rposition(|b| *b == b'\n') {
                    queued.truncate(nl + 1);
                }
                queued.extend_from_slice(ECHO_CAPPED.as_bytes());
                *sent = ECHO_LIMIT;
                *capped = true;
                return;
            }
            queued.append(&mut line);
            *sent += line.len();
        }
    }

    /// `flush`: hand the queued bytes to the emit closure outside any lock.
    pub fn flush(&self) {
        let pending = {
            let mut queued = self.queued.lock().expect("echo queued lock");
            queued.drain(..).collect::<Vec<u8>>()
        };
        if !pending.is_empty() {
            (self.emit)(String::from_utf8_lossy(&pending).into_owned());
        }
    }

    /// `finish`: release every held residual line to its last complete
    /// character, then flush (`echo.go:103-114`).
    pub fn finish(&self, order: &[String]) {
        let mut residual: Vec<(String, Vec<u8>)> = Vec::new();
        {
            let mut open = self.open.lock().expect("echo open lock");
            for id in order {
                if let Some(buf) = open.remove(id) {
                    residual.push((id.clone(), buf));
                }
            }
            for (id, buf) in open.drain() {
                residual.push((id, buf));
            }
        }
        for (id, mut buf) in residual {
            if buf.is_empty() {
                continue;
            }
            complete_runes(&mut buf);
            buf.push(b'\n');
            self.queue(&id, vec![buf]);
        }
        self.flush();
    }

    /// Seed the held partial line from the existing tail: the final
    /// [`ECHO_SEED`] bytes, starting after the first newline when the seed
    /// splits a line, skipping leading continuation bytes.
    pub fn seed_from_tail(tail: &[u8]) -> Vec<u8> {
        let mut seed = last_bytes(tail, ECHO_SEED);
        if let Some(nl) = seed.iter().position(|b| *b == b'\n') {
            seed = seed[nl + 1..].to_vec();
        }
        while let Some(&first) = seed.first() {
            if is_continuation(first) {
                seed.remove(0);
            } else {
                break;
            }
        }
        seed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn recording() -> (Arc<StdMutex<Vec<String>>>, EchoEmit) {
        let seen: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
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
    fn a_partial_line_waits_for_finish() {
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
        // 4 KiB+1 of a multibyte-free stream forces the open-limit slice.
        let chunk = vec![b'x'; ECHO_OPEN_LIMIT + 1];
        echo.take("bash-1", &chunk);
        echo.flush();
        let out = seen.lock().expect("seen lock").join("");
        assert!(out.ends_with('\n'));
        assert!(!out.is_empty());
    }

    #[test]
    fn complete_runes_drops_only_a_trailing_half_character() {
        let mut buf = b"hello \xe4\xb8".to_vec();
        complete_runes(&mut buf);
        assert_eq!(buf, b"hello ");
        let mut intact = b"hello \xe4\xb8\xad".to_vec();
        complete_runes(&mut intact);
        assert_eq!(intact, b"hello \xe4\xb8\xad");
    }

    #[test]
    fn seed_takes_the_tail_and_skips_a_split_line_and_continuations() {
        let tail = b"first line\nsecond line tail";
        let seed = Echo::seed_from_tail(tail);
        assert_eq!(seed, b"second line tail");
        let mid = b"\xe4\xb8\xad rest";
        let seed = Echo::seed_from_tail(mid);
        assert_eq!(seed, b"\xe4\xb8\xad rest");
    }
}
