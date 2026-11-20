//! Host-local metadata for one shell invocation (tools.md §1.6). Never part
//! of provider-visible schema or request bytes: serializers must strip it
//! before a request leaves the host.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::abi::{CallContext, ToolResult};

/// What the shell invocation is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellState {
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    BackgroundStarted,
    NotRun,
}

/// Where in the chain a failure happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellPhase {
    Preflight,
    Authorization,
    Dependency,
    Launch,
    Execution,
    Timeout,
    Cancellation,
}

/// What the run may have changed on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationRisk {
    None,
    NotStarted,
    MayHaveCompleted,
    MayBePartial,
    Unknown,
}

/// Whether the run carried a verification and what it said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    NotVerification,
    NotRun,
    Passed,
    Failed,
    Inconclusive,
}

/// Which shell program ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellName {
    #[serde(rename = "bash")]
    Bash,
    #[serde(rename = "git-bash")]
    GitBash,
    #[serde(rename = "powershell")]
    PowerShell,
    #[serde(rename = "pwsh")]
    Pwsh,
}

impl ShellName {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellName::Bash => "bash",
            ShellName::GitBash => "git-bash",
            ShellName::PowerShell => "powershell",
            ShellName::Pwsh => "pwsh",
        }
    }
}

/// PowerShell version labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellVersion {
    #[serde(rename = "5.1")]
    Ps51,
    #[serde(rename = "7+")]
    Ps7,
}

impl ShellVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellVersion::Ps51 => "5.1",
            ShellVersion::Ps7 => "7+",
        }
    }
}

/// Host-local metadata for one shell invocation. `kind` is always `"shell"`
/// so a UI can tell this payload from other future execution kinds without
/// guessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellExecution {
    /// Constant `"shell"`; a field, not an enum, because more kinds may come.
    #[serde(rename = "kind")]
    pub kind: ShellKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_version: Option<ShellVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Explicit even when false so UIs can show PowerShell 5.1 chaining
    /// limits without reading omission as "unknown".
    #[serde(default)]
    pub supports_and_and: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<ShellState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_phase: Option<ShellPhase>,
    /// Set only when a child process started and produced an exit status;
    /// zero is a valid successful code, distinct from unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Bounded tail of combined stdout+stderr, set only for a run that did
    /// not succeed. Both streams share one pipe so interleaving stays in
    /// child-write order. At most [`OUTPUT_TAIL_MAX_BYTES`]; never a shell
    /// executable path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tail: Option<String>,
    /// The command with leading env assignments dropped, set only when that
    /// changed it. Never a replacement for the command: authorization
    /// surfaces carry the full text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_risk: Option<MutationRisk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
    /// Each stage's exit status for a run the host asked bash to report,
    /// left to right. Empty when it did not ask, or the report did not
    /// survive the run — a pipeline's own exit code is only its last stage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pipe_status: Vec<i32>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub duration_ms: u64,
    /// The background job a background_started run became.
    #[serde(rename = "jobId", skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// The constant `kind` value of a [`ShellExecution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellKind {
    #[serde(rename = "shell")]
    Shell,
}

impl Default for ShellExecution {
    fn default() -> Self {
        Self {
            kind: ShellKind::Shell,
            shell: None,
            shell_version: None,
            platform: None,
            supports_and_and: false,
            state: None,
            failure_phase: None,
            exit_code: None,
            output_tail: None,
            subject: None,
            mutation_risk: None,
            verification: None,
            pipe_status: Vec::new(),
            duration_ms: 0,
            job_id: None,
        }
    }
}

/// Bounds the output tail retained on [`ShellExecution`].
pub const OUTPUT_TAIL_MAX_BYTES: usize = 16 << 10;

/// What a search built-in prints when it found nothing. One constant because
/// two sides need it: the tool writes it for the model, and the host reads it
/// to tell a search that came back empty from one that broke.
pub const NO_MATCHES: &str = "(no matches)";

/// Whether `name` is the host's shell tool — the tool whose results carry a
/// [`ShellExecution`]. The tool is registered as `bash` on every platform;
/// other shell names name the shell it runs, not the tool.
pub fn is_shell_tool(name: &str) -> bool {
    name == "bash"
}

/// The structured outcome of a detailed-executor call. Output stays the
/// model-visible text; execution is host/UI metadata only.
#[derive(Debug, Clone, Default)]
pub struct DetailedResult {
    pub output: String,
    pub images: Vec<String>,
    pub execution: Option<ShellExecution>,
}

/// An optional tool capability returning structured execution metadata
/// alongside the model-visible result text. Tools without it keep the plain
/// execute path.
pub trait DetailedExecutor {
    /// A descriptor for the would-be execution before the process starts
    /// (shell identity, platform, chaining support). Must not launch a
    /// process; args may be empty or invalid — return a best-effort
    /// descriptor from the bound shell configuration.
    fn execution_descriptor(&self, args: &Value) -> ShellExecution;
    /// Runs the tool and returns structured metadata. On policy/preflight
    /// blocks, execution must still be populated with state `NotRun`.
    fn execute_detailed(
        &self,
        ctx: &CallContext,
        args: &Value,
    ) -> Result<DetailedResult, ToolResult>;
}

/// Duration as the milliseconds `ShellExecution::duration_ms` carries.
pub fn duration_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
