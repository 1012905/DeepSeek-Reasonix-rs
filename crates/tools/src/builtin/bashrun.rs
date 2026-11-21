//! The shared foreground shell runner behind the model bash tool (parity
//! target: `internal/tools/shellrun/runner.go`). It classifies exits,
//! collects a bounded output tail, and keeps combined stdout/stderr
//! model-visible output intact. Process-tree tracking is the OS-sandbox
//! wave's machine (Go `internal/base/proc`): the timeout and cancellation
//! kill the direct child here, and the wait-delay bound keeps a lingering
//! grandchild holding the pipe from wedging the call.

#![forbid(unsafe_code)]

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use reasonix_core::fileencoding;
use reasonix_safety::sandbox::shell::hide_window;

use crate::shell::{OUTPUT_TAIL_MAX_BYTES, ShellState};

/// Mirrors the bash tool's child-process wait grace.
pub const DEFAULT_WAIT_DELAY: Duration = Duration::from_secs(5);

/// Bounds the foreground output retained in memory. Tool-result truncation
/// happens only after the process exits, so it cannot protect the host from
/// a command that prints forever (#6473, #6528).
pub const COMBINED_OUTPUT_MAX_BYTES: usize = 10 << 20;
/// Keep the final diagnostics as well as the command's opening context
/// after the cap is crossed. Build and test failures are commonly printed
/// last.
pub const COMBINED_OUTPUT_TAIL_BYTES: usize = 64 << 10;
pub const COMBINED_OUTPUT_TRUNCATED: &str =
    "\n\n...[shell output truncated at 10 MiB; showing the final 64 KiB]...\n\n";

/// How often the waiter re-checks exit, deadline and cancellation.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A coarser wait when the parent has no deadline of its own: the poll
/// loop still answers cancellation and a child exit promptly.
const NO_DEADLINE_CHECK_INTERVAL: Duration = Duration::from_millis(20);

/// One foreground shell launch. `argv` must already include the interpreter
/// and any sandbox wrapping; `command` is only for diagnostics.
#[derive(Clone, Default)]
pub struct Request {
    pub argv: Vec<String>,
    pub dir: String,
    pub env: Vec<String>,
    pub timeout: Duration,
    pub wait_delay: Duration,
    /// Live combined-output chunks for a streaming sink; None drops them.
    pub progress: Option<ProgressSink>,
}

type ProgressSink = Arc<dyn Fn(&str) + Send + Sync>;

/// The structured outcome of a foreground run.
#[derive(Debug, Clone)]
pub struct Result {
    pub combined: String,
    /// The bounded tail of failed combined output.
    pub output_tail: String,
    pub exit_code: Option<i32>,
    pub started: bool,
    pub state: ShellState,
    pub timed_out: bool,
    /// Set when a grandchild held the pipes past the wait delay; the run's
    /// error is dropped the way Go's ErrWaitDelay reads.
    pub wait_delay_expired: bool,
    /// The deadline that fired, rendered the way Go durations read.
    pub timeout_text: String,
    pub err: Option<RunError>,
}

impl Default for Result {
    fn default() -> Self {
        Result {
            combined: String::new(),
            output_tail: String::new(),
            exit_code: None,
            started: false,
            state: ShellState::Failed,
            timed_out: false,
            wait_delay_expired: false,
            timeout_text: String::new(),
            err: None,
        }
    }
}

