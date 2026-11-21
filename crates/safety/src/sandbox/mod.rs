//! The sandbox seam the shell tool executes through: the spec describing how
//! one command is confined, the interpreter it runs under, and the typed
//! backend trait the OS-enforcement wave implements (parity target:
//! `internal/safety/sandbox`, execution-pipeline subset).
//!
//! The zero [`Spec`] is Go's zero value: mode empty, nothing enforced, the
//! command runs unwrapped. A spec asking for enforcement with no
//! [`OsSandbox`] backend wired stays refused by the caller — the same
//! fail-closed a Go host hits when the OS sandbox is unavailable — never a
//! silent unconfined run.

#![forbid(unsafe_code)]

pub mod shell;

pub use shell::{
    FallbackReason, Shell, ShellKind, normalize_null_redirects, power_shell_tool_script,
    power_shell_utf8_script, resolve_shell,
};

use std::collections::BTreeMap;
use std::sync::Arc;

/// The spec's mode value that asks for confinement.
pub const MODE_ENFORCE: &str = "enforce";

/// The temporary-directory variables a session-private temp exports. Go
/// carries the same triple in `sandbox.SessionTempEnvKeys`; the file tools'
/// `SESSION_TEMP_ENV_KEYS` is their tools-crate mirror.
pub const SESSION_TEMP_ENV_KEYS: [&str; 3] = ["TMPDIR", "TMP", "TEMP"];

/// How to confine one command (Go `sandbox.Spec`, the fields the execution
/// pipeline reads). The zero value does not enforce, so an unconfigured
/// caller runs commands unchanged.
#[derive(Default, Clone)]
pub struct Spec {
    /// [`MODE_ENFORCE`] wraps the command; anything else runs it unwrapped.
    pub mode: String,
    /// Directories the command may write to. Confinement itself is the OS
    /// backend's job; the spec only carries the roots to it.
    pub write_roots: Vec<String>,
    /// Allows external egress. Loopback stays open: a command must be able
    /// to reach its own listeners.
    pub network: bool,
    /// The host proxy external traffic goes through, set with `network`
    /// where the backend confines egress.
    pub egress: Option<Arc<dyn EgressRoute>>,
    /// Extra environment variables every bash command inherits, from
    /// `[tools.shell]` env; host-owned session and egress overrides win.
    pub shell_env: BTreeMap<String, String>,
    /// The interpreter the bash tool runs under. Empty resolves lazily.
    pub shell: Shell,
    /// Absolute path of the logical-session private temporary directory.
    /// Empty keeps the platform default.
    pub session_temp: String,
    /// The OS-enforcement backend. `None` is Go-nil: no OS-level enforcement
    /// exists or is configured, and an enforcing spec stays refused rather
    /// than running unconfined.
    pub engine: Option<Arc<dyn OsSandbox>>,
}

impl Spec {
    /// Whether the spec asks for confinement.
    pub fn enforce(&self) -> bool {
        self.mode == MODE_ENFORCE
    }
}

/// One launch the sandbox seam prepared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    /// The argv to spawn, interpreter and any OS wrapping included.
    pub argv: Vec<String>,
    /// Whether an OS sandbox wrapped the command. A caller asking for
    /// enforcement treats `false` as refused.
    pub wrapped: bool,
    /// The session-private temporary directory this launch bound.
    pub session_temp: String,
    /// KEY=value environment overrides the launch needs (session temp,
    /// egress proxy).
    pub env_overrides: Vec<String>,
    /// Linux bubblewrap bound the session temp at /tmp, so the temp
    /// variables point at the virtual path.
    pub linux_sandboxed: bool,
    /// The egress token this launch's traffic is accounted under; empty
    /// when the launch is not confined behind the egress proxy.
    pub egress_token: String,
}

/// The OS-enforcement backend (Go `sandbox.Command` per GOOS: seatbelt,
/// bubblewrap, the Windows write-restricted token). Implementations answer
/// `(argv, wrapped)`; `wrapped == false` under an enforcing spec means the
/// backend could not confine and the caller fails closed.
pub trait OsSandbox: Send + Sync {
    /// Wraps `launch` for `spec` under `sh`, or returns it unwrapped.
    fn command(&self, spec: &Spec, sh: &Shell, launch: &str) -> (Vec<String>, bool);

