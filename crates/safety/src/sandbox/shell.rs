//! The resolved interpreter the bash tool executes commands under (parity
//! target: `internal/safety/sandbox/shell.go`, the execution subset: the
//! type, argv construction, chaining support, PowerShell output encoding,
//! null-redirect normalization, and interpreter resolution with the probe
//! that keeps the WSL `bash.exe` launcher stub off the session).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// A console child pops a window under a GUI parent otherwise.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Output-encoding prologue for captured PowerShell execution: UTF-8 instead
/// of the console code page. Guarded per assignment so a missing console or
/// ConstrainedLanguage skips the line instead of printing an error ahead of
/// every command.
const PS_UTF8_PROLOGUE: &str = "if($ExecutionContext.SessionState.LanguageMode -eq 'FullLanguage'){try{[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)}catch{};try{$OutputEncoding=[Text.UTF8Encoding]::new($false)}catch{}};";

/// Tool-only defaults: Windows PowerShell 5.1 writes `>` and Out-File as
/// UTF-16LE, so there they default to UTF-8. Hooks keep the user's own
/// defaults, so this stays out of the prologue.
const PS_TOOL_FILE_DEFAULTS: &str = "if($PSVersionTable.PSVersion.Major -lt 6){try{$PSDefaultParameterValues['Out-File:Encoding']='utf8'}catch{}};";

/// Why auto-detection settled on PowerShell instead of bash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FallbackReason {
    #[default]
    None,
    NotFound,
    ProbeFailed,
    ProbeTimeout,
}

/// The interpreter a shell command runs under.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShellKind {
    #[default]
    Bash,
    PowerShell,
}

impl ShellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellKind::Bash => "bash",
            ShellKind::PowerShell => "powershell",
        }
    }
}

/// The resolved interpreter: a kind (so callers can adapt prompts) and the
/// executable to invoke. `path` empty means resolve lazily.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shell {
    pub kind: ShellKind,
    pub path: String,
    /// Launcher arguments that come between path and `-c` for a launcher
    /// such as wsl.exe. NUL-joined so Shell stays comparable.
    pub launcher_args: String,
    /// Set only when bash was wanted and could not be used.
    pub fallback: FallbackReason,
}

impl Shell {
    /// The argv that runs `command` under this shell.
    pub fn argv(&self, command: &str) -> Vec<String> {
        let path = if self.path.is_empty() {
            self.kind.as_str().to_owned()
        } else {
            self.path.clone()
        };
        if self.kind == ShellKind::PowerShell {
            return vec![
                path,
                String::from("-NoProfile"),
                String::from("-NonInteractive"),
                String::from("-Command"),
                power_shell_tool_script(&normalize_null_redirects(command, "$null")),
            ];
        }
        let mut argv = vec![path];
        argv.extend(self.args().iter().cloned());
        argv.push(String::from("-c"));
        argv.push(normalize_null_redirects(command, "/dev/null"));
        argv
    }

    /// Whether this shell parses `&&` / `||`. bash does; Windows PowerShell
    /// 5.1 does not — only PowerShell 7+ (pwsh).
    pub fn supports_chaining(&self) -> bool {
        if self.kind != ShellKind::PowerShell {
            return true;
        }
        let base = path_base(&self.path);
        base == "pwsh" || base == "pwsh.exe"
    }

    /// The launcher arguments this shell is started with, if any.
    pub fn args(&self) -> Vec<String> {
        if self.launcher_args.is_empty() {
            Vec::new()
        } else {
            self.launcher_args.split('\0').map(str::to_owned).collect()
        }
    }
}

/// Prepares a PowerShell script for captured execution (the user's own
/// scripts run through hooks keep their defaults).
pub fn power_shell_utf8_script(command: &str) -> String {
    format!("{PS_UTF8_PROLOGUE}{command}")
}

/// The agent's shell tool script: the prologue plus the 5.1 file defaults.
pub fn power_shell_tool_script(command: &str) -> String {
    format!("{PS_UTF8_PROLOGUE}{PS_TOOL_FILE_DEFAULTS}{command}")
}

