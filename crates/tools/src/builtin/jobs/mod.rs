//! TOOL-046/047/048 background jobs manager slice (Go `internal/tools/jobs/`).
//!
//! Done here: the status vocabulary and mappings, the in-memory job table and
//! insertion order, start validation and the invalid-job registration path,
//! kill/status flip, the completion ordering invariants (queue before flip,
//! close done before observer publish), the evidence lease/commit gates, the
//! sink/recorder/observer seams, and the call-context binding.
//!
//! WIP behind these seams: real process spawn and the wait-group lifecycle,
//! artifact file creation with 0700/0600 permissions, temp-file + rename meta
//! writes, incremental offset reads, directory migration and recovery repair,
//! the stall timer and watch/echo pump goroutines, teardown wait and temp-root
//! cleanup. The decision tables feeding those paths are locked in the sibling
//! modules.

#![forbid(unsafe_code)]

pub mod artifacts;
pub mod completion;
pub mod echo;
pub mod evidence;
pub mod finalization;
pub mod lifecycle;
pub mod views;
pub mod wait;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reasonix_contract::eventwire::notice_payload::JobNotice;

use crate::abi::CallContext;

pub use artifacts::{
    ArtifactMeta, DEFAULT_TAIL_BYTES, JOB_LOG_EXT, JOB_META_EXT, JobTimes,
    MUTATION_EVIDENCE_VERSION, MetaMutationEvidence, RECOVERED_BACKGROUND_TOOL_NAME, append_tail,
    persisted_artifact_complete, since_ms,
};
pub use completion::{
    CompletionEvent, CompletionInput, CompletionObserver, CompletionQueueItem, NoticeDescriptor,
    ObserverSlot, ack_item, completion_event_id, completion_note, record_completion,
    resumes_on_completion,
};
pub use echo::{ECHO_CAPPED, ECHO_LIMIT, ECHO_OPEN_LIMIT, ECHO_SEED, Echo, complete_runes};
pub use evidence::{
    ChildEvidenceSummary, ChildEvidenceSummary as EvidenceSummary, CommitOutcome, EvidenceReceipt,
    LeaseOutcome, MutationRisk, commit_decision, pending_evidence, recover_from_meta,
    recovered_task_mutation_receipt, try_lease,
};
pub use finalization::{
    ERR_FINALIZATION, ExitError, ExitInfo, Finalization, FinalizationError, INTERNAL_PANIC_PREFIX,
    JobFinalizers, PANIC_PREFIX, exit_of, is_finalization, panic_cause,
};
pub use lifecycle::{
    JobsValidationError, ReserveDecision, cancelled_status, classify_finish, completion_tag,
    now_ms, preserve_published_status, quiet_seconds, reserve_decision, stall_work_text,
    stalled_drain_text, stalled_notice_text, started_notice_text, validate_path_segment,
    validate_trusted_session_path,
};
pub use views::{
    MAX_DESCRIPTION_RUNES, MAX_DETAIL_RUNES, MAX_TAIL_RUNES, Peek, Presentation, View,
    backfill_exit_code, bounded, byte_count, first_rune_start, last_line, max0, one_line,
    peek_from_tail, still_running,
};
pub use wait::{
    OutputWatch, ResolveRef, WATCH_CARRY, WaitOptions, WaitOutcome, delta, last_bytes, resolve,
    session_matches,
};

/// `DefaultTeardownGrace` (`jobs.go:47`).
pub const DEFAULT_TEARDOWN_GRACE: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Status vocabulary (status.go, process.go)
// ---------------------------------------------------------------------------