/// Why a run failed, kept typed so the tool can name whose deadline fired
/// and whether anything ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The tool-local deadline fired.
    Timeout { deadline_text: String, silent: bool },
    /// The parent cancelled the call.
    Cancelled,
    /// The child produced an exit status; text is Go's `exit status N`.
    Exit { code: i32 },
    /// Launch failed (interpreter not found, spawn error).
    Launch { message: String },
    /// The child ran and failed without an exit status.
    Execution { message: String },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Timeout {
                deadline_text,
                silent,
            } => {
                write!(f, "command timed out (> {deadline_text})")?;
                if *silent {
                    write!(f, "; it produced no output before the deadline")?;
                }
                Ok(())
            }
            RunError::Cancelled => f.write_str("context canceled"),
            RunError::Exit { code } => write!(f, "exit status {code}"),
            RunError::Launch { message } => f.write_str(message),
            RunError::Execution { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for RunError {}

/// Parent-cancellation the host attaches through the call context.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(pub Arc<AtomicBool>);

impl CancelToken {
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Starts the process, captures combined stdout/stderr, and classifies
/// timeout / cancel / launch / execution failures.
pub fn run_foreground(req: Request, cancel: Option<&CancelToken>) -> Result {
    if req.argv.is_empty() {
        return Result {
            state: ShellState::Failed,
            err: Some(RunError::Launch {
                message: String::from("empty argv"),
            }),
            ..Result::default()
        };
    }
    let wait_delay = if req.wait_delay.is_zero() {
        DEFAULT_WAIT_DELAY
    } else {
        req.wait_delay
    };
    let deadline = (!req.timeout.is_zero()).then(|| Instant::now() + req.timeout);

    let mut cmd = std::process::Command::new(&req.argv[0]);
    cmd.args(&req.argv[1..]);
    if !req.dir.is_empty() {
        cmd.current_dir(&req.dir);
    }
    if !req.env.is_empty() {
        cmd.env_clear();
        for kv in &req.env {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    hide_window(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return Result {
                state: ShellState::Failed,
                err: Some(RunError::Launch {
                    message: err.to_string(),
                }),
                ..Result::default()
            };
        }
    };
    let child = &mut child;

    let collector = Arc::new(Collector::new());
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(spawn_reader(
            Box::new(out),
            collector.clone(),
            req.progress.clone(),
        ));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(spawn_reader(
            Box::new(err),
            collector.clone(),
            req.progress.clone(),
        ));
    }

    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        if let Some(token) = cancel
            && token.cancelled()
        {
            cancelled = true;
            break;
        }
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            timed_out = true;
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                let interval = if deadline.is_some() {
                    POLL_INTERVAL
                } else {
                    NO_DEADLINE_CHECK_INTERVAL
                };
                std::thread::sleep(interval);
            }
            Err(err) => {
                return finish_nowait(
                    &collector,
                    Result {
                        state: ShellState::Failed,
                        err: Some(RunError::Execution {
                            message: err.to_string(),
                        }),
                        ..Result::default()
                    },
                );
            }
        }
    }

    if cancelled || timed_out {
        let _ = child.kill();
    }
    let status = child.wait().ok();

    // The wait delay bounds two wedges: a child that will not exit after
    // the kill, and grandchildren that keep the pipes open after the shell
    // leader is gone (#3702). Past the bound the readers are left to drain
    // on their own and what was collected stands.
    let drained = collector.wait_readers(readers, wait_delay);
    let wait_delay_expired = !drained;

    let combined = collector.combined_string();
    let tail = collector.tail_string();
    let silent = combined.trim().is_empty();
    let exit_code = status.as_ref().map(exit_code_of);

    let mut out = Result {
        combined,
        output_tail: String::new(),
        exit_code,
        started: true,
        state: ShellState::Failed,
        timed_out,
        wait_delay_expired,
        timeout_text: go_duration_text(req.timeout),
        err: None,
    };

    if timed_out {
        out.state = ShellState::TimedOut;
        out.err = Some(RunError::Timeout {
            deadline_text: out.timeout_text.clone(),
            silent,
        });
        out.output_tail = tail;
        return out;
    }
    if cancelled {
        out.state = ShellState::Cancelled;
        out.err = Some(RunError::Cancelled);
        out.output_tail = tail;
        return out;
    }
    if wait_delay_expired {
        // Go's ErrWaitDelay: the child ended but its pipes did not. The
        // caller decides whether a preserved daemon counts as success.
        out.err = Some(RunError::Execution {
            message: String::from("wait delay expired with output still pending"),
        });
        out.output_tail = tail;
        return out;
    }
    if let Some(code) = exit_code {
        if code == 0 {
            out.state = ShellState::Completed;
            // The tail exists to explain a failure; success drops it.
            return out;
        }
        out.state = ShellState::Failed;
        out.err = Some(RunError::Exit { code });
        out.output_tail = tail;
        return out;
    }
    out.state = ShellState::Failed;
    out.err = Some(RunError::Execution {
        message: String::from("the child produced no exit status"),
    });
    out.output_tail = tail;
    out
}

fn finish_nowait(collector: &Collector, mut out: Result) -> Result {
    out.combined = collector.combined_string();
    out.output_tail = collector.tail_string();
    out
}

fn exit_code_of(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

fn spawn_reader(
    pipe: Box<dyn std::io::Read + Send>,
    collector: Arc<Collector>,
    progress: Option<ProgressSink>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut pipe = pipe;
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => collector.write(&buf[..n], progress.as_deref()),
                Err(_) => break,
            }
        }
        collector.reader_done();
    })
}

/// Owns the combined buffer and the bounded tail. Writes stay serialized
/// behind one mutex so two pipe readers cannot race on the buffer.
struct Collector {
    inner: Mutex<CollectorState>,
    done: Condvar,
}

