//! The turn machine (Go: `agent.go` `Run` + `run_loop.go`).
//!
//! One run: begin (compose the user turn, announce, persist) → sampling
//! rounds (frozen request, up to [`MAX_SAMPLING_ATTEMPTS`] body attempts, a
//! clean terminal commits) → tool rounds (batch dispatch, results paired
//! into the transcript) → final answer or a resumable stop. Failed attempts
//! never write session state; executed tools always keep their results
//! paired with their calls.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::provider::{Message, Role, ToolCall};
use reasonix_provider::{ChatCompletionRequest, Client, Usage};
use reasonix_state::sessionstore::Session;
use reasonix_tools::Registry;

use crate::boundary::{
    classify_response_boundary, finish_reason_message, truncated_terminal, truncation_cause,
};
use crate::cancel::{CallStops, CancelToken};
use crate::config::{AgentConfig, MAX_EMPTY_FINAL_BLOCKS, MAX_SAMPLING_ATTEMPTS};
use crate::error::{StopCause, TurnError};
use crate::execute::{self, BatchInputs};
use crate::seams::{Asker, Compactor, Gate, RequestComposer, SteerInbox, TranscriptSaver};
use crate::sink::{Sink, kind, level, notice_code, phase};
use crate::stream::consume_stream;
use crate::usage::{emit_turn_usage, finalize_sampling_usage, merge_sampling_usage};

/// What a finished turn hands back (Go: `Run`'s nil-error return plus what
/// the caller could otherwise only read off the session).
#[derive(Debug)]
pub struct TurnOutput {
    /// The final answer's visible text; empty for a resumable pause.
    pub text: String,
    pub reasoning: String,
    pub usage: Option<Usage>,
    /// Wall-clock duration of the whole turn, in milliseconds (Go:
    /// `workDurationMs`).
    pub work_duration_ms: i64,
    pub stop: StopCause,
}

/// The state one run owns (Go: `turnRuntime`). Built fresh per run; state
/// that outlives the run lives on [`Agent`].
#[derive(Debug, Default, Clone)]
pub struct TurnState {
    /// Tool rounds executed so far in this run.
    pub rounds: i64,
    /// Whether any tool ran (Go: `usedAnyTool`).
    pub used_any_tool: bool,
    /// Consecutive empty finals (Go: `emptyFinalBlocks`).
    pub empty_final_blocks: i64,
    /// Whether the finalization round is armed (Go: `graceRound`).
    pub grace_round: bool,
    /// Why the grace round was armed (Go: `landCause`).
    pub stop: Option<StopCause>,
    /// Repair instructions already spent on context-unavailable tools (Go:
    /// `contextToolRepairs`).
    pub context_tool_repairs: i64,
}

/// The per-session usage aggregate that compaction must not reset (Go:
/// `sess.cacheHit/cacheMiss` — the hit-rate would crater every time the
/// visible prefix is folded).
#[derive(Debug, Default)]
struct SessionCache {
    hit: AtomicI64,
    miss: AtomicI64,
}

/// The seams one agent carries. Guarded by one mutex because Go installs
/// them on a live agent (`SetGate` is safe between turns); the loop reads
/// them per round.
#[derive(Default)]
struct Seams {
    composer: Option<Arc<dyn RequestComposer>>,
    gate: Option<Arc<dyn Gate>>,
    asker: Option<Arc<dyn Asker>>,
    compactor: Option<Arc<dyn Compactor>>,
    saver: Option<Arc<dyn TranscriptSaver>>,
    steer: Option<Arc<dyn SteerInbox>>,
}

/// The collaborator bundle one agent drives (Go: `agentServices` +
/// `agentConfig`).
pub struct Agent {
    config: AgentConfig,
    client: Client,
    registry: Arc<Registry>,
    session: Arc<Session>,
    sink: Arc<dyn Sink>,
    seams: std::sync::Mutex<Seams>,
    /// The context-window machine (Go: `a.window()` — built per entry there;
    /// owned here and reached through it). `None` runs the spine exactly as
    /// before: compaction off, no clip, no recovery. Guarded like the seams
    /// because Go installs machines on a live agent.
    window: std::sync::Mutex<Option<Arc<crate::window::Window>>>,

