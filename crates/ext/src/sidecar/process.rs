//! Sidecar process spawn and bounded teardown (contract §14.3).
//!
//! Parity target: `internal/ext/extension/sidecar/process.go`. A sidecar is
//! always spawned in exec form from the resolved executable plus its argv: the
//! runtime contract promises no shell, so pipes, `&&` and `$` expansion carry
//! no meaning. stderr is captured into a bounded ring and surfaced only after
//! credential redaction.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::installed::{RuntimeEnv, RuntimeSpec};
use super::{StartupFailure, startup_failure};
use crate::redact::{redact_credentials, redact_error_text};

/// How long a protocol-aware sidecar gets to exit after stdin closes.
pub const GRACEFUL_CLOSE_WAIT: Duration = Duration::from_millis(750);
/// How long a hard process-tree kill gets to be reaped.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Bytes of the most recent stderr retained for diagnostics.
pub const STDERR_TAIL_BYTES: usize = 16 << 10;

/// Environment variables that always identify the installed package.
pub const ENV_PLUGIN_ROOT: &str = "REASONIX_PLUGIN_ROOT";
pub const ENV_PLUGIN_NAME: &str = "REASONIX_PLUGIN_NAME";
pub const ENV_PLUGIN_VERSION: &str = "REASONIX_PLUGIN_VERSION";

/// The placeholder a manifest command may use to address its own package root.
pub const PLUGIN_ROOT_VAR: &str = "${REASONIX_PLUGIN_ROOT}";

/// Interpreter names a runtime command may never resolve to.
///
/// Routing through one would smuggle shell semantics into a contract that
/// promises there are none.
const SHELL_EXECUTABLES: [&str; 12] = [
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "cmd",
    "cmd.exe",
    "powershell",
    "powershell.exe",
    "pwsh",
    "pwsh.exe",
];

/// Why a runtime command could not be launched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnError {
    #[error("runtime command is empty after expansion")]
    Empty,
    #[error(
        "runtime command {command:?} is not an absolute path after expansion (use {PLUGIN_ROOT_VAR} to address the installed package)"
    )]
    NotAbsolute { command: String },
    #[error(
        "runtime command {command:?} is a shell; the runtime contract is exec form (the command is the extension executable, args are its argv)"
    )]
    Shell { command: String },
    #[error(
        "runtime command {command:?} is not an executable program: {message}; the plugin has to ship a native executable for this system"
    )]
    NotExecutable { command: String, message: String },
    #[error("spawn {command:?}: {message}")]
    Failed { command: String, message: String },
}

/// Expands `${REASONIX_PLUGIN_ROOT}` and enforces the exec contract.
///
/// The resolved command must be an absolute path to the extension executable:
/// a relative name would fall back to a `PATH` lookup, so the package could not
/// say what it runs.
pub fn resolve_runtime_command(command: &str, root: &Path) -> Result<PathBuf, SpawnError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(SpawnError::Empty);
    }
    let expanded = match trimmed.strip_prefix(PLUGIN_ROOT_VAR) {
        Some(rest) => {
            let rest = rest.trim_start_matches(['/', '\\']);
            root.join(rest.replace('\\', "/"))
        }
        None => PathBuf::from(trimmed),
    };
    if !is_absolute(&expanded) {
        return Err(SpawnError::NotAbsolute {
            command: trimmed.to_string(),
        });
    }
    let base = expanded
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if SHELL_EXECUTABLES.contains(&base.as_str()) {
        return Err(SpawnError::Shell {
            command: trimmed.to_string(),
        });
    }
    Ok(expanded)
}

/// Absolute-path check matching Go's `filepath.IsAbs`: a Windows path is
/// absolute when it carries a drive/UNC prefix or leads with a separator.
fn is_absolute(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    #[cfg(windows)]
    {
        let Some(first) = bytes.first() else {
            return false;
        };
        if matches!(first, b'/' | b'\\') {
            return true;
        }
        bytes.len() >= 2 && bytes[1] == b':' && first.is_ascii_alphabetic()
    }
    #[cfg(unix)]
    {
        bytes.first() == Some(&b'/')
    }
}