fn path_base(p: &str) -> &str {
    match p.rfind(['/', '\\']) {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// ResolveShell: auto favours bash; on Windows without a usable bash it
/// falls back to PowerShell. A resolution that found nothing hands back the
/// bare `bash` name rather than an interpreter no probe confirmed.
pub fn resolve_shell() -> Shell {
    if let Some(bash) = resolve_bash() {
        return bash;
    }
    if cfg!(windows) {
        for name in ["pwsh", "powershell"] {
            if let Some(ps) = look_shell(name) {
                return Shell {
                    kind: ShellKind::PowerShell,
                    path: ps,
                    fallback: FallbackReason::NotFound,
                    ..Shell::default()
                };
            }
        }
    }
    Shell {
        kind: ShellKind::Bash,
        path: String::from("bash"),
        fallback: FallbackReason::None,
        ..Shell::default()
    }
}

/// PATH lookup held to the launch-place rule: a shell is not found on a
/// network share or device namespace.
fn look_shell(name: &str) -> Option<String> {
    let path = which(name)?;
    if launch_place(&path) {
        Some(path)
    } else {
        None
    }
}

fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Where a shell may be started from: off Windows any non-empty path; on
/// Windows an absolute local path. Network shares and device namespaces are
/// read from the spelling alone.
fn launch_place(p: &str) -> bool {
    if !cfg!(windows) {
        return !p.is_empty();
    }
    if is_windows_network_path(p) {
        return false;
    }
    is_drive_absolute(p) || std::path::Path::new(p).is_absolute()
}

fn is_drive_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/')
        && (b[0] as char).is_ascii_alphabetic()
}

/// Go `fileutil.IsNetworkPath` by spelling: two leading separators or an
/// NT `\??\` prefix, unless a drive follows.
fn is_windows_network_path(path: &str) -> bool {
    let b = path.as_bytes();
    if b.len() >= 4 && b[0] == b'\\' && b[1] == b'?' && b[2] == b'?' && b[3] == b'\\' {
        return !(b.len() >= 6 && b[5] == b':' && (b[4] as char).is_ascii_alphabetic());
    }
    b.len() >= 2 && b[0] == b'\\' && b[1] == b'\\'
}

/// The bash candidates a Git-for-Windows install ships, in detection order.
fn windows_bash_candidates() -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for env in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(v) = launch_place_env(env) {
            roots.push(format!("{v}\\Git"));
        }
    }
    if let Some(v) = launch_place_env("LOCALAPPDATA") {
        roots.push(format!("{v}\\Programs\\Git"));
    }
    if let Some(g) = which("git")
        && launch_place(&g)
        && let Some(dir) = std::path::Path::new(&g).parent()
        && let Some(root) = dir.parent()
    {
        roots.push(root.to_string_lossy().into_owned());
    }
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for r in &roots {
        for tail in ["\\bin\\bash.exe", "\\usr\\bin\\bash.exe"] {
            add_unique(format!("{r}{tail}"), &mut out, &mut seen);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            if let Some(s) = d.to_str()
                && launch_place(s)
            {
                add_unique(format!("{s}\\bash.exe"), &mut out, &mut seen);
            }
        }
    }
    out
}

fn add_unique(p: String, out: &mut Vec<String>, seen: &mut Vec<String>) {
    let key = p.to_lowercase();
    if !seen.contains(&key) {
        seen.push(key);
        out.push(p);
    }
}

fn launch_place_env(env: &str) -> Option<String> {
    let v = std::env::var(env).ok()?;
    if launch_place(&v) { Some(v) } else { None }
}

/// Windows ships a bash.exe launcher stub under %SystemRoot% that opens the
/// WSL install prompt instead of running anything; the only bash.exe
/// Microsoft places there is that launcher. Spelling answers first so a
/// wedged prompt costs no probe budget; the probe is the authority.
fn is_wsl_launcher_spelling(path: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let lower = path.to_lowercase();
    let Some(root) = std::env::var("SystemRoot")
        .ok()
        .or_else(|| std::env::var("windir").ok())
    else {
        return false;
    };
    let root = root.trim_end_matches(['\\', '/']).to_lowercase();
    ["system32", "syswow64"]
        .iter()
        .any(|dir| lower == format!("{root}\\{dir}\\bash.exe"))
}

/// One launch-and-run probe budget. A cold Git Bash starts in under a
/// second here, so a launch still silent after 5s is wedged, not slow.
const BASH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// A fresh process usually clears an MSYS start wedge, so one retry.
const BASH_PROBE_ATTEMPTS: u32 = 2;

/// Why a bash candidate failed its probe, per path.
fn probe_failures() -> &'static Mutex<HashMap<String, FallbackReason>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, FallbackReason>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn probe_bash_timed_out(path: &str) -> bool {
    probe_failures()
        .lock()
        .map(|m| matches!(m.get(path), Some(FallbackReason::ProbeTimeout)))
        .unwrap_or(false)
}

