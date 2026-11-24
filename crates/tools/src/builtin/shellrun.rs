//! TOOL-045: the shared foreground shell runner parity (internal/tools/shellrun,
//! docs/contracts/tools.md §4).
//!
//! Done here: the exit-classification decision table, the bounded combined
//! collector and tail ring, the live progress writer (line-complete chunks,
//! held non-ASCII, suppressed-line drop, cap marker), the login-shell PATH
//! merge decision table, the shell descriptor classifier, the operative-command
//! subject cut, and console-output decoding.
//!
//! WIP (tracked with the OS-sandbox wave; see #6473, #6528): the real
//! child-process spawn itself lives in [`super::bashrun`]; the platform probe,
//! the login-shell lookup (`LookPath`/executable stat) and the memoized PATH
//! probe subprocess are host machinery this module's decision tables assume
//! are supplied by the runner.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reasonix_core::fileencoding::{self, Kind};
use reasonix_safety::sandbox::shell::{Shell, ShellKind};

use crate::shell::{ShellExecution, ShellName, ShellPhase, ShellState, ShellVersion};

use super::bash_env::{env_value, set_env_value};

/// Mirrors the bash tool's child-process wait grace (Go `DefaultWaitDelay`).
pub const DEFAULT_WAIT_DELAY: Duration = Duration::from_secs(5);

/// Bounds the foreground output retained in memory. Tool-result truncation
/// happens only after the process exits, so it cannot protect the host from a
/// command that prints forever (#6473, #6528).
pub const COMBINED_OUTPUT_MAX_BYTES: usize = 10 << 20;
/// Keep the final diagnostics as well as the command's opening context after
/// the cap is crossed: build and test failures are commonly printed last.
pub const COMBINED_OUTPUT_TAIL_BYTES: usize = 64 << 10;
pub const COMBINED_OUTPUT_TRUNCATED: &str =
    "\n\n...[shell output truncated at 10 MiB; showing the final 64 KiB]...\n\n";

/// Live progress crosses async UI queues and append-only reducers before the
/// final bounded result replaces it; keep that transient path small too.
pub const PROGRESS_OUTPUT_MAX_BYTES: usize = 64 << 10;
pub const PROGRESS_OUTPUT_TRUNCATED: &str = "\n\n...[live shell output capped at 64 KiB; final diagnostics will appear when the command exits]...\n\n";

/// The fallback chain when `$SHELL` names no usable interpreter (Go
/// `loginShell`). POSIX-only; Windows returns the process env unchanged.
pub const LOGIN_SHELL_FALLBACKS: [&str; 3] = ["/bin/zsh", "/bin/bash", "/bin/sh"];

/// The login-shell probe argument sets tried in order (Go `defaultShellPATH`).
/// The script prints the marker line carrying the probe shell's `$PATH`.
pub const PATH_PROBE_SCRIPT: &str = "printf '\\n__REASONIX_BASH_PATH__=%s\\n' \"$PATH\"";
pub const PATH_PROBE_MARKER: &str = "__REASONIX_BASH_PATH__=";

// ---------------------------------------------------------------------------
// Exit classification (§4.1)
// ---------------------------------------------------------------------------

/// What the runner observed, fed into the pure classifier. The spawn half
/// that produces this is the bashrun runner; the decision table here is
/// host-independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Runner returned no error; exit status 0.
    Success,
    /// Child exited with a known code.
    Exit(i32),
    /// Parent cancellation / user stop.
    Cancelled,
    /// Tool-local deadline fired.
    TimedOut,
    /// Error without an exit status. `started` says whether the process ran;
    /// `wait_delay_expired` mirrors Go's `exec.ErrWaitDelay`; `message` is the
    /// launch/execution cause the classifier surfaces.
    Failed {
        started: bool,
        wait_delay_expired: bool,
        message: String,
    },
}

/// The classified outcome of a foreground run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub state: ShellState,
    pub phase: Option<ShellPhase>,
    pub exit_code: Option<i32>,
    pub started: bool,
    /// Bounded tail of failed combined output; empty on success.
    pub output_tail: String,
    pub err: Option<ShellRunError>,
}