/// Builds the sidecar environment.
///
/// The unfiltered inherited environment comes first (the full-trust contract an
/// installed runtime accepted at install time), then the manifest's `env`, then
/// the package identity variables: later entries win, so a manifest can tune
/// but cannot forge its own identity.
pub fn runtime_env(
    env: &RuntimeEnv,
    manifest_env: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::env::vars().collect();
    for (key, value) in manifest_env {
        set_env(&mut out, key, value);
    }
    let version = if env.version.trim().is_empty() {
        env.manifest_version.trim().to_string()
    } else {
        env.version.trim().to_string()
    };
    set_env(&mut out, ENV_PLUGIN_ROOT, &env.root);
    set_env(&mut out, ENV_PLUGIN_NAME, &env.name);
    set_env(&mut out, ENV_PLUGIN_VERSION, &version);
    out
}

fn set_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(existing) = env.iter_mut().find(|(k, _)| env_key_equal(k, key)) {
        existing.1 = value.to_string();
    } else {
        env.push((key.to_string(), value.to_string()));
    }
}

fn env_key_equal(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// A bounded ring holding the most recent stderr bytes.
///
/// Writes never block the child: a reader thread drains the pipe, and only the
/// tail is ever surfaced, only after redaction.
#[derive(Debug)]
pub struct TailBuffer {
    limit: usize,
    buf: Mutex<Vec<u8>>,
}

impl TailBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            buf: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, chunk: &[u8]) {
        let mut buf = self.buf.lock().expect("tail buffer mutex");
        buf.extend_from_slice(chunk);
        if self.limit > 0 && buf.len() > self.limit {
            let excess = buf.len() - self.limit;
            buf.drain(..excess);
        }
    }

    /// The retained tail, trimmed and credential-redacted.
    pub fn redacted(&self) -> String {
        let buf = self.buf.lock().expect("tail buffer mutex");
        let text = String::from_utf8_lossy(&buf);
        redact_credentials(text.trim())
    }

    /// Retained bytes before redaction, for bound assertions.
    pub fn len(&self) -> usize {
        self.buf.lock().expect("tail buffer mutex").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A spawned sidecar: pipes, the stderr tail, and the reaper.
pub struct SidecarProcess {
    name: String,
    pid: u32,
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Mutex<Option<ChildStdout>>,
    stderr: Arc<TailBuffer>,
    reaper: Mutex<Option<JoinHandle<()>>>,
    exited: Arc<AtomicBool>,
    status: Arc<Mutex<Option<ExitStatus>>>,
    stderr_thread: Mutex<Option<JoinHandle<()>>>,
    killed: AtomicBool,
}

impl std::fmt::Debug for SidecarProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarProcess")
            .field("name", &self.name)
            .field("pid", &self.pid)
            .field("exited", &self.exited.load(Ordering::SeqCst))
            .finish()
    }
}

impl SidecarProcess {
    /// Spawns the runtime command in exec form.
    ///
    /// stdout/stdin become the protocol pipes; stderr is captured into the
    /// bounded tail. The child never opens a console window on Windows.
    pub fn spawn(
        name: &str,
        runtime: &RuntimeSpec,
        env: &RuntimeEnv,
        root: &Path,
        manifest_env: &BTreeMap<String, String>,
    ) -> Result<Self, SpawnError> {
        let command = resolve_runtime_command(&runtime.command, root)?;
        let mut cmd = Command::new(&command);
        cmd.args(&runtime.args);
        cmd.env_clear();
        for (key, value) in runtime_env(env, manifest_env) {
            cmd.env(key, value);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(root);
        configure_child(&mut cmd);
        let mut child = cmd.spawn().map_err(|err| spawn_error(&command, err))?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let stderr = Arc::new(TailBuffer::new(STDERR_TAIL_BYTES));
        let stderr_thread = stderr_pipe.map(|pipe| spawn_stderr_reader(pipe, Arc::clone(&stderr)));
        let exited = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(None));
        let reaper = spawn_reaper(child, Arc::clone(&exited), Arc::clone(&status));
        Ok(Self {
            name: name.to_string(),
            pid,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
            stderr,
            reaper: Mutex::new(Some(reaper)),
            exited,
            status,
            stderr_thread: Mutex::new(stderr_thread),
            killed: AtomicBool::new(false),
        })
    }