    cache: SessionCache,
    stops: CallStops,
    cancel: CancelToken,
    running: AtomicBool,
}

impl Agent {
    /// Wires one agent (Go: `New`). The default request composer shapes the
    /// provider-visible request; every seam starts nil.
    pub fn new(
        config: AgentConfig,
        client: Client,
        registry: Arc<Registry>,
        session: Arc<Session>,
        sink: Arc<dyn Sink>,
    ) -> Self {
        Agent {
            config,
            client,
            registry,
            session,
            sink,
            seams: std::sync::Mutex::new(Seams::default()),
            window: std::sync::Mutex::new(None),
            cache: SessionCache::default(),
            stops: CallStops::default(),
            cancel: CancelToken::default(),
            running: AtomicBool::new(false),
        }
    }

    /// Installs the context-window machine (Go: the `window()` entry the
    /// loop reaches). Its Prepare pass, output clip and overflow recovery
    /// join the request lifecycle from here.
    pub fn set_window(&self, window: Arc<crate::window::Window>) {
        *self.window.lock().expect("window lock") = Some(window);
    }

    /// The context window, when one is installed (Go: `a.window()`).
    pub fn window(&self) -> Option<Arc<crate::window::Window>> {
        self.window.lock().expect("window lock").clone()
    }

    /// Installs the request-composition seam.
    pub fn set_composer(&self, composer: Arc<dyn RequestComposer>) {
        self.seams.lock().expect("seams lock").composer = Some(composer);
    }

    /// Installs the per-call permission seam (Go: `SetGate`).
    pub fn set_gate(&self, gate: Arc<dyn Gate>) {
        self.seams.lock().expect("seams lock").gate = Some(gate);
    }

    /// Installs the structured-question seam (Go: `SetAsker`).
    pub fn set_asker(&self, asker: Arc<dyn Asker>) {
        self.seams.lock().expect("seams lock").asker = Some(asker);
    }

    /// Installs the compaction seam (Go: the compact hooks).
    pub fn set_compactor(&self, compactor: Arc<dyn Compactor>) {
        self.seams.lock().expect("seams lock").compactor = Some(compactor);
    }

    /// Installs the admission-durability seam (Go: `SetAdmissionSaver`).
    pub fn set_transcript_saver(&self, saver: Arc<dyn TranscriptSaver>) {
        self.seams.lock().expect("seams lock").saver = Some(saver);
    }

    /// Installs the steer-inbox seam.
    pub fn set_steer_inbox(&self, steer: Arc<dyn SteerInbox>) {
        self.seams.lock().expect("seams lock").steer = Some(steer);
    }