/// Why a run failed, kept typed so the tool names whose deadline fired and
/// whether anything ran.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellRunError {
    /// No argv to launch.
    #[error("empty argv")]
    EmptyArgv,
    /// The tool-local deadline fired.
    #[error("command timed out (> {deadline_text}){silence_note}")]
    Timeout {
        deadline_text: String,
        silence_note: &'static str,
    },
    /// The parent cancelled the call.
    #[error("context canceled")]
    Cancelled,
    /// The child produced an exit status.
    #[error("command exited: exit status {code}")]
    Exit { code: i32 },
    /// Launch failed: the interpreter could not be started.
    #[error("{message}")]
    Launch { message: String },
    /// The child ran and failed without an exit status.
    #[error("{message}")]
    Execution { message: String },
}

/// Reports that a run reached its deadline having printed nothing. A caller
/// cannot read that from the output: no output and output that never arrived
/// look identical, yet they call for opposite next moves.
pub fn timeout_silence_note(combined: &str) -> &'static str {
    if combined.trim().is_empty() {
        "; it produced no output before the deadline"
    } else {
        ""
    }
}

/// Classifies a run in Go's precise order: empty argv → timeout → cancellation
/// → success → known exit code → no exit status (launch vs execution).
///
/// `preserve_wait_delay` mirrors Go: when the tool-local context is still
/// alive and the only error is the wait-delay overrun, the error is cleared —
/// a wait-delay overrun is not a command failure.
pub fn classify(
    argv: &[String],
    outcome: Outcome,
    combined: &str,
    output_tail: &str,
    timeout_text: &str,
    preserve_wait_delay: bool,
) -> Classified {
    if argv.is_empty() {
        return Classified {
            state: ShellState::Failed,
            phase: Some(ShellPhase::Launch),
            exit_code: None,
            started: false,
            output_tail: String::new(),
            err: Some(ShellRunError::EmptyArgv),
        };
    }

    let outcome = match outcome {
        Outcome::Failed {
            wait_delay_expired: true,
            ..
        } if preserve_wait_delay => Outcome::Success,
        other => other,
    };
    match outcome {
        Outcome::TimedOut => Classified {
            state: ShellState::TimedOut,
            phase: Some(ShellPhase::Timeout),
            exit_code: None,
            started: true,
            output_tail: output_tail.to_owned(),
            err: Some(ShellRunError::Timeout {
                deadline_text: timeout_text.to_owned(),
                silence_note: timeout_silence_note(combined),
            }),
        },
        Outcome::Cancelled => Classified {
            state: ShellState::Cancelled,
            phase: Some(ShellPhase::Cancellation),
            exit_code: None,
            started: true,
            output_tail: output_tail.to_owned(),
            err: Some(ShellRunError::Cancelled),
        },
        Outcome::Success => Classified {
            state: ShellState::Completed,
            phase: None,
            exit_code: Some(0),
            started: true,
            // The tail exists to explain a failure; dropping it on success keeps
            // successful runs from persisting ordinary stdout into every record.
            output_tail: String::new(),
            err: None,
        },
        Outcome::Exit(code) => Classified {
            state: ShellState::Failed,
            phase: Some(ShellPhase::Execution),
            exit_code: Some(code),
            started: true,
            output_tail: output_tail.to_owned(),
            err: Some(ShellRunError::Exit { code }),
        },
        Outcome::Failed {
            started, message, ..
        } => {
            let phase = if started {
                ShellPhase::Execution
            } else {
                ShellPhase::Launch
            };
            let err = if started {
                ShellRunError::Execution { message }
            } else {
                ShellRunError::Launch { message }
            };
            Classified {
                state: ShellState::Failed,
                phase: Some(phase),
                exit_code: None,
                started,
                output_tail: output_tail.to_owned(),
                err: Some(err),
            }
        }
    }
}

