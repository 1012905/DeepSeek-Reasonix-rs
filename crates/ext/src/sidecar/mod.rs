//! The host half of Extension Protocol v2: spawn, supervise, and stop sidecars.
//!
//! Parity target: `internal/ext/extension/sidecar` (contract §14). This slice
//! owns process launch and supervision, the initialize handshake gate, and the
//! bounded teardown; the dispatch chain, UI hosting, and provider routing are
//! later slices.
//!
//! Authorization invariant: a sidecar is only ever launched for a package the
//! installed state (`<Reasonix home>/plugin-packages.json`) records as enabled
//! with a `runtime` block. Every entry point here takes a home directory or a
//! package resolved from it — there is no way to point the manager at an
//! arbitrary binary, so a project configuration cannot declare a runtime.

pub mod client;
pub mod installed;
pub mod manager;
pub mod process;

pub use client::{
    Client, ClientError, ClientOptions, DEFAULT_HANDSHAKE_TIMEOUT,
    DEFAULT_SHUTDOWN_REQUEST_TIMEOUT, DEFAULT_WRITE_STALL_BOUND, QUEUED_NOTIFICATIONS,
    UNAVAILABLE_UI_MESSAGE, method,
};
pub use installed::{
    CapabilityRef, InstallStateError, InstalledPlugin, InstalledRuntimePackage, InstalledState,
    MANIFEST_API_VERSION_V2, NATIVE_MANIFEST, RuntimeEnv, RuntimeManifest, RuntimePackages,
    RuntimeSpec, RuntimeTool, STATE_FILE, load_installed_state, load_runtime_packages,
    resolve_root,
};
pub use process::{
    CLOSE_WAIT, ENV_PLUGIN_NAME, ENV_PLUGIN_ROOT, ENV_PLUGIN_VERSION, GRACEFUL_CLOSE_WAIT,
    PLUGIN_ROOT_VAR, STDERR_TAIL_BYTES, SidecarProcess, SpawnError, TailBuffer,
    resolve_runtime_command, runtime_env,
};

use std::time::Duration;

use crate::redact::redact_error_text;

/// Bounded diagnostics for a sidecar that failed to spawn or handshake.
///
/// Only ever carries the redacted stderr tail, never the raw bytes: this is the
/// text that reaches a user-facing error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    pub stage: &'static str,
    pub elapsed_ms: u64,
    pub stderr: String,
    pub detail: String,
}

impl StartupFailure {
    /// "startup <stage> failed after <n>ms: <detail>; stderr: <tail>".
    pub fn message(&self) -> String {
        let stage = if self.stage.trim().is_empty() {
            "unknown"
        } else {
            self.stage.trim()
        };
        let mut message = format!(
            "extension sidecar startup {stage} failed after {}: {}",
            format_elapsed(self.elapsed_ms),
            self.detail
        );
        let stderr = self.stderr.trim();
        if !stderr.is_empty() {
            message.push_str("; stderr: ");
            message.push_str(stderr);
        }
        message
    }
}

impl std::fmt::Display for StartupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for StartupFailure {}

/// Formats an elapsed budget the way the Go diagnostics do.
pub fn format_elapsed(millis: u64) -> String {
    if millis < 1000 {
        format!("{millis}ms")
    } else {
        format!("{:.3}s", millis as f64 / 1000.0)
    }
}

/// Builds a startup failure, bounding and redacting the diagnostics.
pub fn startup_failure(
    stage: &'static str,
    elapsed: Duration,
    stderr: String,
    detail: String,
) -> StartupFailure {
    StartupFailure {
        stage,
        elapsed_ms: elapsed.as_millis() as u64,
        stderr: redact_error_text(&stderr),
        detail: redact_error_text(&detail),
    }
}

pub use manager::{
    MAX_CONCURRENT_PACKAGE_STARTS, Manager, ManagerError, PACKAGE_STARTUP_BUDGET, StartOutcome,
};

/// Starts every enabled runtime package beneath `home`, in parallel.
///
/// Exposed for callers that already hold a home directory rather than a
/// manager: it is a thin wrapper over `Manager::start_packages`.
pub fn start_packages(
    home: &std::path::Path,
    session: crate::lifecycle::SessionContext,
    options: &ClientOptions,
) -> Result<StartOutcome, ManagerError> {
    Manager::start_packages(home, session, options)
}