struct CollectorState {
    combined: BoundedBuffer,
    tail: TailWriter,
    /// Decoded live progress; forwarded once per complete line or ASCII run.
    pending: Vec<u8>,
    readers: usize,
}

impl Collector {
    fn new() -> Collector {
        Collector {
            inner: Mutex::new(CollectorState {
                combined: BoundedBuffer::new(
                    COMBINED_OUTPUT_MAX_BYTES,
                    COMBINED_OUTPUT_TAIL_BYTES,
                    COMBINED_OUTPUT_TRUNCATED,
                ),
                tail: TailWriter::new(OUTPUT_TAIL_MAX_BYTES),
                pending: Vec::new(),
                readers: 0,
            }),
            done: Condvar::new(),
        }
    }

    fn write(&self, p: &[u8], progress: Option<&(dyn Fn(&str) + Send + Sync)>) {
        if p.is_empty() {
            return;
        }
        let mut state = self.inner.lock().expect("collector");
        state.combined.write(p);
        state.tail.write(p);
        if let Some(emit) = progress {
            state.pending.extend_from_slice(p);
            forward_progress(&mut state.pending, emit);
        }
    }

    fn reader_done(&self) {
        let mut state = self.inner.lock().expect("collector");
        state.readers += 1;
        self.done.notify_all();
    }

    /// Waits until every reader ended or the grace expires. `false` means a
    /// grandchild is still holding a pipe.
    fn wait_readers(&self, readers: Vec<std::thread::JoinHandle<()>>, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        let mut state = self.inner.lock().expect("collector");
        while state.readers < readers.len() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _timeout) = self
                .done
                .wait_timeout(state, deadline - now)
                .expect("collector");
            state = guard;
        }
        for handle in readers {
            let _ = handle.join();
        }
        true
    }

    fn combined_string(&self) -> String {
        let state = self.inner.lock().expect("collector");
        decode_shell_output(&state.combined.bytes())
    }

    fn tail_string(&self) -> String {
        let state = self.inner.lock().expect("collector");
        decode_shell_output(&state.tail.buf)
    }
}

/// Live progress: whole lines first so a code-page character split across
/// pipe reads is not mistaken for invalid UTF-8; bare ASCII forwards
/// immediately so dots and prompts stay live.
fn forward_progress(pending: &mut Vec<u8>, emit: &(dyn Fn(&str) + Send + Sync)) {
    while let Some(end) = pending.iter().position(|&c| c == b'\n') {
        let line: &[u8] = &pending[..end + 1];
        emit(&decode_shell_output(line));
        pending.drain(..end + 1);
    }
    let safe = pending.iter().take_while(|&&c| c < 0x80).count();
    if safe > 0 {
        emit(&decode_shell_output(&pending[..safe]));
        pending.drain(..safe);
    }
}

/// Go `decodeShellOutput` over `fileenc.DecodeOutput`: valid UTF-8 stands
/// as-is; a partial trailing rune is trimmed when that alone was invalid;
/// anything else is sniffed and decoded (the console code page).
fn decode_shell_output(b: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(b) {
        return text.to_owned();
    }
    let trimmed = fileencoding::trim_partial_rune(b);
    if trimmed.len() < b.len()
        && let Ok(text) = std::str::from_utf8(trimmed)
        && text.chars().count() < trimmed.len()
    {
        return text.to_owned();
    }
    let (_kind, text) = fileencoding::detect_and_decode(b);
    String::from_utf8_lossy(&text).into_owned()
}

/// Keeps complete output up to the limit. Once output crosses the limit it
/// retains a head plus a rolling tail separated by the marker. Write always
/// reports the full input consumed so a safety cap never changes
/// child-process behavior into an artificial short-write failure.
struct BoundedBuffer {
    buf: Vec<u8>,
    tail: Vec<u8>,
    limit: usize,
    tail_limit: usize,
    marker: &'static str,
    truncated: bool,
}

impl BoundedBuffer {
    fn new(limit: usize, tail_limit: usize, marker: &'static str) -> BoundedBuffer {
        BoundedBuffer {
            buf: Vec::new(),
            tail: Vec::new(),
            limit,
            tail_limit,
            marker,
            truncated: false,
        }
    }

    fn write(&mut self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        if !self.truncated && (self.limit == 0 || self.buf.len() + p.len() <= self.limit) {
            self.buf.extend_from_slice(p);
            return;
        }
        if !self.truncated {
            self.truncated = true;
            let head_limit = self
                .limit
                .saturating_sub(self.tail_limit + self.marker.len());
            let previous = std::mem::take(&mut self.buf);
            append_bounded_tail(&mut self.tail, &previous, self.tail_limit);
            self.buf = previous[..head_limit.min(previous.len())].to_vec();
        }
        append_bounded_tail(&mut self.tail, p, self.tail_limit);
    }

