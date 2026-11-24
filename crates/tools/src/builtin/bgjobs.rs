//! TOOL-026 background-job tools: `bash_output`, `kill_shell`, `wait` (Go
//! `internal/tools/builtin/bgjobs.go`). The tool shell owns the frozen
//! names, descriptions, schemas, refusal branches, and the pure rendering
//! (listing, labels, byte counts, progress phrasing). The job manager —
//! session-scoped table, incremental output cursors, artifact logs, watch
//! channels, completion drain, evidence leases — is the A3 jobs slice; this
//! module declares its seam and fails closed when the context carries none.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, ContextualReasoner, ContextualTool, SequentialTool,
    Tool, ToolResult,
};
use crate::builtin::bashrun::go_duration_text;
use crate::refusal::Refusal;

// ---------------------------------------------------------------------------
// Identities and frozen strings (Go `bgjobs.go:42-45`).
// ---------------------------------------------------------------------------

/// `jobs.no_job_context`: the turn carries no job manager.
pub const CODE_NO_JOB_CONTEXT: &str = "jobs.no_job_context";
/// The refusal sentence shared by all three background tools.
pub const NO_JOB_CONTEXT_MESSAGE: &str = "background jobs are not available in this context";

/// `bash_output`.
pub const BASH_OUTPUT_NAME: &str = "bash_output";
/// `kill_shell`.
pub const KILL_SHELL_NAME: &str = "kill_shell";
/// `wait`.
pub const WAIT_NAME: &str = "wait";

/// Go `bashOutput.Description()`, verbatim.
pub const BASH_OUTPUT_DESCRIPTION: &str = "Read new output from a background job started with bash(run_in_background=true) or task(run_in_background=true). Returns the output produced since the last bash_output call for that job, plus its status (running/done/failed/killed). Does not block. Omit job_id to list the jobs still running, with their kind, label and elapsed time.";

/// Go `killShell.Description()`, verbatim.
pub const KILL_SHELL_DESCRIPTION: &str = "Terminate a running background job (bash or task) started with run_in_background. A no-op if the job has already finished or the id is unknown.";

/// Go `waitJob.Description()`, verbatim.
pub const WAIT_DESCRIPTION: &str = "Block until background jobs finish, then return each job's status and final output/answer. Use to collect the result of a task(run_in_background) or bash(run_in_background) before continuing. Omit job_ids to wait for every running job.";

/// One listing's visible-job ceiling; the rest are counted, not shown.
pub const MAX_LISTED_JOBS: usize = 50;
/// One wait's blocking ceiling in seconds.
pub const WAIT_CEILING_SECONDS: i64 = 300;
/// Label line's rune budget before the ellipsis.
pub const MAX_LABEL_RUNES: usize = 80;

/// Sentinel for a read that found a job ended as failed (Go `errJobFailed`).
pub const ERR_JOB_FAILED: &str = "background job failed";

// ---------------------------------------------------------------------------
// Shared shapes (Go `jobs.Status`, `jobs.View`, `jobs.Result`, `jobs.Progress`).
// The manager implements [`JobManager`]; these are the values it returns.
// ---------------------------------------------------------------------------

/// A job's lifecycle state (Go `jobs.Status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Killed,
    Interrupted,
}

impl JobStatus {
    /// The lowercase status token the model reads.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Killed => "killed",
            JobStatus::Interrupted => "interrupted",
        }
    }

    /// Why a job reached a status nobody asked for; empty otherwise (Go
    /// `Status.Cause`).
    pub fn cause(self) -> &'static str {
        match self {
            JobStatus::Interrupted => {
                " — the session that ran it ended before it finished (a restart or close), not a kill; its output up to then is kept"
            }
            _ => "",
        }
    }
}

/// A running job as shown in the listing (Go `jobs.View`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobView {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// Epoch milliseconds the job started at.
    pub started_at_ms: i64,
}

/// Progress observed while a wait was blocked (Go `jobs.Progress`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JobProgress {
    /// How long the job has been running, epoch-millisecond free.
    pub running_ms: i64,
    /// Output bytes the job produced overall.
    pub produced: u64,
    /// Output bytes produced while this wait was blocked.
    pub delta: u64,
    /// Steps the sub-agent reported overall.
    pub steps: u64,
    /// Steps reported while this wait was blocked.
    pub step_delta: u64,
}

/// One reported job result (Go `jobs.Result`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: JobStatus,
    pub output: String,
    pub progress: Option<JobProgress>,
}