    /// The transcript this agent appends to (Go: `Session()`).
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// The tools this agent's turns may call (Go: `a.svc.tools`' registry,
    /// read by hosts that report the surface a session offers).
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Ends one running call without ending the turn (Go:
    /// `CancelToolCall`); reports whether the call was still pending.
    pub fn cancel_tool_call(&self, id: &str) -> bool {
        self.stops.cancel(id)
    }

    /// Cancels the whole turn at the next await point (Go: the Run
    /// context's cancel).
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Starts a fresh cancellation generation at the host's admission boundary.
    pub fn reset_cancellation(&self) {
        self.cancel.reset();
    }

    /// Appends the user input and drives the tool loop until the model
    /// returns a final answer, the turn is cancelled, or the provider
    /// errors (Go: `Run`).
    pub async fn run(&self, input: &str) -> Result<TurnOutput, TurnError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(TurnError::Composition(
                "this agent is already running a turn".to_owned(),
            ));
        }
        let started = Instant::now();
        let result = self.run_tool_loop(input).await;
        self.running.store(false, Ordering::SeqCst);
        self.cancel.reset();
        let work_ms = started.elapsed().as_millis().clamp(1, i64::MAX as u128) as i64;
        result.map(|mut out| {
            if out.work_duration_ms == 0 {
                out.work_duration_ms = work_ms;
            }
            out
        })
    }

    /// The main tool-round budget and dispatch (Go: `beginRunTurn` +
    /// `runToolLoop`).
    async fn run_tool_loop(&self, input: &str) -> Result<TurnOutput, TurnError> {
        let turn = Mutex::new(TurnState::default());
        self.begin_run_turn(input);
        let mut step = 0i64;

        loop {
            let grace = turn.lock().expect("turn lock").grace_round;
            if self.config.max_steps > 0 && step >= self.config.max_steps && !grace {
                let stop = StopCause::MaxSteps {
                    steps: self.config.max_steps,
                    key: self.config.max_steps_key.clone(),
                };
                self.arm_finalization_round(&turn, &stop);
            }
            if self.cancel.is_cancelled() {
                return Err(TurnError::Cancelled);
            }
            // Consume queued steer and persist it so it survives history
            // replay (Go: `consumeSteer` + `landSteer`).
            let steer = self.seams.lock().expect("seams lock").steer.clone();
            if let Some(steer) = steer
                && let Some(text) = steer.drain()
            {
                self.add_host_message(&text);
            }
            // The prompt only grows from here; compact before the next
            // request so it stays within the model's window (Go: the
            // compact seam).
            let compactor = self.seams.lock().expect("seams lock").compactor.clone();
            if let Some(compactor) = compactor {
                let state = turn.lock().expect("turn lock").clone();
                compactor.maybe_compact(&state)?;
            }

            let round_started = Instant::now();
            let mut frozen = self.prepare_request()?;
            // One model round = one billable record: the merge spans this
            // round's body attempts only, never the whole turn (Go: the
            // billable local of streamWithSamplingRecovery).
            let mut billable: Option<Usage> = None;
            let streamed = self
                .stream_with_sampling_recovery(&mut frozen, &mut billable)
                .await;
            let usage = finalize_sampling_usage(billable.as_ref(), streamed.usage.as_ref());
            if let Some(u) = &usage {
                emit_turn_usage(self.sink.as_ref(), u, self.config.model_ref.as_str(), "");
            }
            if let Some(err) = streamed.err {
                // Exhausted stream retries (or a non-retryable error):
                // intermediate failed attempts never wrote session state;
                // the turn ends with the typed provider failure (Go: the
                // err arm of runToolLoop; the bounded recovery record is
                // the interrupted-recovery wave's shape).
                return Err(TurnError::Provider(err));
            }
            let work_ms = round_started
                .elapsed()
                .as_millis()
                .clamp(1, i64::MAX as u128) as i64;

            // Classify the terminal before anything commits: a call the
            // output limit cut in half reaches neither history nor
            // execution (Go: `classifyResponseBoundary`).
            let boundary = classify_response_boundary(usage.as_ref(), &streamed.calls);
            let mut calls = boundary.committed.clone();
            if let Some(msg) = finish_reason_message(usage.as_ref()) {
                self.notice(level::WARN, "", &msg, &boundary.notice_detail());
            }

            // Commit boundary: only a clean terminal attempt reaches here
            // (Go: the conversation.Add of the assistant message).
            self.session.add(Message {
                role: Role::ASSISTANT,
                content: streamed.text.clone(),
                reasoning_content: streamed.reasoning.clone(),
                tool_calls: calls.clone(),
                work_duration_ms: work_ms,
                thought_ms: streamed.thought_ms,
                model_ref: self.config.model_ref.clone(),
                ..Message::default()
            });

            if calls.is_empty() {
                if boundary.record(|fact| self.add_host_message(fact)) {
                    // The truncated tail was the whole batch: nothing ran
                    // and no answer was finished, so the host fact is this
                    // round's result (Go: the recordTruncationFact arm).
                    step += 1;
                    continue;
                }
                match self.handle_final_response(
                    &turn,
                    &streamed.text,
                    &streamed.reasoning,
                    usage.as_ref(),
                ) {
                    Final::Answer => {
                        return Ok(TurnOutput {
                            text: streamed.text,
                            reasoning: streamed.reasoning,
                            usage,
                            work_duration_ms: 0,
                            stop: StopCause::Final,
                        });
                    }
                    Final::Continue => {
                        step += 1;
                        continue;
                    }
                    Final::Pause(stop) => return Ok(self.paused_output(stop, usage)),
                    Final::Err(err) => return Err(err),
                }
            }

            // Invariant: execute_batch only ever receives tool calls from a
            // committed sampling attempt (Go: `handleToolRound`).
            match self.handle_tool_round(&turn, &mut calls, &streamed.text, usage.as_ref()) {
                ToolRound::Continue => {
                    // After the tool results, never between them and their
                    // call: the provider requires an unbroken
                    // assistant/tool pairing (Go: the recordTruncationFact
                    // arm of runToolLoop).
                    boundary.record(|fact| self.add_host_message(fact));
                    step += 1;
                    continue;
                }
                ToolRound::Final => {
                    // Keep the assistant tool call and host error paired in
                    // the transcript, but accept the co-streamed answer
                    // (Go: the unavailable-tools final arm).
                    return Ok(TurnOutput {
                        text: streamed.text,
                        reasoning: streamed.reasoning,
                        usage,
                        work_duration_ms: 0,
                        stop: StopCause::Final,
                    });
                }
                ToolRound::Pause(stop) => return Ok(self.paused_output(stop, usage)),
                ToolRound::Err(err) => return Err(err),
            }
        }
    }

    fn paused_output(&self, stop: StopCause, usage: Option<Usage>) -> TurnOutput {
        TurnOutput {
            text: String::new(),
            reasoning: String::new(),
            usage,
            work_duration_ms: 0,
            stop,
        }
    }

    /// Composes, announces, and persists the user turn (Go:
    /// `beginRunTurn` + `openUserTurn` + `LandAuthoredUserMessage`).
    /// Composing before announcing is what lets the announcement name the
    /// message.
    fn begin_run_turn(&self, input: &str) {
        let raw = input.trim();
        let msgs = self.session.snapshot();
        let index = msgs.len();
        self.sink.emit(Event {
            kind: kind::TURN_STARTED.to_owned(),
            text: raw.to_owned(),
            msg_index: Some(index as i64),
            model_ref: self.config.model_ref.clone(),
            ..Event::default()
        });
        self.emit_turn_phase(phase::WORKING);
        let created_at = now_ms();
        if let Some(window) = self.window() {
            // The window reads this stamp to find the running turn's request
            // and to scope per-turn no-fold reports (Go: the
            // activeTurnCreatedAt write in beginRunTurn).
            window.note_active_turn_created_at(created_at);
        }
        self.session.add(Message {
            role: Role::USER,
            content: raw.to_owned(),
            raw_content: raw.to_owned(),
            created_at,
            ..Message::default()
        });
        let saver = self.seams.lock().expect("seams lock").saver.clone();
        if let Some(saver) = saver {
            saver.save_admitted_message();
        }
    }

    fn emit_turn_phase(&self, name: &str) {
        self.sink.emit(Event {
            kind: kind::TURN_PHASE.to_owned(),
            phase: name.to_owned(),
            text: name.to_owned(),
            ..Event::default()
        });
    }

    fn notice(&self, level_value: &str, code: &str, text: &str, detail: &str) {
        self.sink.emit(Event {
            kind: kind::NOTICE.to_owned(),
            level: level_value.to_owned(),
            code: code.to_owned(),
            text: text.to_owned(),
            detail: detail.to_owned(),
            ..Event::default()
        });
    }

    /// One host-authored message into the transcript (Go: the
    /// `HostAuthored: true` conversation.Add calls).
    fn add_host_message(&self, content: &str) {
        self.session.add(Message {
            role: Role::USER,
            content: content.to_owned(),
            host_authored: true,
            ..Message::default()
        });
    }

    /// Builds the request once and freezes it for the whole attempt
    /// lifecycle — stream retries must not rewrite session history mid-round
    /// (Go: `prepareSamplingRequest`). With a window installed, the machine
    /// owns the pass: Prepare at the pressure trigger, the output-budget
    /// clip, and the anchor the provider's report will complete.
    fn prepare_request(&self) -> Result<ChatCompletionRequest, TurnError> {
        let base = crate::seams::RequestBase {
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
        };
        let composer: Arc<dyn RequestComposer> = self
            .seams
            .lock()
            .expect("seams lock")
            .composer
            .clone()
            .unwrap_or_else(|| Arc::new(crate::compose::DefaultComposer::default()));
        if let Some(window) = self.window() {
            window.set_composer(composer.clone());
            return window.prepare_request(composer);
        }
        composer.compose(
            &self.config.system_prompt,
            &self.session.snapshot(),
            &self.registry,
            &base,
        )
    }

    /// Coordinates the replay lifecycle for one model round: prepare once,
    /// run up to [`MAX_SAMPLING_ATTEMPTS`] body attempts, and only commit
    /// after a clean terminal (Go: `streamWithSamplingRecovery`). An
    /// interrupted attempt replays the exact frozen request; a context
    /// overflow gets the window machine's one-shot fold-and-rebuild; every
    /// other failure is terminal.
    async fn stream_with_sampling_recovery(
        &self,
        frozen: &mut ChatCompletionRequest,
        billable: &mut Option<Usage>,
    ) -> crate::stream::StreamedTurn {
        // The one-shot latch travels with the frozen request (Go:
        // `samplingRequest.overflowFolded`).
        let mut overflow_folded = false;
        for attempt in 1..=MAX_SAMPLING_ATTEMPTS {
            let attempt_id = new_stream_attempt_id(attempt);
            self.emit_stream_attempt(&attempt_id, "begin", attempt, "");
            let stream = match self.client.stream(frozen).await {
                Ok(stream) => stream,
                // A pre-stream failure is terminal — unless the provider is
                // refusing the input as too long, which the window machine
                // answers with its one-shot fold-and-replay (Go:
                // handleSamplingError runs the same recovery on a Stream()
                // call failure).
                Err(err) => {
                    if attempt < MAX_SAMPLING_ATTEMPTS
                        && let Some(window) = self.window()
                        && matches!(
                            window.recover_context_overflow(frozen, &mut overflow_folded, &err),
                            crate::window::recovery::OverflowRecovery::Recovered
                        )
                    {
                        self.emit_stream_attempt(
                            &attempt_id,
                            "discard",
                            attempt,
                            "context overflow",
                        );
                        continue;
                    }
                    let reason = err.stream_interrupt_reason().to_owned();
                    self.emit_stream_attempt(&attempt_id, "discard", attempt, &reason);
                    return crate::stream::StreamedTurn {
                        err: Some(err),
                        ..crate::stream::StreamedTurn::default()
                    };
                }
            };
            let result =
                consume_stream(stream, self.sink.as_ref(), &attempt_id, Some(&self.cancel)).await;
            if let Some(u) = &result.usage {
                self.cache
                    .hit
                    .fetch_add(u.cache_hit_tokens, Ordering::SeqCst);
                self.cache
                    .miss
                    .fetch_add(u.cache_miss_tokens, Ordering::SeqCst);
            }
            *billable = merge_sampling_usage(billable.take(), result.usage.as_ref());
            // lastUsage is the latest single-request shape, never the
            // multi-attempt billable aggregate — that would inflate
            // compaction decisions (Go: storeLatestRequestUsage).
            if let Some(window) = self.window()
                && let Some(u) = &result.usage
            {
                window.store_latest_request_usage(u);
            }
            match &result.err {
                None => {
                    self.emit_stream_attempt(&attempt_id, "commit", attempt, "");
                    return result;
                }
                Some(err) => {
                    let reason = err.stream_interrupt_reason().to_owned();
                    self.emit_stream_attempt(&attempt_id, "discard", attempt, &reason);
                    if attempt < MAX_SAMPLING_ATTEMPTS && err.is_stream_interrupted() {
                        self.stream_retry_sleep(attempt).await;
                        continue;
                    }
                    if attempt < MAX_SAMPLING_ATTEMPTS
                        && !err.is_stream_interrupted()
                        && let Some(window) = self.window()
                    {
                        match window.recover_context_overflow(frozen, &mut overflow_folded, err) {
                            crate::window::recovery::OverflowRecovery::Recovered => {
                                self.emit_stream_attempt(
                                    &attempt_id,
                                    "discard",
                                    attempt,
                                    "context overflow",
                                );
                                continue;
                            }
                            crate::window::recovery::OverflowRecovery::Failed { reason } => {
                                // The person sees why the recovery that was
                                // tried did not help (Go wraps the terminal
                                // error; the typed provider failure keeps its
                                // identity here, so the reason rides a
                                // notice instead).
                                self.notice(
                                    level::WARN,
                                    "",
                                    "context overflow recovery did not help",
                                    &reason,
                                );
                            }
                            crate::window::recovery::OverflowRecovery::NotApplicable => {}
                        }
                    }
                    return result;
                }
            }
        }
        // MAX_SAMPLING_ATTEMPTS is at least one, so every iteration returns.
        unreachable!("sampling loop always returns")
    }

    /// Waits out a body-retry backoff (Go: `streamRetryDelay`); tests pin
    /// [`AgentConfig::retry_backoff`] to zero.
    async fn stream_retry_sleep(&self, attempt: u32) {
        let base = self.config.retry_backoff.unwrap_or_else(|| {
            let shift = (attempt - 1).min(4);
            std::time::Duration::from_millis(500u64 << shift)
        });
        if !base.is_zero() {
            tokio::time::sleep(base).await;
        }
    }

    fn emit_stream_attempt(&self, id: &str, action: &str, attempt: u32, reason: &str) {
        self.sink.emit(Event {
            kind: kind::STREAM_ATTEMPT.to_owned(),
            stream_attempt: Some(reasonix_contract::eventwire::event::StreamAttempt {
                id: id.to_owned(),
                action: action.to_owned(),
                attempt: attempt as i64,
                max: MAX_SAMPLING_ATTEMPTS as i64,
                reason: reason.to_owned(),
                ..reasonix_contract::eventwire::event::StreamAttempt::default()
            }),
            ..Event::default()
        });
    }

    /// Processes a no-tool assistant turn (Go: `handleFinalResponse` — the
    /// spine's subset: the empty-final retry and the grace-round ends).
    fn handle_final_response(
        &self,
        turn: &Mutex<TurnState>,
        text: &str,
        reasoning: &str,
        usage: Option<&Usage>,
    ) -> Final {
        let visible = has_visible_final_answer(text);
        let blocked_out = {
            let mut state = turn.lock().expect("turn lock");
            if state.grace_round {
                if visible {
                    return Final::Answer;
                }
                // A round ceiling is a user-selected boundary: the summary
                // stays and the turn pauses resumably instead of silently
                // opening another round (Go: the graceRound arms).
                return Final::Pause(state.stop.clone().unwrap_or_else(|| StopCause::MaxSteps {
                    steps: self.config.max_steps,
                    key: self.config.max_steps_key.clone(),
                }));
            }
            if visible {
                return Final::Answer;
            }
            state.empty_final_blocks += 1;
            state.empty_final_blocks >= MAX_EMPTY_FINAL_BLOCKS
        };
        if blocked_out {
            let blocks = turn.lock().expect("turn lock").empty_final_blocks;
            return Final::Err(TurnError::EmptyFinal {
                blocks,
                finish_reason: usage.map(|u| u.finish_reason.clone()).unwrap_or_default(),
            });
        }
        self.notice(
            level::INFO,
            notice_code::EMPTY_FINAL,
            "No visible answer was produced; asking the assistant to respond again.",
            &empty_final_notice_detail(&self.config.model_ref, usage, reasoning.len()),
        );
        self.add_host_message(&empty_final_retry_message(usage));
        Final::Continue
    }

    /// Executes a tool batch, persists tool messages, and handles the
    /// cancellation and boundary stops (Go: `handleToolRound`).
    fn handle_tool_round(
        &self,
        turn: &Mutex<TurnState>,
        calls: &mut [ToolCall],
        text: &str,
        _usage: Option<&Usage>,
    ) -> ToolRound {
        {
            let mut state = turn.lock().expect("turn lock");
            state.empty_final_blocks = 0;
            state.used_any_tool = true;
        }
        if let Some(stop) = grace_stop(turn, self.config.max_steps, &self.config.max_steps_key) {
            // The grace round's calls were authored before the nudge
            // existed, so they reason from an answer the model had not
            // received: paired and refused (Go:
            // `stopUnexecutedBoundaryCalls`).
            for call in calls.iter() {
                self.session.add(Message {
                    role: Role::TOOL,
                    content: GRACE_REFUSAL.to_owned(),
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    ..Message::default()
                });
            }
            return ToolRound::Pause(stop);
        }

        let unavailable = self.unavailable_contextual_tools(calls);
        let repairs = turn.lock().expect("turn lock").context_tool_repairs;
        if !unavailable.is_empty() && repairs > 0 {
            let msg = format!(
                "blocked: context-unavailable tools were called again after the repair instruction: {}",
                unavailable.join(", ")
            );
            for call in calls.iter() {
                self.session.add(Message {
                    role: Role::TOOL,
                    content: msg.clone(),
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    tool_failure: Some(reasonix_contract::provider::ToolFailure {
                        refusal_code: String::new(),
                        blocked: true,
                    }),
                    ..Message::default()
                });
            }
            if has_visible_final_answer(text) {
                return ToolRound::Final;
            }
            return ToolRound::Err(TurnError::ContextToolLoop { tools: unavailable });
        }

        let (gate, asker) = {
            let seams = self.seams.lock().expect("seams lock");
            (seams.gate.clone(), seams.asker.clone())
        };
        let inputs = BatchInputs {
            registry: &self.registry,
            sink: self.sink.as_ref(),
            gate: gate.as_deref(),
            asker,
            stops: &self.stops,
            cancel: Some(&self.cancel),
        };
        let batch = execute::execute_batch(calls, &inputs);

        for (call, outcome) in calls.iter().zip(&batch.outcomes) {
            self.session.add(Message {
                role: Role::TOOL,
                content: outcome.output.clone(),
                raw_content: if outcome.raw_output.is_empty()
                    || outcome.raw_output == outcome.output
                {
                    String::new()
                } else {
                    outcome.raw_output.clone()
                },
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                tool_failure: execute::tool_failure(outcome),
                ..Message::default()
            });
        }
        // If the turn was cancelled during tool execution, return after
        // storing the batch results so the session keeps paired tool-call
        // history (Go: the ctx.Err() arm).
        if self.cancel.is_cancelled() {
            return ToolRound::Err(TurnError::Cancelled);
        }
        if !unavailable.is_empty() {
            if has_visible_final_answer(text) {
                return ToolRound::Final;
            }
            let nudge = format!(
                "The following tools are unavailable in the current workflow phase: {}. Do not call \
                 them again. Respond to the user's request with visible answer text now; call a \
                 different tool only if it is still needed to complete the request.",
                unavailable.join(", ")
            );
            self.add_host_message(&nudge);
            turn.lock().expect("turn lock").context_tool_repairs += 1;
        }
        ToolRound::Continue
    }

    /// Names the context-unavailable tools a batch calls (Go:
    /// `unavailableContextualToolCalls`): resolvable names whose
    /// [`reasonix_tools::ContextualTool`] capability answers not-visible.
    fn unavailable_contextual_tools(&self, calls: &[ToolCall]) -> Vec<String> {
        let ctx = reasonix_tools::CallContext::new();
        let mut names: Vec<String> = Vec::new();
        for call in calls {
            let Some(tool) = resolve_tool(&self.registry, &call.name) else {
                continue;
            };
            let Some(ct) = tool
                .capabilities()
                .get::<dyn reasonix_tools::ContextualTool>()
            else {
                continue;
            };
            if ct.provider_visible(&ctx) {
                continue;
            }
            if !names.iter().any(|n| n == tool.name()) {
                names.push(tool.name().to_owned());
            }
        }
        names
    }

    /// The single place a turn is told to stop using tools (Go:
    /// `armFinalizationRound`).
    fn arm_finalization_round(&self, turn: &Mutex<TurnState>, stop: &StopCause) {
        {
            let mut state = turn.lock().expect("turn lock");
            if state.grace_round {
                return;
            }
            state.grace_round = true;
            state.stop = Some(stop.clone());
        }
        let (text, detail) = match &stop {
            StopCause::MaxSteps { steps, key } => (
                "Tool round limit reached; asking the assistant to summarize progress.".to_owned(),
                format!("budget ({key}={steps}) exhausted: one grace round to finalize"),
            ),
            StopCause::Budget { axis, detail } => (
                "This task reached its spend budget; asking for a final answer.".to_owned(),
                format!("{axis}: {detail}"),
            ),
            StopCause::Final => (String::new(), String::new()),
        };
        self.add_host_message(&finalization_nudge(stop));
        self.notice(level::INFO, notice_code::TOOL_BUDGET, &text, &detail);
    }
}