    fn bytes(&self) -> Vec<u8> {
        if !self.truncated {
            return self.buf.clone();
        }
        let mut out = Vec::with_capacity(self.buf.len() + self.marker.len() + self.tail.len());
        out.extend_from_slice(&self.buf);
        out.extend_from_slice(self.marker.as_bytes());
        out.extend_from_slice(&self.tail);
        out
    }
}

fn append_bounded_tail(dst: &mut Vec<u8>, p: &[u8], limit: usize) {
    if limit == 0 {
        dst.clear();
        return;
    }
    if p.len() >= limit {
        dst.clear();
        dst.extend_from_slice(&p[p.len() - limit..]);
        return;
    }
    let overflow = (dst.len() + p.len()).saturating_sub(limit);
    if overflow > 0 {
        dst.drain(..overflow);
    }
    dst.extend_from_slice(p);
}

/// The bounded tail of a failed run (Go `tailWriter`).
struct TailWriter {
    limit: usize,
    buf: Vec<u8>,
}

impl TailWriter {
    fn new(limit: usize) -> TailWriter {
        TailWriter {
            limit,
            buf: Vec::new(),
        }
    }

    fn write(&mut self, p: &[u8]) {
        self.buf.extend_from_slice(p);
        if self.limit > 0 && self.buf.len() > self.limit {
            let start = self.buf.len() - self.limit;
            self.buf = self.buf[start..].to_vec();
        }
    }
}

/// Renders a duration the way Go's `time.Duration.String` reads, so the
/// timeout messages name deadlines in the shape the reference model learned.
pub fn go_duration_text(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos == 0 {
        return String::from("0s");
    }
    if nanos < 1_000 {
        return format!("{nanos}ns");
    }
    if nanos < 1_000_000 {
        return trim_unit(nanos as f64 / 1_000.0, "µs");
    }
    if nanos < 1_000_000_000 {
        return trim_unit(nanos as f64 / 1_000_000.0, "ms");
    }
    let mut seconds = nanos as f64 / 1_000_000_000.0;
    let seconds_int = seconds as u64;
    let minutes = seconds_int / 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;
    let mut out = String::new();
    if hours > 0 {
        out.push_str(&format!("{hours}h"));
    }
    if minutes > 0 || !out.is_empty() {
        out.push_str(&format!("{minutes}m"));
    }
    seconds = (seconds - (hours as f64 * 3600.0) - (minutes as f64 * 60.0)).max(0.0);
    if !out.is_empty() || seconds > 0.0 {
        if seconds == (seconds as u64) as f64 {
            out.push_str(&format!("{}s", seconds as u64));
        } else {
            out.push_str(&format!("{seconds}s"));
        }
    }
    out
}