    /// Why an enforced sandbox cannot run on this host, with the
    /// platform-specific remediation. Backends without a finer fault report
    /// the generic refusal.
    fn unavailable_message(&self) -> String {
        unavailable_message()
    }
}

/// Resolves one launch through the seam. Off-spec runs unchanged; an
/// enforcing spec consults the backend, whose `wrapped == false` leaves the
/// refusal decision with the caller.
pub fn command(spec: &Spec, sh: &Shell, launch: &str) -> (Vec<String>, bool) {
    if !spec.enforce() {
        return (sh.argv(launch), false);
    }
    match &spec.engine {
        Some(engine) => engine.command(spec, sh, launch),
        None => (sh.argv(launch), false),
    }
}

/// The host proxy a confined command's external traffic goes through. A
/// command presents its token so refusals are charged to it.
pub trait EgressRoute: Send + Sync {
    /// The KEY=value variables pointing the command's clients at the proxy.
    fn env(&self, token: &str) -> Vec<String>;
    /// What was refused under token, ending it.
    fn refusals(&self, token: &str) -> Vec<String>;
}

/// The temporary-directory overrides for a session-private temp dir. When
/// `linux_sandboxed` is true (bubblewrap bound the dir at /tmp) the values
/// point at the virtual /tmp path; otherwise at the host directory. An
/// empty session temp yields no overrides.
pub fn session_temp_env(session_temp: &str, linux_sandboxed: bool) -> Vec<String> {
    if session_temp.is_empty() {
        return Vec::new();
    }
    let value = if linux_sandboxed && cfg!(target_os = "linux") {
        "/tmp"
    } else {
        session_temp
    };
    SESSION_TEMP_ENV_KEYS
        .iter()
        .map(|key| format!("{key}={value}"))
        .collect()
}

/// [`session_temp_env`] as a name-value map.
pub fn session_temp_env_map(session_temp: &str, linux_sandboxed: bool) -> BTreeMap<String, String> {
    session_temp_env(session_temp, linux_sandboxed)
        .into_iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect()
}

/// Why an enforced bash sandbox cannot run and the remediation. Without a
/// backend wired the host has no platform-specific fault to name, so the
/// message stays generic.
pub fn unavailable_message() -> String {
    "bash sandbox requested but unavailable on this host; refusing to run unconfined. \
     Set [sandbox] bash = \"off\" in config.toml / Settings -> Sandbox to run shell commands \
     unconfined, or install the OS sandbox backend for this platform."
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_spec_does_not_enforce() {
        let spec = Spec::default();
        assert!(!spec.enforce());
        let sh = Shell {
            kind: ShellKind::Bash,
            path: String::from("/bin/sh"),
            ..Shell::default()
        };
        let (argv, wrapped) = command(&spec, &sh, "echo hi");
        assert_eq!(argv, sh.argv("echo hi"));
        assert!(!wrapped);
    }

    #[test]
    fn enforcing_spec_without_a_backend_stays_unwrapped_and_refused_by_caller() {
        let spec = Spec {
            mode: String::from(MODE_ENFORCE),
            ..Spec::default()
        };
        let sh = Shell {
            kind: ShellKind::Bash,
            path: String::from("/bin/sh"),
            ..Shell::default()
        };
        let (argv, wrapped) = command(&spec, &sh, "echo hi");
        assert_eq!(argv, sh.argv("echo hi"));
        assert!(!wrapped, "no backend: the caller must fail closed");
    }

    #[test]
    fn session_temp_env_covers_the_standard_triple() {
        assert!(session_temp_env("", false).is_empty());
        let got = session_temp_env("/sess/tmp", false);
        assert_eq!(
            got,
            vec![
                String::from("TMPDIR=/sess/tmp"),
                String::from("TMP=/sess/tmp"),
                String::from("TEMP=/sess/tmp")
            ]
        );
        let map = session_temp_env_map("/sess/tmp", false);
        assert_eq!(map.get("TMPDIR").map(String::as_str), Some("/sess/tmp"));
    }
}