enum Final {
    Answer,
    Continue,
    Pause(StopCause),
    Err(TurnError),
}

enum ToolRound {
    Continue,
    Final,
    Pause(StopCause),
    Err(TurnError),
}

/// Whether the active turn already carries a caller-armed pause (Go: the
/// `graceRound` check in `stopUnexecutedBoundaryCalls`).
fn grace_stop(turn: &Mutex<TurnState>, steps: i64, key: &str) -> Option<StopCause> {
    let state = turn.lock().expect("turn lock");
    state.grace_round.then(|| {
        state.stop.clone().unwrap_or_else(|| StopCause::MaxSteps {
            steps,
            key: key.to_owned(),
        })
    })
}

fn resolve_tool(registry: &Registry, name: &str) -> Option<Arc<dyn reasonix_tools::Tool>> {
    match registry.resolve_call(name) {
        reasonix_tools::registry::ResolveCall::Exact(t) => Some(t),
        _ => None,
    }
}

/// Go: `emptyFinalNoticeDetail`.
fn empty_final_notice_detail(
    model_ref: &str,
    usage: Option<&Usage>,
    reasoning_len: usize,
) -> String {
    let finish = usage
        .map(|u| u.finish_reason.clone())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "empty final answer blocked: {model_ref} returned no visible answer text (finish={finish}, \
         reasoning={reasoning_len} chars); retrying"
    )
}