/// Why a wait returned (Go `jobs.WaitOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Finished,
    Matched,
    TimedOut,
    Cancelled,
}

/// The job manager seam (Go `jobs.Manager`). A3 supplies the real table,
/// cursors, and watch channels; every method here is the contract the tools
/// call through.
pub trait JobManager: Send + Sync {
    /// The session this manager's jobs belong to (Go `jobs.SessionFromContext`).
    fn session(&self) -> String;

    /// New output and status for one job; `found` is false for an unknown id
    /// (Go `OutputForSession`).
    fn output_for_session(&self, session: &str, job_id: &str) -> (String, JobStatus, bool);

    /// The jobs still running for the session (Go `RunningForSession`).
    fn running_for_session(&self, session: &str) -> Vec<JobView>;

    /// Stops a running job; false when the id is unknown or already finished
    /// (Go `KillForSession`). That false is a no-op, not an error.
    fn kill_for_session(&self, session: &str, job_id: &str) -> bool;

    /// Whether a running job resumes the turn on completion (Go
    /// `ResumesOnCompletion`).
    fn resumes_on_completion(&self, session: &str) -> bool;

    /// Blocks until the named jobs finish, the match pattern is hit, or the
    /// timeout fires (Go `WaitForSession`). An empty id set waits for every
    /// running job.
    fn wait_for_session(
        &self,
        session: &str,
        job_ids: &[String],
        timeout: Duration,
        match_pattern: Option<&str>,
    ) -> (Vec<JobResult>, WaitOutcome);
}

/// The manager bound to this call, if the composition root installed one (Go
/// `jobs.FromContext`).
pub fn job_manager_from(ctx: &CallContext) -> Option<Arc<dyn JobManager>> {
    ctx.value()?.downcast_ref::<Arc<dyn JobManager>>().cloned()
}

/// A tool-layer failure. The model-visible output collected before the failure
/// rides alongside the message, the way Go returns `(string, error)`.
#[derive(Debug)]
pub struct BgJobError {
    message: String,
    output: String,
}

impl BgJobError {
    fn plain(message: String) -> Self {
        Self {
            message,
            output: String::new(),
        }
    }

    /// The output the call had already collected, kept for the model.
    pub fn output(&self) -> &str {
        &self.output
    }
}

impl std::fmt::Display for BgJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BgJobError {}

// ---------------------------------------------------------------------------
// Pure rendering (Go `listRunningJobs`, `oneLine`, `byteCount`, `stillRunning`,
// `resumeNote`, `jobsFailed`, `filterLines`, `clampWaitSeconds`).
// ---------------------------------------------------------------------------

/// Collapses whitespace and truncates to `limit` runes with `...` (Go `oneLine`).
pub fn one_line(s: &str, limit: usize) -> String {
    let folded: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let runes: Vec<char> = folded.chars().collect();
    if runes.len() > limit {
        runes[..limit].iter().collect::<String>() + "..."
    } else {
        folded
    }
}

/// Renders a byte count in Go's unit ladder (Go `byteCount`): KB from 1024,
/// MB from `1<<20 - 51` so a one-decimal KB figure never rounds past 1024.0.
pub fn byte_count(n: u64) -> String {
    const MB: u64 = 1 << 20;
    const KB: u64 = 1 << 10;
    match n {
        n if n >= MB - 51 => format!("{:.1} MB", n as f64 / MB as f64),
        n if n >= KB => format!("{:.1} KB", n as f64 / KB as f64),
        n => format!("{n} B"),
    }
}

/// Renders what happened while the caller waited (Go `stillRunning`). No
/// thresholds and no verdict: a slow task and a stopped one differ by what
/// arrived, and the caller knows which its task should look like.
pub fn still_running(progress: Option<&JobProgress>) -> String {
    let Some(p) = progress else {
        return String::new();
    };
    let elapsed_secs = ((p.running_ms.max(0) + 500) / 1000) as u64;
    let mut parts = vec![format!(
        "running {}",
        go_duration_text(Duration::from_millis(elapsed_secs * 1000))
    )];
    if p.produced > 0 {
        parts.push(format!(
            "{} of output, {} of it while waiting",
            byte_count(p.produced),
            byte_count(p.delta)
        ));
    } else if p.steps > 0 {
        parts.push(format!(
            "no output text (it does not write here), {} step(s) reported, {} while waiting",
            p.steps, p.step_delta
        ));
    } else {
        parts.push("no output text and no steps reported yet".to_owned());
    }
    " — ".to_owned() + &parts.join(", ")
}