/// Proves a bash candidate actually runs a command before the session is
/// handed to it. Memoized per path; a plain failure is not retried.
fn probe_bash(path: &str) -> bool {
    if !cfg!(windows) {
        return true;
    }
    let mut why = FallbackReason::ProbeFailed;
    for _ in 0..BASH_PROBE_ATTEMPTS {
        why = run_probe_with_timeout(path, BASH_PROBE_TIMEOUT);
        if why != FallbackReason::ProbeTimeout {
            break;
        }
    }
    let ok = why == FallbackReason::None;
    if let Ok(mut m) = probe_failures().lock() {
        if ok {
            m.remove(path);
        } else {
            m.insert(path.to_owned(), why);
        }
    }
    ok
}

/// Runs the probe with its own timeout so a candidate stuck on the WSL
/// prompt answers as a timeout, not a hang.
fn run_probe_with_timeout(path: &str, timeout: Duration) -> FallbackReason {
    let mut cmd = std::process::Command::new(path);
    cmd.args(["-c", "true"]);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    hide_window(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return FallbackReason::ProbeFailed;
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return FallbackReason::None,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return FallbackReason::ProbeTimeout;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return FallbackReason::ProbeFailed,
        }
    }
}

/// Resolves a bash interpreter: the candidates in detection order, each
/// proven by a probe.
fn resolve_bash() -> Option<Shell> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = look_shell("bash") {
        candidates.push(p);
    }
    if cfg!(windows) {
        candidates.extend(windows_bash_candidates());
    }
    for path in candidates {
        if is_wsl_launcher_spelling(&path) || probe_bash_timed_out(&path) {
            continue;
        }
        if probe_bash(&path) {
            return Some(Shell {
                kind: ShellKind::Bash,
                path,
                ..Shell::default()
            });
        }
    }
    None
}