impl Default for Classified {
    fn default() -> Self {
        Classified {
            state: ShellState::Failed,
            phase: None,
            exit_code: None,
            started: false,
            output_tail: String::new(),
            err: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded collector (§4.1)
// ---------------------------------------------------------------------------

/// Keeps complete output up to the limit; once crossed it retains a head plus
/// a rolling tail separated by the marker. Writes always report fully consumed
/// so a safety cap never changes child-process behavior into an artificial
/// short-write failure.
struct BoundedBuffer {
    buf: Vec<u8>,
    tail: Vec<u8>,
    limit: usize,
    tail_limit: usize,
    marker: &'static str,
    truncated: bool,
}

impl BoundedBuffer {
    fn new(limit: usize, tail_limit: usize, marker: &'static str) -> Self {
        Self {
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

/// The bounded tail of a failed run (Go `tailWriter`). `cut` says bytes before
/// `buf` were dropped to hold the limit.
struct TailWriter {
    limit: usize,
    buf: Vec<u8>,
    cut: bool,
}

impl TailWriter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            buf: Vec::new(),
            cut: false,
        }
    }

    fn write(&mut self, p: &[u8]) {
        self.buf.extend_from_slice(p);
        if self.limit > 0 && self.buf.len() > self.limit {
            let start = self.buf.len() - self.limit;
            self.buf = self.buf[start..].to_vec();
            self.cut = true;
        }
    }
}

/// Sink for live progress lines; shared by the collector and the test seam.
pub type ProgressEmit = Arc<dyn Fn(&str) + Send + Sync>;

/// Live progress: whole lines first so a code-page character split across
/// pipe reads is not mistaken for invalid UTF-8; bare ASCII forwards
/// immediately so dots and prompts stay live.
struct ProgressWriter {
    emit: Option<ProgressEmit>,
    limit: usize,
    forwarded: usize,
    accepted: usize,
    marker: &'static str,
    truncated: bool,
    /// A line the host appended for its own use; dropped from the live stream.
    suppress: String,
    held: Vec<u8>,
    pending: Vec<u8>,
}

impl ProgressWriter {
    fn new(emit: Option<ProgressEmit>, limit: usize, marker: &'static str) -> Self {
        Self {
            emit,
            limit,
            forwarded: 0,
            accepted: 0,
            marker,
            truncated: false,
            suppress: String::new(),
            held: Vec::new(),
            pending: Vec::new(),
        }
    }

    fn set_suppress(&mut self, suppress: &str) {
        self.suppress = suppress.to_owned();
    }

    fn write(&mut self, p: &[u8]) -> usize {
        if p.is_empty() {
            return 0;
        }
        let written = p.len();
        if self.emit.is_none() || self.truncated {
            return written;
        }
        let p: Vec<u8> = if !self.suppress.is_empty() {
            self.without_suppressed(p)
        } else {
            p.to_vec()
        };
        if p.is_empty() {
            return written;
        }
        let remaining = self.limit.saturating_sub(self.accepted);
        let forward = p.len().min(remaining);
        if forward > 0 {
            self.accepted += forward;
            self.write_decoded(&p[..forward]);
        }
        if forward < p.len() && !self.truncated {
            self.flush_pending(Cut {
                head: false,
                tail: true,
            });
            self.truncate();
        }
        written
    }

    /// Releases the last line after the child exits. Until then non-ASCII
    /// bytes stay together so a split code-page character is not recorded as a
    /// replacement rune.
    fn flush(&mut self) {
        if !self.truncated {
            self.flush_pending(Cut::default());
        }
    }

    fn write_decoded(&mut self, p: &[u8]) {
        self.pending.extend_from_slice(p);
        while !self.truncated {
            let Some(end) = self.pending.iter().position(|&c| c == b'\n') else {
                break;
            };
            let line: Vec<u8> = self.pending[..=end].to_vec();
            self.pending.drain(..=end);
            self.emit_decoded(&line, Cut::default());
        }
        if self.truncated {
            self.pending.clear();
            return;
        }
        // ASCII has the same meaning in UTF-8 and the Windows code pages.
        let safe = self.pending.iter().take_while(|&&c| c < 0x80).count();
        if safe > 0 {
            let chunk: Vec<u8> = self.pending[..safe].to_vec();
            self.pending.drain(..safe);
            self.emit_decoded(&chunk, Cut::default());
        }
    }

    fn flush_pending(&mut self, cut: Cut) {
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            self.emit_decoded(&pending, cut);
        }
    }

    fn emit_decoded(&mut self, data: &[u8], cut: Cut) {
        let mut decoded = decode_shell_output(data, cut);
        let remaining = self.limit.saturating_sub(self.forwarded);
        let clipped = decoded.len() > remaining;
        if clipped {
            let mut end = remaining;
            let bytes = decoded.as_bytes();
            while end > 0 && !is_rune_start(bytes[end]) {
                end -= 1;
            }
            decoded = decoded[..end].to_owned();
        }
        if !decoded.is_empty() {
            self.forwarded += decoded.len();
            if let Some(emit) = &self.emit {
                emit(&decoded);
            }
        }
        if clipped {
            self.truncate();
        }
    }

    fn truncate(&mut self) {
        if self.truncated {
            return;
        }
        self.truncated = true;
        if !self.marker.is_empty()
            && let Some(emit) = &self.emit
        {
            emit(self.marker);
        }
    }

    /// Drops whole lines carrying the suppressed text and holds back a tail that
    /// could be its beginning, so a report split across two writes is still
    /// caught. What stays held when the process ends is at most the report.
    fn without_suppressed(&mut self, p: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.held.len() + p.len());
        buf.extend_from_slice(&self.held);
        buf.extend_from_slice(p);
        self.held.clear();
        if let Some(cut) = partial_suffix_start(&buf, &self.suppress) {
            self.held.extend_from_slice(&buf[cut..]);
            buf.truncate(cut);
        }
        if !buf
            .windows(self.suppress.len())
            .any(|w| w == self.suppress.as_bytes())
        {
            return buf;
        }
        let mut kept: Vec<u8> = Vec::new();
        for line in buf.split_inclusive(|&c| c == b'\n') {
            if !line
                .windows(self.suppress.len())
                .any(|w| w == self.suppress.as_bytes())
            {
                kept.extend_from_slice(line);
            }
        }
        self.held.clear();
        kept
    }
}

/// Returns where `buf` ends with a proper prefix of `marker`, or None. A
/// whole marker already inside buf is not "partial".
fn partial_suffix_start(buf: &[u8], marker: &str) -> Option<usize> {
    let marker = marker.as_bytes();
    let limit = buf.len().min(marker.len().saturating_sub(1));
    for n in (1..=limit).rev() {
        if buf.ends_with(&marker[..n]) {
            return Some(buf.len() - n);
        }
    }
    None
}

/// Owns the combined buffer, the bounded tail and the optional progress sink.
/// Writes stay serialized behind one mutex so two pipe readers cannot race the
/// buffer.
pub struct OutputCollector {
    combined: Mutex<BoundedBuffer>,
    tail: Mutex<TailWriter>,
    progress: Mutex<Option<ProgressWriter>>,
}

impl OutputCollector {
    /// Builds a collector sized for the foreground run.
    pub fn new(progress: Option<ProgressEmit>, suppress: &str) -> Self {
        let mut progress_writer = ProgressWriter::new(
            progress,
            PROGRESS_OUTPUT_MAX_BYTES,
            PROGRESS_OUTPUT_TRUNCATED,
        );
        progress_writer.set_suppress(suppress);
        Self {
            combined: Mutex::new(BoundedBuffer::new(
                COMBINED_OUTPUT_MAX_BYTES,
                COMBINED_OUTPUT_TAIL_BYTES,
                COMBINED_OUTPUT_TRUNCATED,
            )),
            tail: Mutex::new(TailWriter::new(crate::shell::OUTPUT_TAIL_MAX_BYTES)),
            progress: Mutex::new(Some(progress_writer)),
        }
    }