    /// Takes the protocol pipes, leaving the process handle with this type.
    pub fn take_pipes(&self) -> (Option<ChildStdout>, Option<ChildStdin>) {
        (
            self.stdout.lock().expect("process mutex").take(),
            self.stdin.lock().expect("process mutex").take(),
        )
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr.redacted()
    }

    pub fn stderr_len(&self) -> usize {
        self.stderr.len()
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Waits up to `budget` for the process to exit.
    pub fn wait_for_exit(&self, budget: Duration) -> bool {
        let started = Instant::now();
        loop {
            if self.has_exited() {
                return true;
            }
            if started.elapsed() >= budget {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5).min(budget));
        }
    }

    /// Terminates the whole process tree.
    ///
    /// A launcher's surviving grandchild can keep inherited pipes open, so the
    /// kill walks the tree rather than signalling only the leader.
    pub fn kill_tree(&self) {
        if self.killed.swap(true, Ordering::SeqCst) {
            return;
        }
        kill_process_tree(self.pid);
    }

    /// The bounded close sequence: stdin EOF, a short grace, a tree kill, and
    /// a bounded reap. Idempotent and never longer than the two budgets.
    pub fn close(&self) {
        self.stdin.lock().expect("process mutex").take();
        if self.wait_for_exit(GRACEFUL_CLOSE_WAIT) {
            self.reap();
            return;
        }
        self.kill_tree();
        self.wait_for_exit(CLOSE_WAIT);
        self.reap();
    }

    fn reap(&self) {
        if let Some(handle) = self.reaper.lock().expect("process mutex").take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.lock().expect("process mutex").take() {
            let _ = handle.join();
        }
    }

    /// The exit status once the process is reaped.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        *self.status.lock().expect("process mutex")
    }
}

impl Drop for SidecarProcess {
    /// A dropped process never outlives its owner: the tree is killed and the
    /// reaper is detached rather than joined, so a caller cannot be stalled.
    fn drop(&mut self) {
        self.stdin.lock().expect("process mutex").take();
        if !self.has_exited() {
            self.kill_tree();
        }
    }
}

fn spawn_stderr_reader(mut pipe: ChildStderr, tail: Arc<TailBuffer>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => return,
                Ok(read) => tail.push(&chunk[..read]),
                Err(_) => return,
            }
        }
    })
}

fn spawn_reaper(
    mut child: Child,
    exited: Arc<AtomicBool>,
    status: Arc<Mutex<Option<ExitStatus>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        if let Ok(reaped) = child.wait() {
            *status.lock().expect("process mutex") = Some(reaped);
        }
        exited.store(true, Ordering::SeqCst);
    })
}

/// Wraps a spawn failure with bounded, redacted startup diagnostics.
pub fn spawn_failure(
    stage: &'static str,
    started: Instant,
    stderr: &TailBuffer,
    err: &SpawnError,
) -> StartupFailure {
    startup_failure(
        stage,
        started.elapsed(),
        stderr.redacted(),
        redact_error_text(&err.to_string()),
    )
}

fn spawn_error(command: &Path, err: std::io::Error) -> SpawnError {
    let command = redact_error_text(&command.display().to_string());
    let message = redact_error_text(&err.to_string());
    if is_bad_exec_format(&err) {
        return SpawnError::NotExecutable { command, message };
    }
    SpawnError::Failed { command, message }
}

