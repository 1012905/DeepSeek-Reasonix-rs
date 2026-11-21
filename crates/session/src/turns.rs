//! The controller write side (Go `internal/session/control` — the turn
//! group: `Submit`, `Cancel`, `Steer`, and the turn lifecycle a frontend
//! needs to start and steer turns). One controller drives one runtime: it
//! owns the agent machine, binds the admission saver to the sessionstore
//! save, and lands every turn's output on the user through the sink the
//! boot composed.
//!
//! Approval orchestration (Go `approval*.go`), slash dispatch, the durable
//! inbox, and the goal loop are separate waves; this build submits raw
//! user turns and refuses nothing else with typed errors.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use reasonix_agent::config::AgentConfig;
use reasonix_agent::seams::{SteerInbox, TranscriptSaver};
use reasonix_agent::sink::Sink;
use reasonix_agent::{Agent, TurnError};
use reasonix_contract::eventwire::event::Event;
use reasonix_provider::Client;
use reasonix_state::sessionstore::{Session, new_session_path};
use reasonix_tools::Registry;

use crate::controller::Controller;

/// The parts one controller is built from (Go `control.Options` — the
/// turn-group subset this build ports). The sink is the boot-composed emit
/// pipeline; the controller shares it with the agent so a turn's events and
/// its completion fan out through one chain.
pub struct TurnControllerParts {
    pub config: AgentConfig,
    pub client: Client,
    pub registry: Arc<Registry>,
    pub sink: Arc<dyn Sink>,
    pub workspace_root: String,
    /// Where new transcripts land (Go `SessionDir`); empty keeps the
    /// transcript in memory only until a path is pinned.
    pub session_dir: String,
    /// The name new transcript files carry (Go `Label` — the model name).
    pub label: String,
}

/// Why a submission was dropped without starting a turn (Go: the refused
/// half of `Admission`). The identity is the variant, never a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitRefusal {
    /// A turn is already running on this controller (Go: the
    /// `ctrl.Running()` guard — a dropped concurrent submit; durable
    /// follow-up is the inbox machine, not yet ported).
    Busy,
}

/// How one submitted turn ended (Go `finishGuardedTurn`'s `done` event,
/// minus the unported checkpoint/inbox fields). The same facts land on the
/// wire as the `turn_done` event.
#[derive(Debug)]
pub struct TurnReceipt {
    pub cancelled: bool,
    /// The typed turn failure; `None` when the model gave a final answer.
    pub error: Option<TurnError>,
}

/// One controller over one runtime (Go `control.Controller`, the turn
/// group). The frozen read surface ([`Controller`]) is implemented so the
/// hub can adopt the runtime the controller drives.
pub struct TurnController {
    agent: Arc<Agent>,
    session: Arc<Session>,
    sink: Arc<dyn Sink>,
    workspace_root: String,
    session_dir: Mutex<String>,
    label: String,
    session_path: Mutex<String>,
    steer: Arc<MemorySteerInbox>,
    running: AtomicBool,
    cancel_requested: AtomicBool,
}

impl TurnController {
    /// Builds the controller and its agent, binding the seams the turn
    /// group owns (Go `NewController`: `SetAdmissionSaver`, the steer
    /// inbox). The admission saver writes the sessionstore save directly —
    /// the binding the agent spine's [`TranscriptSaver`] docs name.
    pub fn new(parts: TurnControllerParts) -> Arc<Self> {
        let TurnControllerParts {
            config,
            client,
            registry,
            sink,
            workspace_root,
            session_dir,
            label,
        } = parts;
        // The transcript opens with the cache-stable prompt (Go:
        // sessionstore.NewSession(b.prompt.prompt)); the composer leaves a
        // leading system message alone, so the wire carries it once.
        let session = Arc::new(Session::new(&config.system_prompt));
        let agent = Arc::new(Agent::new(
            config,
            client,
            registry,
            session.clone(),
            sink.clone(),
        ));
        let steer = Arc::new(MemorySteerInbox::default());
        agent.set_steer_inbox(steer.clone());
        let ctrl = Arc::new(Self {
            agent,
            session,
            sink,
            workspace_root,
            session_dir: Mutex::new(session_dir),
            label,
            session_path: Mutex::new(String::new()),
            steer,
            running: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
        });
        ctrl.agent
            .set_transcript_saver(Arc::new(AdmissionSave(ctrl.clone())));
        ctrl
    }

    /// The agent machine this controller drives (Go `c.executor`).
    pub fn agent(&self) -> &Arc<Agent> {
        &self.agent
    }

    /// The transcript this controller's turns write (Go
    /// `c.executor.Session()`).
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Whether a turn is running (Go `Running`): the admission guard the
    /// frontends read before submitting.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Whether [`Self::cancel`] was asked of the active turn (Go
    /// `CancelRequested`).
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    /// Pins a fresh auto-save file when none is set and a session dir is
    /// configured (Go `EnsureSessionPath`). A no-op once a path is pinned.
    pub fn ensure_session_path(&self) {
        let mut path = self.session_path.lock().expect("path lock");
        if !path.is_empty() || self.session_dir().is_empty() {
            return;
        }
        *path = new_session_path(self.session_dir(), &self.label)
            .to_string_lossy()
            .into_owned();
    }

    /// Points auto-save at `path` without carrying new history (Go
    /// `SetSessionPath`, the fresh-pin half of `Resume`).
    pub fn set_session_path(&self, path: &str) {
        *self.session_path.lock().expect("path lock") = path.to_owned();
    }

    /// Replaces where new transcripts land (Go `SetSessionDir`).
    pub fn set_session_dir(&self, dir: &str) {
        *self.session_dir.lock().expect("dir lock") = dir.to_owned();
    }