/// Go: `emptyFinalRetryMessage`.
fn empty_final_retry_message(usage: Option<&Usage>) -> String {
    const OPENING: &str =
        "The previous assistant response finished without any visible answer text. ";
    if truncated_terminal(usage) {
        return format!(
            "{OPENING}Host fact, not a mistake in your work: it {} and was cut off while reasoning. \
             Continue the same task now with a shorter reasoning pass and give a concise visible \
             answer to the user. Do not send reasoning only.",
            truncation_cause(usage.map(|u| u.finish_reason.as_str()).unwrap_or(""))
        );
    }
    format!(
        "{OPENING}Continue the same task now and provide a concise visible answer to the user. Do \
         not send reasoning only."
    )
}

fn has_visible_final_answer(text: &str) -> bool {
    !text.trim().is_empty()
}

/// The refusal the grace round pairs unexecuted calls with (Go:
/// `pairUnexecutedGraceCalls`).
const GRACE_REFUSAL: &str =
    "blocked: the tool-call round budget is exhausted; no more tools will run in this turn";

/// The model-facing nudge the grace round lands (Go: `landCause.nudge`).
fn finalization_nudge(stop: &StopCause) -> String {
    const CLOSE: &str = "Do not call any more tools. Synthesize a final answer from the work \
         already completed: what was accomplished, what remains, and any decision the user should make.";
    match stop {
        StopCause::Budget { axis, .. } => format!(
            "This task has reached its {axis} budget. {CLOSE} Use the evidence already collected \
             and label what is still uncertain; the user can continue in the next message."
        ),
        StopCause::MaxSteps { key, .. } => format!(
            "Your tool-call round limit ({key}) has been reached. {CLOSE} The user can increase \
             {key} or continue in the next turn if more work is needed."
        ),
        StopCause::Final => CLOSE.to_owned(),
    }
}

fn new_stream_attempt_id(attempt: u32) -> String {
    use std::sync::atomic::AtomicU64;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    format!("sa-{attempt}-{seq}")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