fn trim_unit(value: f64, unit: &str) -> String {
    if value == (value as u64) as f64 {
        format!("{}{unit}", value as u64)
    } else {
        format!("{value}{unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_buffer_keeps_head_and_tail() {
        let mut b = BoundedBuffer::new(100, 20, "\n[cut]\n");
        b.write(b"0123456789".repeat(5).as_slice());
        assert!(!b.truncated);
        b.write(&[b'x'; 200]);
        let out = b.bytes();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("0123456789"), "head kept");
        assert!(text.contains("[cut]"), "marker names the cut");
        assert!(text.ends_with('x'), "tail kept");
        assert!(text.len() <= 100 + 30, "bounded at {}", text.len());
    }

    #[test]
    fn bounded_buffer_under_limit_is_verbatim() {
        let mut b = BoundedBuffer::new(1000, 20, "\n[cut]\n");
        b.write(b"hello");
        assert_eq!(b.bytes(), b"hello");
    }

    #[test]
    fn tail_writer_keeps_the_last_limit_bytes() {
        let mut t = TailWriter::new(8);
        t.write(b"abcdef");
        t.write(b"ghijklmn");
        assert_eq!(t.buf, b"ghijklmn");
    }

    #[test]
    fn go_duration_reads_like_go() {
        assert_eq!(go_duration_text(Duration::from_millis(150)), "150ms");
        assert_eq!(go_duration_text(Duration::from_secs(1)), "1s");
        assert_eq!(go_duration_text(Duration::from_secs(120)), "2m0s");
        assert_eq!(go_duration_text(Duration::from_secs(600)), "10m0s");
        assert_eq!(go_duration_text(Duration::from_secs(3661)), "1h1m1s");
        assert_eq!(go_duration_text(Duration::ZERO), "0s");
    }

    #[test]
    fn exit_status_names_the_code_like_go() {
        assert_eq!(RunError::Exit { code: 3 }.to_string(), "exit status 3");
        assert_eq!(
            RunError::Timeout {
                deadline_text: String::from("1s"),
                silent: false
            }
            .to_string(),
            "command timed out (> 1s)"
        );
        assert_eq!(
            RunError::Timeout {
                deadline_text: String::from("150ms"),
                silent: true
            }
            .to_string(),
            "command timed out (> 150ms); it produced no output before the deadline"
        );
    }

    #[test]
    fn decode_shell_output_reads_utf8_and_sniffs_the_rest() {
        assert_eq!(decode_shell_output(b"plain"), "plain");
        assert_eq!(decode_shell_output("中文".as_bytes()), "中文");
        // GBK bytes for 中文 decode through the sniffing path.
        let gbk = vec![0xD6, 0xD0, 0xCE, 0xC4];
        assert_eq!(decode_shell_output(&gbk), "中文");
    }

    #[test]
    fn run_foreground_classifies_exits() {
        let sh = if cfg!(windows) { "cmd" } else { "sh" };
        let argv: Vec<String> = if cfg!(windows) {
            vec![sh.to_owned(), String::from("/c"), String::from("exit 3")]
        } else {
            vec![sh.to_owned(), String::from("-c"), String::from("exit 3")]
        };
        let out = run_foreground(
            Request {
                argv,
                ..Request::default()
            },
            None,
        );
        assert_eq!(out.state, ShellState::Failed);
        assert_eq!(out.exit_code, Some(3));
        assert_eq!(
            out.err,
            Some(RunError::Exit { code: 3 }),
            "raw exit text: {:?}",
            out.err
        );
        assert!(out.combined.is_empty());
        assert!(out.output_tail.is_empty(), "no tail on a silent run");
    }

    #[test]
    fn run_foreground_launch_failure_names_the_cause() {
        let out = run_foreground(
            Request {
                argv: vec![
                    String::from("reasonix-no-such-interpreter"),
                    String::from("-c"),
                ],
                ..Request::default()
            },
            None,
        );
        assert_eq!(out.state, ShellState::Failed);
        assert!(!out.started);
        assert!(matches!(out.err, Some(RunError::Launch { .. })));
    }

    #[test]
    fn run_foreground_times_out_at_the_deadline() {
        let sh = if cfg!(windows) { "cmd" } else { "sh" };
        let sleep = if cfg!(windows) {
            String::from("timeout /t 5 /nobreak >nul")
        } else {
            String::from("sleep 5")
        };
        let start = Instant::now();
        let out = run_foreground(
            Request {
                argv: vec![sh.to_owned(), flag_for(sh), sleep],
                timeout: Duration::from_millis(150),
                ..Request::default()
            },
            None,
        );
        assert_eq!(out.state, ShellState::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5), "returned slowly");
        assert_eq!(out.timeout_text, "150ms");
        assert!(matches!(out.err, Some(RunError::Timeout { .. })));
    }

    #[test]
    fn run_foreground_collects_combined_output() {
        let sh = if cfg!(windows) { "cmd" } else { "sh" };
        let script = if cfg!(windows) {
            String::from("echo hello & echo world 1>&2")
        } else {
            String::from("echo hello; echo world >&2")
        };
        let out = run_foreground(
            Request {
                argv: vec![sh.to_owned(), flag_for(sh), script],
                ..Request::default()
            },
            None,
        );
        assert_eq!(out.state, ShellState::Completed);
        assert!(
            out.combined.contains("hello"),
            "combined = {:?}",
            out.combined
        );
        assert!(
            out.combined.contains("world"),
            "stderr joins combined: {:?}",
            out.combined
        );
    }

    #[test]
    fn cancel_token_stops_a_long_child() {
        let sh = if cfg!(windows) { "cmd" } else { "sh" };
        let sleep = if cfg!(windows) {
            String::from("timeout /t 5 /nobreak >nul")
        } else {
            String::from("sleep 5")
        };
        let token = CancelToken::default();
        let flag = token.0.clone();
        let spawner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            flag.store(true, Ordering::SeqCst);
        });
        let out = run_foreground(
            Request {
                argv: vec![sh.to_owned(), flag_for(sh), sleep],
                ..Request::default()
            },
            Some(&token),
        );
        spawner.join().unwrap();
        assert_eq!(out.state, ShellState::Cancelled);
        assert!(matches!(out.err, Some(RunError::Cancelled)));
    }

    fn flag_for(sh: &str) -> String {
        if sh == "cmd" {
            String::from("/c")
        } else {
            String::from("-c")
        }
    }
}