    /// Synchronous turn admission (Go: the `running` swap inside
    /// `runGuarded`, taken before the caller's goroutine starts). A caller
    /// that admits and then spawns never answers 202 for a turn another
    /// submit already owns — the admission decides, not the spawn order.
    pub fn try_admit(&self) -> bool {
        !self.running.swap(true, Ordering::SeqCst)
    }

    /// Drives an admitted turn to completion (Go: the body of `Submit` after
    /// admission: `runTurnLoop` → `runWithRunner`). Slash commands, memory
    /// quick-adds and shell lines are the composition machine and stay
    /// unported — this entry is the trusted user-turn path
    /// (`SubmitUserTurn`).
    ///
    /// The `turn_done` event reaches the sink before this returns, so a
    /// spawned caller publishes completion through the stream exactly once.
    pub async fn run_admitted(&self, input: &str) -> TurnReceipt {
        self.cancel_requested.store(false, Ordering::SeqCst);
        self.ensure_session_path();
        let result = self.agent.run(input).await;
        self.running.store(false, Ordering::SeqCst);
        self.finish_turn(result)
    }

    /// Submits raw user input as one turn and drives it to completion (Go
    /// `Submit`'s plain-turn path). Admission and the turn are one call.
    pub async fn submit(&self, input: &str) -> Result<TurnReceipt, SubmitRefusal> {
        if !self.try_admit() {
            return Err(SubmitRefusal::Busy);
        }
        Ok(self.run_admitted(input).await)
    }

    /// Aborts the in-flight turn at its next await point (Go `Cancel`).
    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.agent.cancel();
    }

    /// Ends one running tool call without ending the turn (Go
    /// `CancelToolCall`); reports whether the call was still pending.
    pub fn cancel_tool_call(&self, id: &str) -> bool {
        self.agent.cancel_tool_call(id)
    }

    /// Queues mid-turn guidance the agent consumes at its next round
    /// boundary (Go `Steer` through the executor's inbox).
    pub fn steer(&self, text: &str) {
        self.steer.push(text);
    }

    /// Persists the transcript now (Go `Snapshot`). A no-op without a
    /// pinned path.
    pub fn snapshot(&self) -> Result<(), reasonix_state::sessionstore::session::SaveError> {
        let path = self.session_path.lock().expect("path lock").clone();
        if path.is_empty() {
            return Ok(());
        }
        self.session.save(&path)
    }

    /// Saves a changed transcript and lands the completion event (Go:
    /// `finishInFlightTurn`'s snapshot followed by `finishGuardedTurn`'s
    /// `TurnDone`). The transcript is durable before the completion event
    /// fans out.
    fn finish_turn(&self, result: Result<reasonix_agent::TurnOutput, TurnError>) -> TurnReceipt {
        let cancelled = self.cancel_requested.load(Ordering::SeqCst);
        let (work_ms, error, turn_cancelled) = match result {
            Ok(out) => (out.work_duration_ms, None, false),
            Err(err) => {
                let was_cancelled = cancelled && is_cancelled(&err);
                (0, Some(err), was_cancelled)
            }
        };
        if let Err(err) = self.snapshot() {
            // A failed turn-end save is logged, never raised: the user's
            // answer still reached the sink (Go: the snapshot warn paths).
            tracing::warn!("controller: turn-end snapshot failed: {err}");
        }
        let err_text = error.as_ref().map(|e| e.to_string()).unwrap_or_default();
        self.sink.emit(Event {
            kind: TURN_DONE.to_owned(),
            err: err_text,
            cancelled: turn_cancelled,
            work_duration_ms: work_ms,
            ..Event::default()
        });
        TurnReceipt {
            cancelled: turn_cancelled,
            error,
        }
    }
}

/// `event.TurnDone`'s wire name. The spine never emits it, so the name
/// lives with its only producer.
pub const TURN_DONE: &str = "turn_done";

/// Go: `errors.Is(err, context.Canceled)` — a turn the caller aborted, as
/// the spine and the provider transport each spell it.
fn is_cancelled(err: &TurnError) -> bool {
    matches!(
        err,
        TurnError::Cancelled | TurnError::Provider(reasonix_provider::ProviderError::Cancelled)
    )
}

impl Controller for TurnController {
    fn session_path(&self) -> String {
        self.session_path.lock().expect("path lock").clone()
    }

    fn session_dir(&self) -> String {
        self.session_dir.lock().expect("dir lock").clone()
    }

    fn workspace_root(&self) -> String {
        self.workspace_root.clone()
    }
}

/// The admission saver (Go `Controller.SaveAdmittedMessage`): the next
/// periodic save is a turn away, so without this one a kill in between
/// leaves the prompt in memory only. A failed save is logged and does not
/// refuse the turn.
struct AdmissionSave(Arc<TurnController>);

impl TranscriptSaver for AdmissionSave {
    fn save_admitted_message(&self) {
        if self.0.session_path().is_empty() {
            return;
        }
        if let Err(err) = self.0.snapshot() {
            tracing::warn!("controller: save admitted user message: {err}");
        }
    }
}

/// The steer inbox a controller queues guidance into (Go `steerInbox`, the
/// in-memory half; the durable inbox wave replaces the storage).
#[derive(Default)]
struct MemorySteerInbox {
    queue: Mutex<VecDeque<String>>,
}

impl MemorySteerInbox {
    fn push(&self, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.queue
                .lock()
                .expect("steer lock")
                .push_back(text.to_owned());
        }
    }
}

impl SteerInbox for MemorySteerInbox {
    fn drain(&self) -> Option<String> {
        self.queue.lock().expect("steer lock").pop_front()
    }
}