/// `ERROR_BAD_EXE_FORMAT` is 193; a shell script on Windows reports it.
fn is_bad_exec_format(err: &std::io::Error) -> bool {
    cfg!(windows) && err.raw_os_error() == Some(193)
}

#[cfg(windows)]
fn configure_child(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn configure_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A private process group makes the negative-pid kill reach descendants.
    cmd.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn configure_child(_cmd: &mut Command) {}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    // taskkill /T walks the live tree; /F ends it. Bounded so a hung taskkill
    // cannot stall teardown.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("taskkill");
    command.args(["/F", "/T", "/PID", &pid.to_string()]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.creation_flags(CREATE_NO_WINDOW);
    let _ = command.status();
}

#[cfg(unix)]
fn kill_process_tree(pid: u32) {
    // The child leads its own process group, so the negative pid reaches every
    // descendant. `kill(1)` keeps the port without linking libc.
    for target in [format!("-{pid}"), pid.to_string()] {
        let _ = Command::new("kill")
            .args(["-KILL", &target])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(not(any(windows, unix)))]
fn kill_process_tree(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(root: &str) -> RuntimeEnv {
        RuntimeEnv {
            root: root.to_string(),
            name: "demo".into(),
            version: "1.2.3".into(),
            manifest_version: "0.0.0".into(),
        }
    }

    #[test]
    fn command_expansion_requires_an_absolute_non_shell_path() {
        let root = Path::new("/opt/plugins/demo");
        let resolved = resolve_runtime_command("${REASONIX_PLUGIN_ROOT}/bin/demo", root).unwrap();
        assert_eq!(resolved, root.join("bin/demo"));

        for (command, want) in [
            ("  ", SpawnError::Empty),
            (
                "node",
                SpawnError::NotAbsolute {
                    command: "node".into(),
                },
            ),
            (
                "bin/demo",
                SpawnError::NotAbsolute {
                    command: "bin/demo".into(),
                },
            ),
            (
                "/bin/sh",
                SpawnError::Shell {
                    command: "/bin/sh".into(),
                },
            ),
        ] {
            assert_eq!(resolve_runtime_command(command, root), Err(want));
        }
        // A Windows shell spelling is only absolute on Windows: elsewhere the
        // drive-letter form answers NotAbsolute before the shell rule can see it.
        #[cfg(windows)]
        assert!(matches!(
            resolve_runtime_command("C:\\Windows\\System32\\cmd.exe", root),
            Err(SpawnError::Shell { .. })
        ));
    }

    #[test]
    fn identity_variables_cannot_be_forged_by_manifest_env() {
        let mut manifest = BTreeMap::new();
        manifest.insert(ENV_PLUGIN_NAME.to_string(), "spoofed".to_string());
        manifest.insert("EXTRA".to_string(), "kept".to_string());
        let built = runtime_env(&env("/opt/plugins/demo"), &manifest);
        let lookup = |key: &str| built.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        assert_eq!(lookup(ENV_PLUGIN_NAME).as_deref(), Some("demo"));
        assert_eq!(
            lookup(ENV_PLUGIN_ROOT).as_deref(),
            Some("/opt/plugins/demo")
        );
        assert_eq!(lookup(ENV_PLUGIN_VERSION).as_deref(), Some("1.2.3"));
        assert_eq!(lookup("EXTRA").as_deref(), Some("kept"));
    }

    #[test]
    fn tail_buffer_keeps_only_the_end_and_redacts() {
        let tail = TailBuffer::new(64);
        for index in 0..32 {
            tail.push(format!("line {index} padding padding\n").as_bytes());
        }
        assert_eq!(tail.len(), 64);
        tail.push(b"failed: api_key=sk-abcdef1234567890SECRETKEY\n");
        let redacted = tail.redacted();
        assert!(
            !redacted.contains("sk-abcdef1234567890SECRETKEY"),
            "{redacted}"
        );
        assert!(redacted.contains("failed"), "{redacted}");
    }
}