/// The host's resume promise for a running job, where the manager keeps it
/// (Go `resumeNote`). Without it a model that stops polling reports the job as
/// abandoned.
pub fn resume_note(manager: &dyn JobManager, session: &str) -> String {
    if !manager.resumes_on_completion(session) {
        return String::new();
    }
    "\n\nWhen a running job finishes, a new turn starts with its status, so you can end this turn instead of polling; the job keeps running and is not abandoned."
        .to_owned()
}

/// Lists the running jobs, newest-bound first per the manager's order (Go
/// `listRunningJobs`). An empty listing answers the no-jobs sentence.
pub fn list_running_jobs(views: &[JobView], now_ms: i64) -> String {
    if views.is_empty() {
        return "No background jobs are running.".to_owned();
    }
    let hidden = views.len().saturating_sub(MAX_LISTED_JOBS);
    let shown = &views[..views.len() - hidden];
    let mut b = format!("Running background jobs ({}):", views.len());
    for v in shown {
        let elapsed_ms = (now_ms - v.started_at_ms).max(0);
        let elapsed_secs = ((elapsed_ms + 500) / 1000) as u64;
        b.push_str(&format!(
            "\n- {}  kind={}  elapsed={}",
            v.id,
            v.kind,
            go_duration_text(Duration::from_millis(elapsed_secs * 1000))
        ));
        let label = one_line(&v.label, MAX_LABEL_RUNES);
        if !label.is_empty() {
            b.push_str(&format!("  {label}"));
        }
    }
    if hidden > 0 {
        b.push_str(&format!("\n{hidden} more running job(s) not listed."));
    }
    b
}

/// Keeps only the lines matching the regular expression (Go `filterLines`).
pub fn filter_lines(text: &str, pattern: &str) -> Result<String, regex::Error> {
    let rx = Regex::new(pattern)?;
    let keep: Vec<&str> = text.split('\n').filter(|line| rx.is_match(line)).collect();
    Ok(keep.join("\n"))
}

/// Clamps one wait's blocking seconds to the ceiling (Go `clampWaitSeconds`).
/// Omitting the timeout means "wait for the jobs", not "forever"; the clamp is
/// reported rather than applied silently.
pub fn clamp_wait_seconds(requested: i64) -> (i64, bool) {
    if requested <= 0 || requested > WAIT_CEILING_SECONDS {
        (WAIT_CEILING_SECONDS, true)
    } else {
        (requested, false)
    }
}

/// The call's failure when any reported job ended as failed, naming those
/// jobs (Go `jobsFailed`). `None` when every job is still running or finished.
pub fn jobs_failed(results: &[JobResult]) -> Option<BgJobError> {
    let failed: Vec<&str> = results
        .iter()
        .filter(|r| r.status == JobStatus::Failed)
        .map(|r| r.id.as_str())
        .collect();
    if failed.is_empty() {
        return None;
    }
    Some(BgJobError::plain(format!(
        "{ERR_JOB_FAILED}: {}",
        failed.join(", ")
    )))
}

/// Collects a finished job's evidence lease (Go `collectBackgroundEvidence`).
/// Plan-mode skip, the evidence ledger lookup, the readiness probe, and the
/// lease/merge ride with the evidence slice; the seam is called here so the
/// A3/evidence wiring has one attachment point.
/// TODO(wave38-a2-evidence-lease): implement plan-mode skip, ledger lease, and
/// child-evidence merge once the evidence context seam exists.
fn collect_evidence(_ctx: &CallContext, _manager: &dyn JobManager, _job_id: &str) {}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tool argument shapes.
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(default)]
struct OutputArgs {
    job_id: String,
    filter: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct KillArgs {
    job_id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct WaitArgs {
    job_ids: Vec<String>,
    timeout_seconds: i64,
    until_output_matches: String,
}

fn invalid_args(err: serde_json::Error) -> BgJobError {
    BgJobError::plain(format!("invalid args: {err}"))
}

fn no_manager_error() -> BgJobError {
    BgJobError::plain(NO_JOB_CONTEXT_MESSAGE.to_owned())
}

// ---------------------------------------------------------------------------
// Capabilities: the three tools are visible only when the context carries a
// manager; `bash_output` and `wait` run sequentially even though they are
// read-only, because they read what an earlier call in the same reply started.
// ---------------------------------------------------------------------------

struct JobContextual;
impl Capability for JobContextual {}
impl ContextualTool for JobContextual {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        job_manager_from(ctx).is_some()
    }
}

struct JobReasoner;
impl Capability for JobReasoner {}
impl ContextualTool for JobReasoner {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        job_manager_from(ctx).is_some()
    }
}
impl ContextualReasoner for JobReasoner {
    fn unavailable(&self, _ctx: &CallContext) -> Refusal {
        Refusal::code(CODE_NO_JOB_CONTEXT, NO_JOB_CONTEXT_MESSAGE)
    }
}