/// The five job states. `Running` includes a job that was cancelled but whose
/// goroutine has not unwound yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Status {
    #[default]
    Running,
    Done,
    Failed,
    Killed,
    Interrupted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Killed => "killed",
            Self::Interrupted => "interrupted",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Option<Self> {
        Some(match raw {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "killed" => Self::Killed,
            "interrupted" => Self::Interrupted,
            _ => return None,
        })
    }

    /// `Cause`: only `Interrupted` carries the verbatim session-ended suffix.
    pub fn cause(self) -> &'static str {
        match self {
            Self::Interrupted => {
                " — the session that ran it ended before it finished (a restart or close), not a kill; its output up to then is kept"
            }
            _ => "",
        }
    }

    /// `ProcessClass` projection into the shared process vocabulary; the bool
    /// is false when the status has no mapping.
    pub fn process_class(self) -> (&'static str, bool) {
        match self {
            Self::Running => ("running", true),
            Self::Done => ("completed", true),
            Self::Failed => ("failed", true),
            Self::Killed => ("stopped", true),
            Self::Interrupted => ("interrupted", true),
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Result / progress / teardown shapes
// ---------------------------------------------------------------------------

/// One job's progress reading for a wait or view snapshot (`progress.go`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub running_ms: i64,
    pub silent_ms: i64,
    pub spoke: bool,
    pub produced: i64,
    pub delta: i64,
    pub steps: i64,
    pub step_delta: i64,
}

/// A read result handed back to the wrapper layer (`jobs.go:50-58`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobResult {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: Status,
    pub output: String,
    pub progress: Option<Progress>,
}

/// A job that did not exit inside the teardown grace (`jobs.go:61-66`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeardownJob {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub waited_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeardownResult {
    pub timed_out: Vec<TeardownJob>,
}