    /// Feeds one child-write into combined, tail and progress.
    pub fn write(&self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        if let Ok(mut combined) = self.combined.lock() {
            combined.write(p);
        }
        if let Ok(mut tail) = self.tail.lock() {
            tail.write(p);
        }
        if let Ok(mut progress) = self.progress.lock()
            && let Some(writer) = progress.as_mut()
        {
            writer.write(p);
        }
    }

    /// Flushes any held progress chunk after the child exits.
    pub fn flush_progress(&self) {
        if let Ok(mut progress) = self.progress.lock()
            && let Some(writer) = progress.as_mut()
        {
            writer.flush();
        }
    }

    /// Decoded combined output.
    pub fn combined_string(&self) -> String {
        let combined = self.combined.lock().expect("collector lock");
        decode_shell_output(&combined.bytes(), Cut::default())
    }

    /// Decoded bounded tail of a failed run.
    pub fn tail_string(&self) -> String {
        let tail = self.tail.lock().expect("collector lock");
        decode_shell_output(
            &tail.buf,
            Cut {
                head: tail.cut,
                tail: false,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Output decoding (§4.5)
// ---------------------------------------------------------------------------

/// Which ends of a bounded buffer lost bytes to its bound (Go `fileenc.Cut`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cut {
    /// Bytes before the buffer were dropped.
    pub head: bool,
    /// Bytes after the buffer were dropped.
    pub tail: bool,
}

fn is_rune_start(b: u8) -> bool {
    b & 0xC0 != 0x80
}

fn trim_edges(data: &[u8], cut: Cut) -> &[u8] {
    let mut data = data;
    if cut.head {
        for _ in 0..3 {
            if data.is_empty() || is_rune_start(data[0]) {
                break;
            }
            data = &data[1..];
        }
    }
    if cut.tail {
        data = fileencoding::trim_partial_rune(data);
    }
    data
}

/// Reads child bytes as text. A Windows console tool answers in the machine's
/// code page rather than UTF-8; valid UTF-8 stands as-is, a partial trailing
/// rune is trimmed when that alone was invalid, and anything else is sniffed
/// and decoded as the console code page. `cut` says which ends held a split
/// character.
pub fn decode_shell_output(b: &[u8], cut: Cut) -> String {
    let edges = trim_edges(b, cut);
    if let Ok(text) = std::str::from_utf8(edges) {
        return text.to_owned();
    }
    let whole = fileencoding::trim_partial_rune(edges);
    if whole.len() < edges.len()
        && let Ok(text) = std::str::from_utf8(whole)
        && text.chars().count() < whole.len()
    {
        return text.to_owned();
    }
    let (kind, _) = fileencoding::detect_and_decode(b);
    let kind = match kind {
        // No charset restores it; read it as GB18030, since a cut can split a
        // code-page character anywhere.
        Kind::LossyUtf8 => Kind::Gb18030,
        other => other,
    };
    let text = fileencoding::decode(b, kind);
    String::from_utf8_lossy(&text).into_owned()
}

// ---------------------------------------------------------------------------
// Login-shell PATH merge (§4.2)
// ---------------------------------------------------------------------------

/// Splits a PATH value on the host list separator, trims entries and dedupes
/// primary-first (Go `mergePathLists`).
pub fn merge_path_lists(primary: &str, secondary: &str) -> String {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for part in primary.split(sep).chain(secondary.split(sep)) {
        let part = part.trim();
        if part.is_empty() || seen.iter().any(|s| s == part) {
            continue;
        }
        seen.push(part.to_owned());
        out.push(part.to_owned());
    }
    out.join(&sep.to_string())
}

/// Parses the marker line out of a login-shell probe's combined output. The
/// last match wins; surrounding banner noise is ignored (Go `parseShellPATH`).
pub fn parse_shell_path(out: &[u8], marker: &str) -> String {
    let text = String::from_utf8_lossy(out).replace("\r\n", "\n");
    let mut found = String::new();
    let mut hit = false;
    for line in text.split('\n').rev() {
        if let Some(rest) = line.strip_prefix(marker) {
            found = rest.trim().to_owned();
            hit = true;
            break;
        }
    }
    if hit { found } else { String::new() }
}

/// The environment a shell command runs in: the process env, with the login
/// shell's PATH merged in on non-Windows. Windows returns the process env
/// unchanged. The probe subprocess itself is wip; `login_path` is its result.
pub fn command_env(env: Vec<String>, login_path: Option<&str>) -> Vec<String> {
    if cfg!(windows) {
        return env;
    }
    let Some(login_path) = login_path.map(str::trim).filter(|p| !p.is_empty()) else {
        return env;
    };
    let current = env_value(&env, "PATH").unwrap_or_default();
    let merged = merge_path_lists(login_path, &current);
    if merged == current {
        return env;
    }
    set_env_value(env, "PATH", &merged)
}

/// The ordered login-shell candidates for the resolution decision table: the
/// `$SHELL` value first, then the POSIX fallback chain. Executable stat and
/// `LookPath` resolution are wip host probes; this returns the paths to check.
pub fn login_shell_candidates(shell_env: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(shell) = shell_env.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(shell.to_owned());
    }
    out.extend(LOGIN_SHELL_FALLBACKS.iter().map(|s| s.to_string()));
    out
}

// ---------------------------------------------------------------------------
// Shell descriptor (§4.3)
// ---------------------------------------------------------------------------

/// Classifies a resolved shell to contract names: `powershell.exe` →
/// powershell/5.1; `pwsh` (or any chaining PowerShell) → pwsh/7+; a Git for
/// Windows bash path → git-bash; else bash.
pub fn classify_shell(
    kind: ShellKind,
    path: &str,
    supports_and_and: bool,
) -> (ShellName, Option<ShellVersion>) {
    let base = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    match kind {
        ShellKind::PowerShell => {
            if base == "pwsh" || supports_and_and {
                (ShellName::Pwsh, Some(ShellVersion::Ps7))
            } else {
                (ShellName::PowerShell, Some(ShellVersion::Ps51))
            }
        }
        ShellKind::Bash => {
            if is_git_bash_path(path) {
                (ShellName::GitBash, None)
            } else {
                (ShellName::Bash, None)
            }
        }
    }
}

fn is_git_bash_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let norm = path.replace('\\', "/").to_lowercase();
    norm.contains("/git/") && norm.contains("bash")
}

/// The human-facing shell label for cards and CLI lines (Go `DisplayName`).
pub fn display_name(shell: ShellName) -> &'static str {
    match shell {
        ShellName::GitBash => "Git Bash",
        ShellName::PowerShell => "Windows PowerShell",
        ShellName::Pwsh => "PowerShell 7+",
        ShellName::Bash => "bash",
    }
}

/// The host platform label: windows / darwin / linux.
pub fn platform_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// Fills the identity-only fields of a [`ShellExecution`] from a resolved
/// sandbox shell; callers set state/phase after the run.
pub fn descriptor_from_shell(shell: &Shell) -> ShellExecution {
    let supports_and_and = shell.supports_chaining();
    let (name, version) = classify_shell(shell.kind, &shell.path, supports_and_and);
    ShellExecution {
        shell: Some(name),
        shell_version: version,
        platform: Some(platform_name().to_owned()),
        supports_and_and,
        ..ShellExecution::default()
    }
}

// ---------------------------------------------------------------------------
// Operative command (§4.4)
// ---------------------------------------------------------------------------

/// Returns the command with its leading environment assignments dropped, and
/// whether any were. The cut is positional from the left edge, so quoting and
/// substitutions survive as written. Unparseable input or no leading
/// assignment prefix comes back unchanged.
///
/// WIP: the AST-precise byte-offset cut waits on the shellparse port; this
/// conservative scanner handles the leading `NAME=value` prefix the subject
/// surface records, including quoted values and `$(...)` substitutions.
pub fn operative_command(command: &str) -> (String, bool) {
    let bytes = command.as_bytes();
    let mut i = skip_ws(bytes, 0);
    let mut saw_assignment = false;

    while i < bytes.len() {
        let name_end = match assignment_name_end(bytes, i) {
            Some(end) => end,
            None => break,
        };
        saw_assignment = true;
        let value_start = name_end + 1;
        let value_end = scan_value(bytes, value_start);
        i = skip_ws(bytes, value_end);
    }

    if !saw_assignment || i >= bytes.len() {
        return (command.to_owned(), false);
    }
    let remainder = command[i..].trim().to_owned();
    if remainder.is_empty() {
        return (command.to_owned(), false);
    }
    (remainder, true)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// If the byte offset starts a `NAME=` assignment, returns the index just
/// after `=`; otherwise None.
fn assignment_name_end(b: &[u8], i: usize) -> Option<usize> {
    let start = i;
    let mut j = i;
    if !b[j].is_ascii_alphabetic() && b[j] != b'_' {
        return None;
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
    }
    if j < b.len() && b[j] == b'=' && j > start {
        Some(j)
    } else {
        None
    }
}

/// Scans an assignment value through quotes, substitutions and backticks to
/// the next unquoted whitespace.
fn scan_value(b: &[u8], mut i: usize) -> usize {
    let mut quote: Option<u8> = None;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => {
                match c {
                    b'"' | b'\'' => quote = Some(c),
                    b'(' => paren_depth += 1,
                    b')' => paren_depth = paren_depth.saturating_sub(1),
                    b'{' => brace_depth += 1,
                    b'}' => brace_depth = brace_depth.saturating_sub(1),
                    b'`' => quote = Some(b'`'),
                    _ if c.is_ascii_whitespace() && paren_depth == 0 && brace_depth == 0 => {
                        return i;
                    }
                    _ => {}
                }
                i += 1;
            }
        }
    }
    i
}