struct JobSequential;
impl Capability for JobSequential {}
impl SequentialTool for JobSequential {
    fn sequential(&self, _ctx: &CallContext, _args: &Value) -> bool {
        true
    }
}

fn contextual_caps(sequential: bool) -> CapabilitySet {
    let mut caps = CapabilitySet::default()
        .with(
            "contextual",
            Arc::new(JobContextual) as Arc<dyn ContextualTool>,
        )
        .with(
            "contextual_reasoner",
            Arc::new(JobReasoner) as Arc<dyn ContextualReasoner>,
        );
    if sequential {
        caps = caps.with(
            "sequential",
            Arc::new(JobSequential) as Arc<dyn SequentialTool>,
        );
    }
    caps
}

// ---------------------------------------------------------------------------
// `bash_output`: non-blocking poll of one job's new output, or the running
// list when no id is given.
// ---------------------------------------------------------------------------

/// The `bash_output` built-in.
#[derive(Debug, Default, Clone, Copy)]
pub struct BashOutputTool;

impl Tool for BashOutputTool {
    fn name(&self) -> &str {
        BASH_OUTPUT_NAME
    }

    fn description(&self) -> &str {
        BASH_OUTPUT_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(
            r#"{"type":"object","properties":{"job_id":{"type":"string","description":"The background job id (e.g. \"bash-1\") returned when it was started."},"filter":{"type":"string","description":"Optional regular expression; only matching lines of the new output are returned."}}}"#,
        )
        .expect("the bash_output schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let p: OutputArgs = serde_json::from_value(args.clone()).map_err(invalid_args)?;
        let Some(jm) = job_manager_from(ctx) else {
            return Err(Box::new(no_manager_error()));
        };
        let session = jm.session();

        if p.job_id.is_empty() {
            let views = jm.running_for_session(&session);
            return Ok(list_running_jobs(&views, now_ms()));
        }

        let (text, status, found) = jm.output_for_session(&session, &p.job_id);
        if !found {
            return Err(Box::new(BgJobError::plain(format!(
                "no background job {:?}",
                p.job_id
            ))));
        }
        if status != JobStatus::Running {
            collect_evidence(ctx, &*jm, &p.job_id);
        }
        let text = if !p.filter.is_empty() && !text.is_empty() {
            match filter_lines(&text, &p.filter) {
                Ok(filtered) => filtered,
                Err(err) => {
                    return Err(Box::new(BgJobError::plain(format!(
                        "invalid filter regexp: {err}"
                    ))));
                }
            }
        } else {
            text
        };

        let header = format!("[{}] {}{}", p.job_id, status.as_str(), status.cause());
        let mut body = if text.trim().is_empty() {
            "(no new output)".to_owned()
        } else {
            text
        };
        if status == JobStatus::Running {
            body.push_str(&resume_note(&*jm, &session));
        }

        let results = vec![JobResult {
            id: p.job_id.clone(),
            kind: String::new(),
            label: String::new(),
            status,
            output: String::new(),
            progress: None,
        }];
        let output = format!("{header}\n{body}");
        match jobs_failed(&results) {
            Some(mut err) => {
                err.output = output;
                Err(Box::new(err))
            }
            None => Ok(output),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        contextual_caps(true)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// `kill_shell`: terminate a running job. Unknown or already finished is a
// no-op success.
// ---------------------------------------------------------------------------

/// The `kill_shell` built-in.
#[derive(Debug, Default, Clone, Copy)]
pub struct KillShellTool;

impl Tool for KillShellTool {
    fn name(&self) -> &str {
        KILL_SHELL_NAME
    }

    fn description(&self) -> &str {
        KILL_SHELL_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(
            r#"{"type":"object","properties":{"job_id":{"type":"string","description":"The background job id to terminate (e.g. \"bash-1\")."}},"required":["job_id"]}"#,
        )
        .expect("the kill_shell schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let p: KillArgs = serde_json::from_value(args.clone()).map_err(invalid_args)?;
        if p.job_id.is_empty() {
            return Err(Box::new(BgJobError::plain("job_id is required".to_owned())));
        }
        let Some(jm) = job_manager_from(ctx) else {
            return Err(Box::new(no_manager_error()));
        };
        let session = jm.session();
        if jm.kill_for_session(&session, &p.job_id) {
            Ok(format!("Killed background job {:?}.", p.job_id))
        } else {
            Ok(format!(
                "Background job {:?} was not running (already finished or unknown).",
                p.job_id
            ))
        }
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        contextual_caps(false)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// `wait`: block until jobs finish, then report each status and final output.
// ---------------------------------------------------------------------------

/// The `wait` built-in.
#[derive(Debug, Default, Clone, Copy)]
pub struct WaitJobTool;

impl Tool for WaitJobTool {
    fn name(&self) -> &str {
        WAIT_NAME
    }

    fn description(&self) -> &str {
        WAIT_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(
            r#"{"type":"object","properties":{"job_ids":{"type":"array","items":{"type":"string"},"description":"Background job ids to wait for. Omit to wait for every currently-running job."},"timeout_seconds":{"type":"integer","description":"Seconds to block before returning. Every wait is capped at 300, including when this is omitted. On timeout the result carries progress; read it, then wait again.","minimum":1,"maximum":300},"until_output_matches":{"type":"string","description":"Optional regular expression (RE2). Return early as soon as a job writes something matching it. Use it to wait once for the line you actually care about — a completion marker, or an error — instead of polling to check."}}}"#,
        )
        .expect("the wait schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        // An empty argument envelope is a permitted wait for every running job.
        let p: WaitArgs = if args.is_null() {
            WaitArgs::default()
        } else {
            serde_json::from_value(args.clone()).map_err(invalid_args)?
        };
        let Some(jm) = job_manager_from(ctx) else {
            return Err(Box::new(no_manager_error()));
        };
        let session = jm.session();

        let (wait_seconds, capped) = clamp_wait_seconds(p.timeout_seconds);
        let match_pattern = p.until_output_matches.trim();
        let match_pattern = if match_pattern.is_empty() {
            None
        } else {
            match Regex::new(match_pattern) {
                Ok(_) => Some(match_pattern.to_owned()),
                Err(err) => {
                    return Err(Box::new(BgJobError::plain(format!(
                        "until_output_matches is not a valid regular expression: {err}"
                    ))));
                }
            }
        };

        let (results, outcome) = jm.wait_for_session(
            &session,
            &p.job_ids,
            Duration::from_secs(wait_seconds.max(0) as u64),
            match_pattern.as_deref(),
        );
        if results.is_empty() {
            return Ok("No background jobs to wait for.".to_owned());
        }

        let mut b = String::new();
        // Only a wait that actually reached the ceiling is described as having
        // waited it: a finished job returns first, and saying otherwise would
        // report a wait that never happened.
        if capped && outcome == WaitOutcome::TimedOut {
            b.push_str(&format!(
                "Waited {wait_seconds}ds, the ceiling for one wait; ask again to keep waiting.\n\n"
            ));
        }
        if outcome == WaitOutcome::Matched {
            b.push_str("Returned early: a job wrote something matching until_output_matches.\n\n");
        }

        let mut running = false;
        for (i, r) in results.iter().enumerate() {
            if r.status != JobStatus::Running {
                collect_evidence(ctx, &*jm, &r.id);
            } else {
                running = true;
            }
            if i > 0 {
                b.push_str("\n\n");
            }
            let label = if r.label.is_empty() {
                r.id.clone()
            } else {
                format!("{} ({})", r.id, r.label)
            };
            b.push_str(&format!(
                "[{label}] {}{}",
                r.status.as_str(),
                r.status.cause()
            ));
            b.push_str(&still_running(r.progress.as_ref()));
            if !r.output.trim().is_empty() {
                b.push('\n');
                b.push_str(&r.output);
            }
        }
        if running {
            b.push_str(&resume_note(&*jm, &session));
        }

        match jobs_failed(&results) {
            Some(mut err) => {
                err.output = b;
                Err(Box::new(err))
            }
            None => Ok(b),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        contextual_caps(true)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The three background-job tools a composition root registers together (Go
/// `init`'s three `RegisterBuiltin` calls).
pub fn bgjobs_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BashOutputTool),
        Arc::new(KillShellTool),
        Arc::new(WaitJobTool),
    ]
}