impl TeardownResult {
    pub fn has_timed_out(&self) -> bool {
        !self.timed_out.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Sink / recorder seams
// ---------------------------------------------------------------------------

/// Notice sink the manager emits coded notices through. The host supplies
/// the real sink; a [`DiscardSink`] stands in for nil (`event.Discard`).
pub trait JobSink: Send + Sync {
    fn emit(&self, code: &str, level: &str, text: &str, detail: &str);
}

/// Nil sink: drops every notice (`event.Discard`).
pub struct DiscardSink;

impl JobSink for DiscardSink {
    fn emit(&self, _code: &str, _level: &str, _text: &str, _detail: &str) {}
}

/// Best-effort per-job task recorder. `record_start` runs on the caller
/// goroutine, `record_done` on the job goroutine; implementations must not
/// block or fail the job pipeline (`jobs.go:178-181`).
pub trait TaskRecorder: Send + Sync {
    fn record_start(&self, id: &str, kind: &str, label: &str);
    fn record_done(&self, id: &str, status: Status, err: Option<&str>);
}

// ---------------------------------------------------------------------------
// Keying and identity
// ---------------------------------------------------------------------------

/// `jobKey`: trimmed session plus trimmed id, NUL-joined (`jobs.go:1643-1645`).
pub fn job_key(session: &str, id: &str) -> String {
    format!("{}\x00{}", session.trim(), id.trim())
}

/// Process-level manager owner id: 16 random hex bytes; on random failure
/// fall back to `pid-nanosecond-seq` (`jobs.go:239-245`).
pub fn new_owner_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_ok() {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        OWNER_FALLBACK_SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

static OWNER_FALLBACK_SEQ: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------
// Job
// ---------------------------------------------------------------------------

struct JobInner {
    tail: Vec<u8>,
    written: i64,
    /// Incremental reader offset; the artifact file branch that advances it
    /// is WIP (matrix §6.2).
    #[allow(dead_code)]
    read_offset: i64,
    status: Status,
    result: String,
    result_read: bool,
    times: JobTimes,
    run_returned: bool,
    stalled: bool,
    exit_code: Option<i32>,
    signal: i32,
    artifact_err: String,
    artifact_complete: bool,
    artifact_path: String,
    last: String,
    evidence_committed: bool,
    evidence: Option<ChildEvidenceSummary>,
    watches: Vec<Arc<OutputWatch>>,
}

impl JobInner {
    fn new(status: Status, times: JobTimes) -> Self {
        Self {
            tail: Vec::new(),
            written: 0,
            read_offset: 0,
            status,
            result: String::new(),
            result_read: false,
            times,
            run_returned: false,
            stalled: false,
            exit_code: None,
            signal: 0,
            artifact_err: String::new(),
            artifact_complete: false,
            artifact_path: String::new(),
            last: String::new(),
            evidence_committed: false,
            evidence: None,
            watches: Vec::new(),
        }
    }
}

/// One background job. Identity fields are public; stream and terminal state
/// sit behind the job lock, mirroring the two-level `m.mu` / `j.mu` design.
pub struct Job {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub session_id: String,
    pub presentation: Presentation,
    done: Arc<AtomicBool>,
    inner: Mutex<JobInner>,
}

impl Job {
    fn new(
        id: String,
        kind: String,
        label: String,
        session_id: String,
        presentation: Presentation,
        status: Status,
        times: JobTimes,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            kind,
            label,
            session_id,
            presentation,
            done: Arc::new(AtomicBool::new(false)),
            inner: Mutex::new(JobInner::new(status, times)),
        })
    }

    pub fn status(&self) -> Status {
        self.inner.lock().expect("job lock").status
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn written(&self) -> i64 {
        self.inner.lock().expect("job lock").written
    }

    pub fn steps(&self) -> i64 {
        self.inner.lock().expect("job lock").times.steps
    }

    pub fn tail_clone(&self) -> Vec<u8> {
        self.inner.lock().expect("job lock").tail.clone()
    }

    pub fn artifact_err(&self) -> String {
        self.inner.lock().expect("job lock").artifact_err.clone()
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.inner.lock().expect("job lock").exit_code
    }

    /// `artifactRefLocked`: the log path only when the artifact survived
    /// cleanly; empty otherwise.
    pub fn result_ref(&self) -> String {
        let inner = self.inner.lock().expect("job lock");
        if persisted_artifact_complete(inner.artifact_complete, &inner.artifact_path) {
            inner.artifact_path.clone()
        } else {
            String::new()
        }
    }

    /// The job's own last line, kept for the ended view tail.
    pub fn last_line(&self) -> String {
        self.inner.lock().expect("job lock").last.clone()
    }

    pub fn is_stalled(&self) -> bool {
        self.inner.lock().expect("job lock").stalled
    }

    /// `jobWriter.Write`: touch activity, count bytes, keep the bounded tail,
    /// notify watches, hand the bytes to the artifact writer (WIP).
    pub fn write(&self, p: &[u8], now_ms: i64) {
        let mut inner = self.inner.lock().expect("job lock");
        inner.times.touch(now_ms);
        inner.written += p.len() as i64;
        append_tail(&mut inner.tail, p, DEFAULT_TAIL_BYTES);
        let watches = inner.watches.clone();
        drop(inner);
        for watch in watches {
            watch.notify(p);
        }
    }

    /// Attaches a watch. Seed matching is evaluated by [`OutputWatch::new`].
    pub fn attach_watch(&self, watch: Arc<OutputWatch>) {
        self.inner.lock().expect("job lock").watches.push(watch);
    }

    /// In-memory output read: tail bytes plus the one-shot result passthrough,
    /// with the incomplete-artifact suffix (`jobs.go:697-727`).
    pub fn take_output(&self) -> (String, Status) {
        let mut inner = self.inner.lock().expect("job lock");
        let mut text = String::from_utf8_lossy(&inner.tail).into_owned();
        if text.is_empty()
            && !inner.status.is_terminal()
            && !inner.result.is_empty()
            && !inner.result_read
        {
            text = inner.result.clone();
            inner.result_read = true;
        }
        if !inner.artifact_err.is_empty() {
            text = format!("{text}\njob artifact incomplete: {}", inner.artifact_err);
        }
        (text, inner.status)
    }

    fn record_exit(&self, exit: Option<ExitInfo>) {
        let mut inner = self.inner.lock().expect("job lock");
        if let Some(exit) = exit
            && inner.exit_code.is_none()
        {
            inner.exit_code = Some(exit.code);
            inner.signal = exit.signal;
        }
    }

    fn record_error(&self, err: &str) {
        let mut inner = self.inner.lock().expect("job lock");
        if inner.artifact_err.is_empty() {
            inner.artifact_err = err.to_owned();
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ManagerInner {
    jobs: HashMap<String, Arc<Job>>,
    order: Vec<String>,
    completed: Vec<CompletionQueueItem>,
    active: String,
    destroying: HashSet<String>,
    reservations: HashMap<String, u32>,
    loaded: HashSet<String>,
}

/// The background job table. Concurrency mirrors Go's two-level lock:
/// `ManagerInner` guards the table and queues; [`Job`] guards its own stream
/// and terminal fields.
pub struct Manager {
    owner_id: String,
    sink: Option<Arc<dyn JobSink>>,
    observer: Mutex<ObserverSlot>,
    recorder: Mutex<Option<Arc<dyn TaskRecorder>>>,
    stalled_warning_ms: i64,
    teardown_grace: Duration,
    root_cancelled: AtomicBool,
    seq: AtomicU64,
    inner: Mutex<ManagerInner>,
}

impl Default for Manager {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Manager {
    /// `NewManager`: nil sink becomes the discard sink; owner id is registered
    /// in the process-level live-owner table (WIP: that registry lives with
    /// the host runtime).
    pub fn new(sink: Option<Arc<dyn JobSink>>) -> Self {
        Self {
            owner_id: new_owner_id(),
            sink,
            observer: Mutex::new(ObserverSlot::default()),
            recorder: Mutex::new(None),
            stalled_warning_ms: 0,
            teardown_grace: DEFAULT_TEARDOWN_GRACE,
            root_cancelled: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            inner: Mutex::new(ManagerInner::default()),
        }
    }

    /// `WithStalledWarningAfter`: a non-positive value disables the stall
    /// monitor.
    pub fn with_stalled_warning(mut self, warning: Option<Duration>) -> Self {
        self.stalled_warning_ms = warning.map(|d| d.as_millis() as i64).unwrap_or(0);
        self
    }

    pub fn set_task_recorder(&self, recorder: Option<Arc<dyn TaskRecorder>>) {
        *self.recorder.lock().expect("recorder lock") = recorder;
    }

    pub fn set_completion_observer(&self, observer: Option<Arc<dyn CompletionObserver>>) {
        self.observer.lock().expect("observer lock").set(observer);
    }

    pub fn teardown_grace(&self) -> Duration {
        self.teardown_grace
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn stalled_warning_ms(&self) -> i64 {
        self.stalled_warning_ms
    }

    pub fn set_active_session(&self, session: &str) {
        self.inner.lock().expect("manager lock").active = session.trim().to_owned();
    }

    pub fn active_session(&self) -> String {
        self.inner.lock().expect("manager lock").active.clone()
    }

    /// Whether session artifacts have been loaded (recovery path WIP).
    pub fn is_session_loaded(&self, session: &str) -> bool {
        self.inner
            .lock()
            .expect("manager lock")
            .loaded
            .contains(session.trim())
    }

    /// Cancels the manager root: jobs still launching read this as
    /// `Interrupted` rather than `Killed`. Real cancellation propagation to
    /// run closures is WIP.
    pub fn cancel_root(&self) {
        self.root_cancelled.store(true, Ordering::Release);
    }

    pub fn is_root_cancelled(&self) -> bool {
        self.root_cancelled.load(Ordering::Acquire)
    }

    // -- Start path ---------------------------------------------------------

    /// `StartForSession`: validate, then either register the visible Failed
    /// placeholder or a Running job. Real artifact open and run closure spawn
    /// are WIP; the table registration and notice/recorder routing are real.
    pub fn start(
        self: &Arc<Self>,
        parent_session: &str,
        kind: &str,
        label: &str,
        presentation: Presentation,
        now_ms: i64,
    ) -> Result<Arc<Job>, JobsValidationError> {
        let parent = parent_session.trim();
        let kind_trimmed = kind.trim();
        validate_path_segment(parent, "parentSession")?;
        validate_path_segment(kind_trimmed, "kind")?;

        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("{kind_trimmed}-{seq}");
        let times = JobTimes {
            started: now_ms,
            activity: now_ms,
            finished: 0,
            steps: 0,
        };
        let job = Job::new(
            id,
            kind_trimmed.to_owned(),
            label.trim().to_owned(),
            parent.to_owned(),
            presentation,
            Status::Running,
            times,
        );

        {
            let mut inner = self.inner.lock().expect("manager lock");
            inner.jobs.insert(job_key(parent, &job.id), job.clone());
            inner.order.push(job.id.clone());
        }

        self.emit_started(&job);
        if let Some(recorder) = self.recorder.lock().expect("recorder lock").clone() {
            recorder.record_start(&job.id, &job.kind, &job.label);
        }
        Ok(job)
    }

    /// Registers a validation-failed placeholder the way `startInvalid` does:
    /// a visible Failed job before any goroutine or artifact exists
    /// (`jobs.go:338-362`).
    pub fn register_invalid(
        self: &Arc<Self>,
        parent_session: &str,
        kind: &str,
        err: &JobsValidationError,
        now_ms: i64,
    ) -> Arc<Job> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("invalid-{seq}");
        let times = JobTimes {
            started: now_ms,
            activity: now_ms,
            finished: now_ms,
            steps: 0,
        };
        let job = Job::new(
            id,
            kind.trim().to_owned(),
            String::new(),
            parent_session.trim().to_owned(),
            Presentation::default(),
            Status::Running,
            times,
        );
        {
            let mut inner = self.inner.lock().expect("manager lock");
            inner
                .jobs
                .insert(job_key(&job.session_id, &job.id), job.clone());
            inner.order.push(job.id.clone());
        }
        job.record_error(&err.to_string());
        self.finish_uninvoked(&job, Status::Failed, Some(&err.to_string()), now_ms);
        job
    }

    // -- Lookup -------------------------------------------------------------

    fn find(&self, session: &str, id: &str) -> Option<Arc<Job>> {
        let inner = self.inner.lock().expect("manager lock");
        let session = session.trim();
        if session.is_empty() {
            inner.jobs.values().find(|j| j.id == id).cloned()
        } else {
            inner.jobs.get(&job_key(session, id)).cloned()
        }
    }

    pub fn job(&self, session: &str, id: &str) -> Option<Arc<Job>> {
        self.find(session, id)
    }

    /// `KillForSession`: flip Running to Killed synchronously, before the run
    /// goroutine unwinds (`jobs.go:795-817`). The actual cancel call is WIP.
    pub fn kill_for_session(&self, session: &str, id: &str) -> bool {
        let Some(job) = self.find(session, id) else {
            return false;
        };
        let mut inner = job.inner.lock().expect("job lock");
        if inner.status != Status::Running {
            return false;
        }
        inner.status = Status::Killed;
        true
    }

    pub fn output_for_session(&self, session: &str, id: &str) -> Option<(String, Status)> {
        self.find(session, id).map(|job| job.take_output())
    }

    // -- Reservation / unfinished ------------------------------------------

    /// `ReserveStartForSession`: running jobs and held reservations both count
    /// against the limit; `limit <= 0` imposes no bound.
    pub fn reserve_start(
        self: &Arc<Self>,
        session: &str,
        kind: &str,
        limit: i64,
    ) -> (Reservation, u32, bool) {
        let key = job_key(session, kind);
        let mut inner = self.inner.lock().expect("manager lock");
        let running = inner
            .jobs
            .values()
            .filter(|j| j.session_id == session.trim() && j.kind == kind && !j.is_done())
            .count() as u32;
        let reserved = inner.reservations.get(&key).copied().unwrap_or(0);
        let decision = reserve_decision(running, reserved, limit);
        let held = if decision.allowed {
            let next = reserved + 1;
            inner.reservations.insert(key.clone(), next);
            next
        } else {
            reserved
        };
        (
            Reservation {
                manager: Arc::downgrade(self),
                key,
            },
            running + held,
            decision.allowed,
        )
    }

    fn release_reservation(&self, key: &str) {
        let mut inner = self.inner.lock().expect("manager lock");
        if let Some(count) = inner.reservations.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                inner.reservations.remove(key);
            }
        }
    }

    pub fn has_unfinished_for_session(&self, session: &str) -> bool {
        let inner = self.inner.lock().expect("manager lock");
        let session = session.trim();
        inner
            .jobs
            .values()
            .any(|j| session_matches(session, &j.session_id) && !j.is_done())
    }

    // -- Completion orchestration ------------------------------------------

    /// Finish an uninvoked job (validation failure, pre-launch cancel): the
    /// same completion accounting as the normal goroutine收尾
    /// (`finalization.go:49-74`).
    pub fn finish_uninvoked(&self, job: &Arc<Job>, status: Status, err: Option<&str>, now_ms: i64) {
        {
            let mut inner = job.inner.lock().expect("job lock");
            inner.times.finished = now_ms;
            inner.run_returned = true;
        }
        self.complete_job(job, status, err, None, String::new(), now_ms);
    }

    /// The in-memory completion driver. Order invariants: completion queued
    /// before the status flip; `done` closed before the observer publish
    /// (`run.go:88-104`). The real run goroutine calls this same path once
    /// spawn lands.
    pub fn complete_job(
        &self,
        job: &Arc<Job>,
        status: Status,
        err: Option<&str>,
        exit: Option<ExitInfo>,
        result_ref: String,
        now_ms: i64,
    ) {
        let input = CompletionInput {
            parent_session: job.session_id.clone(),
            id: job.id.clone(),
            kind: job.kind.clone(),
            label: job.label.clone(),
            result_ref: result_ref.clone(),
            status,
            err: err.map(str::to_owned),
            exit,
            finished_at_ms: now_ms,
        };
        let (event, notice) = record_completion(&input);

        let destroying = {
            let mut inner = self.inner.lock().expect("manager lock");
            let destroying = inner.destroying.contains(&job.session_id);
            if !destroying {
                inner.completed.push(CompletionQueueItem {
                    session_id: event.session_id.clone(),
                    text: event.note.clone(),
                    event_id: event.id.clone(),
                });
            }
            destroying
        };

        if let Some(recorder) = self.recorder.lock().expect("recorder lock").clone() {
            recorder.record_done(&job.id, status, err);
        }

        let should_emit =
            !destroying && should_emit_public(&self.active_session(), &event.session_id);
        if should_emit {
            self.emit_notice(&notice);
        }

        // Status flip: a cancel-published Killed/Interrupted stands.
        {
            let mut inner = job.inner.lock().expect("job lock");
            if !lifecycle::preserve_published_status(inner.status) {
                inner.status = status;
            }
            inner.times.finished = now_ms;
        }
        if let Some(exit) = exit {
            job.record_exit(Some(exit));
        }

        job.done.store(true, Ordering::Release);
        self.publish_completion(&event);
    }

    fn publish_completion(&self, event: &CompletionEvent) {
        let observer = self.observer.lock().expect("observer lock").get();
        let present = observer.is_some();
        let failed = match &observer {
            Some(observer) => observer.on_completion(event).is_err(),
            None => false,
        };
        let decision = completion::publish_decision(event.id.is_empty(), present, failed);
        if decision.ack {
            let mut inner = self.inner.lock().expect("manager lock");
            ack_item(&mut inner.completed, &event.id);
        }
    }

    fn emit_started(&self, job: &Job) {
        if !should_emit_public(&self.active_session(), &job.session_id) {
            return;
        }
        let text = started_notice_text(&job.kind, &job.id, &job.label);
        let payload = JobNotice {
            kind: job.kind.clone(),
            id: job.id.clone(),
            label: job.label.clone(),
            ..JobNotice::default()
        };
        self.emit_notice_raw(
            reasonix_contract::eventwire::notice::code::JOB_STARTED,
            reasonix_contract::eventwire::notice::LEVEL_INFO,
            &text,
            &payload.encode(),
        );
    }

    fn emit_notice(&self, notice: &NoticeDescriptor) {
        self.emit_notice_raw(
            &notice.code,
            &notice.level,
            &notice.text,
            &notice.payload.encode(),
        );
    }

    fn emit_notice_raw(&self, code: &str, level: &str, text: &str, detail: &str) {
        if let Some(sink) = &self.sink {
            sink.emit(code, level, text, detail);
        }
    }

    // -- Drain / observer state --------------------------------------------

    /// Drains queued legacy notes for a session. Real cross-turn fold is WIP.
    pub fn drain_completions(&self, session: &str) -> Vec<CompletionQueueItem> {
        let mut inner = self.inner.lock().expect("manager lock");
        let session = session.trim();
        let (keep, drain): (Vec<_>, Vec<_>) = inner
            .completed
            .iter()
            .cloned()
            .partition(|item| !session_matches(session, &item.session_id));
        inner.completed = keep;
        drain
    }

    pub fn resumes_on_completion(&self, session: &str) -> bool {
        let observer_present = self.observer.lock().expect("observer lock").present();
        resumes_on_completion(observer_present, &self.active_session(), session)
    }

    pub fn observed_by(&self, observer: &Arc<dyn CompletionObserver>) -> bool {
        self.observer
            .lock()
            .expect("observer lock")
            .observed_by(observer)
    }

    // -- Evidence gates ------------------------------------------------------

    pub fn try_lease_evidence(
        &self,
        session: &str,
        id: &str,
    ) -> (Option<ChildEvidenceSummary>, bool) {
        let Some(job) = self.find(session, id) else {
            return (None, false);
        };
        let inner = job.inner.lock().expect("job lock");
        let outcome = try_lease(
            true,
            !job.is_done(),
            inner.evidence_committed,
            inner.evidence.as_ref(),
        );
        (outcome.summary, outcome.ready)
    }

    pub fn pending_evidence_ids(&self, session: &str) -> Vec<String> {
        let inner = self.inner.lock().expect("manager lock");
        let session = session.trim();
        inner
            .jobs
            .values()
            .filter(|j| {
                if !session_matches(session, &j.session_id) || !j.status().is_terminal() {
                    return false;
                }
                let locked = j.inner.lock().expect("job lock");
                pending_evidence(
                    true,
                    locked.evidence_committed,
                    locked.evidence.as_ref().is_some_and(|e| e.has_receipts()),
                )
            })
            .map(|j| j.id.clone())
            .collect()
    }

    pub fn commit_evidence(&self, session: &str, id: &str) -> bool {
        let Some(job) = self.find(session, id) else {
            return false;
        };
        let mut inner = job.inner.lock().expect("job lock");
        let outcome = commit_decision(true, !job.is_done(), inner.evidence_committed);
        if outcome.committed {
            inner.evidence_committed = true;
            inner.evidence = None;
        }
        outcome.committed
    }
}

/// Held start reservation; drops decrement the in-flight slot once
/// (`jobs.go:856-894`).
pub struct Reservation {
    manager: Weak<Manager>,
    key: String,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            manager.release_reservation(&self.key);
        }
    }
}

fn should_emit_public(active: &str, session: &str) -> bool {
    completion::should_emit(active, session)
}

// ---------------------------------------------------------------------------
// Call context binding (jobs.go:1647-1684)
// ---------------------------------------------------------------------------

/// Marker masking an ancestor manager the way `WithoutManager` does.
pub struct NoManager;

/// Binds the manager onto a fresh call context. `CallContext` carries one
/// typed binding, so an ancestor value is not preserved the way Go's layered
/// `context.Context` is; the composition root binds the manager it wants.
pub fn bind_manager(manager: Arc<Manager>) -> CallContext {
    let boxed: Arc<Arc<Manager>> = Arc::new(manager);
    CallContext::new().with_value(boxed)
}

/// Reads the bound manager, if any. A plain context or a masked value reads
/// as absent (`FromContext`).
pub fn manager_from(ctx: &CallContext) -> Option<Arc<Manager>> {
    let value = ctx.value()?;
    value.downcast_ref::<Arc<Manager>>().cloned()
}

/// Masks any ancestor manager, so a sub-agent without its own manager cannot
/// drive the parent's jobs (`WithoutManager`).
pub fn without_manager() -> CallContext {
    let marker: Arc<NoManager> = Arc::new(NoManager);
    CallContext::new().with_value(marker)
}

#[cfg(test)]
mod mod_tests;