/// Stops a child process from flashing a console window on Windows, where a
/// GUI parent has no console of its own to inherit.
pub fn hide_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Rewrites null-device redirect aliases to `sink` ("/dev/null" for bash,
/// "$null" for PowerShell) so permission-approved null-sink commands
/// discard output under the resolved shell. Quoted and escaped text passes
/// through untouched. The input is UTF-8; bytes outside the ASCII
/// structural set are copied verbatim.
pub fn normalize_null_redirects(command: &str, sink: &str) -> String {
    let b = command.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut quote: u8 = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            out.push(c);
            i += 1;
            if c == b'\\' && quote == b'"' && i < b.len() {
                out.push(b[i]);
                i += 1;
                continue;
            }
            // PowerShell's escape, and only inside its double quotes:
            // single-quoted text is literal in both shells.
            if c == b'`' && quote == b'"' && i < b.len() {
                out.push(b[i]);
                i += 1;
                continue;
            }
            if c == quote {
                quote = 0;
            }
            continue;
        }
        match c {
            b'\'' | b'"' => {
                quote = c;
                out.push(c);
                i += 1;
            }
            b'\\' | b'`' => {
                out.push(c);
                i += 1;
                if i < b.len() {
                    out.push(b[i]);
                    i += 1;
                }
            }
            _ => match consume_null_redirect(b, i, sink) {
                Some((replacement, next)) => {
                    out.extend_from_slice(replacement.as_bytes());
                    i = next;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            },
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn consume_null_redirect(s: &[u8], start: usize, sink: &str) -> Option<(String, usize)> {
    let mut i = start;
    if i >= s.len() {
        return None;
    }
    if s[i] == b'&' {
        i += 1;
        if i < s.len() && s[i] == b'>' {
            i += 1;
            if i < s.len() && s[i] == b'>' {
                i += 1;
            }
        } else {
            return None;
        }
    } else {
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        if i >= s.len() || s[i] != b'>' {
            return None;
        }
        i += 1;
        if i < s.len() && s[i] == b'>' {
            i += 1;
        }
    }
    let op_end = i;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    let (next, ok) = consume_null_sink(s, i)?;
    if !ok {
        return None;
    }
    // The operator span is ASCII structural only (digits, spaces, `>`, `&`),
    // so the UTF-8 fold is lossless.
    let op = std::str::from_utf8(&s[start..op_end]).ok()?;
    Some((format!("{op}{sink}"), next))
}

fn consume_null_sink(s: &[u8], i: usize) -> Option<(usize, bool)> {
    for sink in ["/dev/null", "$null", "nul"] {
        let sb = sink.as_bytes();
        if i + sb.len() > s.len() {
            continue;
        }
        let got = &s[i..i + sb.len()];
        let matched = if sink == "/dev/null" {
            got == sb
        } else {
            got.eq_ignore_ascii_case(sb)
        };
        if !matched {
            continue;
        }
        let next = i + sb.len();
        if next < s.len() && !is_null_redirect_word_end(s[next]) {
            return Some((next, false));
        }
        return Some((next, true));
    }
    None
}

fn is_null_redirect_word_end(c: u8) -> bool {
    matches!(
        c,
        b' ' | b'\t' | b'\n' | b'\r' | b';' | b'&' | b'|' | b'<' | b'>' | b')' | b']'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_builds_per_kind() {
        let bash = Shell {
            kind: ShellKind::Bash,
            path: String::from("/bin/bash"),
            ..Shell::default()
        };
        assert_eq!(
            bash.argv("echo hi"),
            vec![
                String::from("/bin/bash"),
                String::from("-c"),
                String::from("echo hi")
            ]
        );
        let ps = Shell {
            kind: ShellKind::PowerShell,
            path: String::from("powershell"),
            ..Shell::default()
        };
        let argv = ps.argv("echo hi");
        assert_eq!(argv[0], "powershell");
        assert_eq!(argv[1], "-NoProfile");
        assert_eq!(argv[2], "-NonInteractive");
        assert_eq!(argv[3], "-Command");
        assert!(
            argv[4].starts_with("if($ExecutionContext"),
            "prologue rides first"
        );
        assert!(argv[4].ends_with("echo hi"));
        let empty = Shell::default();
        assert_eq!(empty.argv("x")[0], "bash");
    }

    #[test]
    fn chaining_follows_pwsh_only() {
        let bash = Shell {
            kind: ShellKind::Bash,
            path: String::from("/bin/bash"),
            ..Shell::default()
        };
        assert!(bash.supports_chaining());
        let ps51 = Shell {
            kind: ShellKind::PowerShell,
            path: String::from("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            ..Shell::default()
        };
        assert!(!ps51.supports_chaining());
        let pwsh = Shell {
            kind: ShellKind::PowerShell,
            path: String::from("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
            ..Shell::default()
        };
        assert!(pwsh.supports_chaining());
    }

    #[test]
    fn null_redirects_normalize_to_the_shell_sink() {
        assert_eq!(
            normalize_null_redirects("go test 2>nul", "/dev/null"),
            "go test 2>/dev/null"
        );
        assert_eq!(
            normalize_null_redirects("cmd >/dev/null 2>&1", "$null"),
            "cmd >$null 2>&1"
        );
        assert_eq!(
            normalize_null_redirects("cmd >$null", "/dev/null"),
            "cmd >/dev/null"
        );
        assert_eq!(
            normalize_null_redirects("cmd 2>NUL", "/dev/null"),
            "cmd 2>/dev/null"
        );
        assert_eq!(
            normalize_null_redirects("echo 'a > nul b'", "/dev/null"),
            "echo 'a > nul b'",
            "quoted text is literal"
        );
        assert_eq!(
            normalize_null_redirects("echo \"x > nul\"", "/dev/null"),
            "echo \"x > nul\"",
            "double-quoted text is literal"
        );
        assert_eq!(
            normalize_null_redirects("curl -o NUL.example", "/dev/null"),
            "curl -o NUL.example",
            "a longer name is not the sink"
        );
        assert_eq!(normalize_null_redirects("x > nul", "$null"), "x >$null");
        assert_eq!(
            normalize_null_redirects("git diff >/dev/null", "/dev/null"),
            "git diff >/dev/null"
        );
    }

    #[test]
    fn null_redirects_keep_non_ascii_verbatim() {
        let cmd = "Write-Output '参数' >$null";
        assert_eq!(normalize_null_redirects(cmd, "$null"), cmd);
        assert_eq!(
            normalize_null_redirects("echo 参数 > nul", "/dev/null"),
            "echo 参数 >/dev/null"
        );
    }

    #[test]
    fn launch_place_refuses_shares_and_keeps_local() {
        if cfg!(windows) {
            assert!(!launch_place("\\\\srv\\share\\bash.exe"));
            assert!(launch_place("C:\\Program Files\\Git\\bin\\bash.exe"));
            assert!(launch_place("C:\\Git\\usr\\bin\\bash.exe"));
            assert!(!launch_place("bash.exe"), "relative is not a place");
        } else {
            assert!(launch_place("/bin/bash"));
            assert!(!launch_place(""));
        }
    }

    #[test]
    fn wsl_launcher_spelling_is_excluded_on_windows() {
        if cfg!(windows) {
            assert!(is_wsl_launcher_spelling("C:\\Windows\\System32\\bash.exe"));
            assert!(!is_wsl_launcher_spelling("C:\\Git\\bin\\bash.exe"));
        } else {
            assert!(!is_wsl_launcher_spelling("/bin/bash"));
        }
    }
}
