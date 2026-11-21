//! One tool round: dispatch, resolution, execution, output bounding (Go:
//! `execute_batch.go` + `execute_one.go`, the spine's subset).
//!
//! ToolDispatch events go out up front in call order; contiguous known
//! read-only calls fan out across threads while writers and unknown names
//! run serially so write/read ordering stays provider-ordered.
//! ToolResult events land after the batch, also in call order. After a
//! mutating call fails or is blocked, later mutations in the same batch are
//! skipped — read-only diagnosis still runs (Go: `mutationDependencyBarrier`).

use std::sync::{Arc, Mutex};
use std::thread;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::eventwire::tool::Tool as WireTool;
use reasonix_contract::provider::{ArgumentNormalization, ToolCall, ToolFailure};
use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::registry::ResolveCall;
use serde_json::Value;

use crate::cancel::{CallStops, STOPPED_CALL_OUTPUT};
use crate::sink::{Sink, issuer, kind};

/// Refusal identity of the calls the host stops before they run after a
/// failed change (Go: `CodeBatchSkippedAfterFailedChange`).
pub const CODE_BATCH_SKIPPED_AFTER_FAILED_CHANGE: &str = "batch.skipped_after_failed_change";

/// How many read-only calls may run at once (Go: `runParallel`'s
/// maxParallel).
const MAX_PARALLEL: usize = 8;

/// One tool call's result (Go: `toolOutcome`). `output` is the bounded form
/// the model sees; `raw_output` keeps the full original when bounding cut it.
#[derive(Debug, Default, Clone)]
pub struct ToolOutcome {
    pub output: String,
    pub raw_output: String,
    pub blocked: bool,
    pub err_msg: String,
    /// The identity of a host refusal, carried beside the words rather than
    /// recovered from them. Empty when the call was not refused.
    pub refusal_code: String,
    pub executed: bool,
    pub diff: String,
    pub added: i64,
    pub removed: i64,
}

impl ToolOutcome {
    /// Whether this outcome counts as a failure for the batch barrier (Go:
    /// `batchFailureBarrier`'s first question).
    fn failed(&self) -> bool {
        !self.err_msg.is_empty() || self.blocked
    }
}

/// What one thread of a parallel segment computes (Go: the `run` closure's
/// per-index writes).
struct PreparedCall {
    index: usize,
    tool: Arc<dyn Tool>,
    args: Value,
}

/// The result of one provider tool-call batch (Go: `batchExecution`).
#[derive(Debug, Default)]
pub struct BatchExecution {
    pub outcomes: Vec<ToolOutcome>,
}

/// What a dispatcher needs to run one batch. The gate seam rules before
/// every execution; `None` means no gating (Go: a nil gate). The asker rides
/// along because the `ask` tool reaches it through the call context, the way
/// Go carried it in `context.Context`.
pub struct BatchInputs<'a> {
    pub registry: &'a reasonix_tools::Registry,
    pub sink: &'a dyn Sink,
    pub gate: Option<&'a dyn crate::seams::Gate>,
    pub asker: Option<Arc<dyn crate::seams::Asker>>,
    pub stops: &'a CallStops,
    pub cancel: Option<&'a crate::cancel::CancelToken>,
}

/// The per-call context: the asker when the host installed one, nothing
/// otherwise (a headless run's tool that asks gets the Go-nil semantics). The
/// binding is the `Arc<dyn Asker>` itself, which is what
/// [`reasonix_tools::asker_from`] downcasts back to.
fn call_context(asker: Option<&Arc<dyn crate::seams::Asker>>) -> CallContext {
    match asker {
        Some(asker) => {
            let bound: Arc<dyn crate::seams::Asker> = asker.clone();
            CallContext::new().with_value(Arc::new(bound))
        }
        None => CallContext::new(),
    }
}

/// Records the schema-proven envelope unwrap on each call before anything
/// dispatches (Go: `prepareCallBatch`'s argument arm). The recorded
/// arguments stay what the model sent; `execution_arguments()` serves what
/// executes.
fn prepare_call_batch(calls: &mut [ToolCall], registry: &reasonix_tools::Registry) {
    for call in calls {
        let raw = call.arguments.clone();
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(value) = reasonix_tools::parse_unique(&raw) else {
            continue;
        };
        let Some(tool) = resolve(registry, call) else {
            continue;
        };
        let schema = tool.schema();
        if let Some(unwrapped) = reasonix_tools::unwrap_arguments(&schema, &value) {
            let effective = serde_json::to_string(&unwrapped).unwrap_or_else(|_| raw.clone());
            call.argument_normalization = Some(ArgumentNormalization {
                reason: reasonix_contract::provider::ARGUMENT_ENVELOPE_UNWRAPPED.to_owned(),
                original: raw,
                effective,
            });
        }
    }
}

/// Dispatches one model turn's tool calls (Go: `executeBatch`).
pub fn execute_batch(calls: &mut [ToolCall], inputs: &BatchInputs<'_>) -> BatchExecution {
    prepare_call_batch(calls, inputs.registry);
    for call in calls.iter() {
        emit_dispatch(inputs.sink, call, inputs.registry);
    }
    // The round ends at the first call whose result is a user decision (Go
    // `scheduleUpToDecision` + `markDeferredAfterDecision`). Calls written
    // after it were authored without the answer, read-only ones included, so
    // running any of them would act on a question still open.
    let scheduled = schedule_up_to_decision(inputs.registry, calls);
    let outcomes: Mutex<Vec<Option<ToolOutcome>>> =
        Mutex::new((0..calls.len()).map(|_| None).collect());
    if scheduled < calls.len() {
        let mut slots = outcomes.lock().expect("batch lock");
        for slot in slots.iter_mut().skip(scheduled) {
            *slot = Some(deferred_after_decision());
        }
    }
    let barrier = Mutex::new(false);

    for segment in partition_tool_calls(inputs.registry, &calls[..scheduled]) {
        if inputs.cancel.is_some_and(|c| c.is_cancelled()) {
            break;
        }
        match segment {
            Segment::Parallel { start, end } => {
                let prepared: Vec<PreparedCall> = (start..end)
                    .filter_map(|i| prepare(calls, inputs, &outcomes, i))
                    .collect();
                run_parallel(prepared, &outcomes, inputs.asker.as_ref());
            }
            Segment::Serial { index } => {
                if prepare(calls, inputs, &outcomes, index).is_none() {
                    continue;
                }
                if *barrier.lock().expect("barrier lock")
                    && !read_only_of(inputs.registry, &calls[index])
                {
                    outcomes.lock().expect("batch lock")[index] = Some(barrier_skip());
                    continue;
                }
                // `prepare` already ran the gate for this call; gating it
                // again would open a second prompt for the same call and a
                // first answer consumed by no waiter would be lost (Go:
                // one admission per call).
                let outcome = execute_one(&calls[index], inputs, true);
                if outcome.failed() && !read_only_of(inputs.registry, &calls[index]) {
                    *barrier.lock().expect("barrier lock") = true;
                }
                outcomes.lock().expect("batch lock")[index] = Some(outcome);
            }
        }
    }
    // A cancellation that reached the tail closes the calls it passed over
    // (Go: `markCancelledFrom`); a live barrier fills them with the
    // dependency refusal instead (Go: `markDependencySkipped`).
    let mut finished: Vec<ToolOutcome> = Vec::with_capacity(calls.len());
    let barrier_up = *barrier.lock().expect("barrier lock");
    let cancelled = inputs.cancel.is_some_and(|c| c.is_cancelled());
    {
        let mut slots = outcomes.lock().expect("batch lock");
        for (i, slot) in slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(if cancelled || inputs.stops.is_stopped(&calls[i].id) {
                    ToolOutcome {
                        output: STOPPED_CALL_OUTPUT.to_owned(),
                        err_msg: "cancelled".to_owned(),
                        ..ToolOutcome::default()
                    }
                } else if barrier_up && !read_only_of(inputs.registry, &calls[i]) {
                    barrier_skip()
                } else {
                    // A read-only call the barrier does not cover still runs:
                    // diagnosis from a known state (Go: the residual
                    // read-only arm). Unknown names keep the barrier answer.
                    // A call the loop passed over reaches the gate here for
                    // the first time (Go: the residual read-only arm).
                    execute_one(&calls[i], inputs, false)
                });
            }
            finished.push(slot.take().expect("filled above"));
        }
    }
    for (call, outcome) in calls.iter().zip(&finished) {
        emit_result(inputs.sink, call, outcome);
    }
    BatchExecution { outcomes: finished }
}

enum Segment {
    /// Contiguous known read-only calls (Go: `toolCallBatch{parallel: true}`).
    Parallel { start: usize, end: usize },
    /// A writer, an unresolvable name, or a tool that declares it needs its
    /// own place (Go: the single-call serial batch).
    Serial { index: usize },
}

/// Keeps provider order while letting contiguous read-only tools run
/// together (Go: `partitionToolCalls`). `runs_sequentially` opts a tool out
/// of shared batches independently of read-only.
fn partition_tool_calls(registry: &reasonix_tools::Registry, calls: &[ToolCall]) -> Vec<Segment> {
    let ctx = CallContext::new();
    let mut segments = Vec::new();
    let mut i = 0;
    while i < calls.len() {
        if parallelisable(registry, &calls[i], &ctx) {
            let start = i;
            i += 1;
            while i < calls.len() && parallelisable(registry, &calls[i], &ctx) {
                i += 1;
            }
            segments.push(Segment::Parallel { start, end: i });
            continue;
        }
        segments.push(Segment::Serial { index: i });
        i += 1;
    }
    segments
}

/// Read-only says a call needs no approval; it does not say the call may
/// share a batch (Go: `parallelisable`).
fn parallelisable(registry: &reasonix_tools::Registry, call: &ToolCall, ctx: &CallContext) -> bool {
    let Some(tool) = resolve(registry, call) else {
        return false;
    };
    let args = parse_raw(call.execution_arguments());
    if reasonix_tools::runs_sequentially(tool.as_ref(), ctx, &args) {
        return false;
    }
    tool.read_only()
}

fn resolve(registry: &reasonix_tools::Registry, call: &ToolCall) -> Option<Arc<dyn Tool>> {
    match registry.resolve_call(&call.name) {
        ResolveCall::Exact(t) => Some(t),
        ResolveCall::Ambiguous(_) | ResolveCall::Unknown => None,
    }
}

fn read_only_of(registry: &reasonix_tools::Registry, call: &ToolCall) -> bool {
    resolve(registry, call).is_some_and(|t| t.read_only())
}

fn parse_raw(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    reasonix_tools::parse_unique(raw).unwrap_or(Value::Null)
}

/// Fills a slot that already carries an outcome; `None` means the slot was
/// done and the caller skips it. The pre-execution gates (stop, cancel,
/// permission) run here too: a parallel read-only call passes through
/// exactly the same admission a serial one does (Go: `run`'s shared body).
fn prepare(
    calls: &[ToolCall],
    inputs: &BatchInputs<'_>,
    outcomes: &Mutex<Vec<Option<ToolOutcome>>>,
    index: usize,
) -> Option<PreparedCall> {
    if outcomes.lock().expect("batch lock")[index].is_some() {
        return None;
    }
    let call = &calls[index];
    if inputs.cancel.is_some_and(|c| c.is_cancelled()) || inputs.stops.is_stopped(&call.id) {
        outcomes.lock().expect("batch lock")[index] = Some(ToolOutcome {
            output: STOPPED_CALL_OUTPUT.to_owned(),
            err_msg: "cancelled".to_owned(),
            ..ToolOutcome::default()
        });
        return None;
    }
    let tool = resolve(inputs.registry, call)?;
    let args = match parse_arguments(call, &tool) {
        Ok(args) => args,
        Err(detail) => {
            outcomes.lock().expect("batch lock")[index] = Some(malformed_arguments_outcome(detail));
            return None;
        }
    };
    if let Some(gate) = inputs.gate {
        let verdict = gate.check_verdict(tool.name(), &args, tool.read_only());
        if !verdict.allow {
            let outcome = refused_outcome(
                &format!("blocked: {}", verdict.reason),
                &verdict.refusal_code,
            );
            outcomes.lock().expect("batch lock")[index] = Some(outcome);
            return None;
        }
    }
    Some(PreparedCall { index, tool, args })
}

/// Runs one parallel read-only segment (Go: `runParallel`), collecting by
/// index so result order stays provider-ordered.
fn run_parallel(
    prepared: Vec<PreparedCall>,
    outcomes: &Mutex<Vec<Option<ToolOutcome>>>,
    asker: Option<&Arc<dyn crate::seams::Asker>>,
) {
    let handles: Vec<thread::JoinHandle<(usize, ToolOutcome)>> = prepared
        .into_iter()
        .take(MAX_PARALLEL)
        .map(|p| {
            let ctx = call_context(asker);
            thread::spawn(move || {
                let outcome = match p.tool.execute(&ctx, &p.args) {
                    Ok(output) => {
                        let (body, raw, diff, added, removed) =
                            bound_tool_output(&p.tool, &ctx, &p.args, &output);
                        ToolOutcome {
                            output: body,
                            raw_output: raw,
                            executed: true,
                            diff,
                            added,
                            removed,
                            ..ToolOutcome::default()
                        }
                    }
                    Err(err) => failure_outcome(err.as_ref()),
                };
                (p.index, outcome)
            })
        })
        .collect();
    for handle in handles {
        let (index, outcome) = handle.join().expect("parallel tool thread");
        outcomes.lock().expect("batch lock")[index] = Some(outcome);
    }
}

/// Pre-fills one call the barrier covers (Go: `barrierSkip`).
fn barrier_skip() -> ToolOutcome {
    let msg = "blocked: skipped because an earlier modification in this tool batch failed or was \
               blocked. Fix or re-run the failed change first; verification was not executed.";
    ToolOutcome {
        output: msg.to_owned(),
        blocked: true,
        err_msg: msg.to_owned(),
        refusal_code: CODE_BATCH_SKIPPED_AFTER_FAILED_CHANGE.to_owned(),
        ..ToolOutcome::default()
    }
}

/// The exclusive end of the calls that may run: everything up to and
/// including the first decision barrier (Go `scheduleUpToDecision`'s
/// returned index plus one), or the whole batch when the reply carries none.
fn schedule_up_to_decision(registry: &reasonix_tools::Registry, calls: &[ToolCall]) -> usize {
    for (index, call) in calls.iter().enumerate() {
        let Some(tool) = resolve(registry, call) else {
            continue;
        };
        if reasonix_tools::is_decision_barrier(tool.as_ref()) {
            return index + 1;
        }
    }
    calls.len()
}

/// The refusal a call past a decision barrier carries (Go
/// `markDeferredAfterDecision`): it is not carried over, because the answer it
/// would have reasoned from did not exist when the model wrote it.
fn deferred_after_decision() -> ToolOutcome {
    const MSG: &str = "not run: this round stopped at a question for the user. It was written before \
                       their answer existed, so it is not carried over — read the answer, then decide \
                       what to do next.";
    ToolOutcome {
        output: MSG.to_owned(),
        blocked: true,
        err_msg: "not run: stopped at a user decision".to_owned(),
        ..ToolOutcome::default()
    }
}

/// Executes one call end to end (Go: `executeOne` — the spine's stages:
/// resolve, refuse the unknown, gate, parse, run, bound). `already_gated`
/// is true when [`prepare`] admitted this call, so the answer is not
/// requested twice for one call.
fn execute_one(call: &ToolCall, inputs: &BatchInputs<'_>, already_gated: bool) -> ToolOutcome {
    if inputs.cancel.is_some_and(|c| c.is_cancelled()) || inputs.stops.is_stopped(&call.id) {
        return ToolOutcome {
            output: STOPPED_CALL_OUTPUT.to_owned(),
            err_msg: "cancelled".to_owned(),
            ..ToolOutcome::default()
        };
    }
    let Some(tool) = resolve(inputs.registry, call) else {
        if let Some(refusal) = inputs.registry.call_refusal(&call.name) {
            return refused_outcome(&refusal.to_string(), &refusal.code);
        }
        return ToolOutcome {
            output: format!("error: unknown tool {:?}", call.name),
            err_msg: format!("unknown tool {:?}", call.name),
            ..ToolOutcome::default()
        };
    };
    let args = match parse_arguments(call, &tool) {
        Ok(args) => args,
        Err(detail) => return malformed_arguments_outcome(detail),
    };
    if !already_gated && let Some(gate) = inputs.gate {
        let verdict = gate.check_verdict(tool.name(), &args, tool.read_only());
        if !verdict.allow {
            return refused_outcome(
                &format!("blocked: {}", verdict.reason),
                &verdict.refusal_code,
            );
        }
    }
    let ctx = call_context(inputs.asker.as_ref());
    match tool.execute(&ctx, &args) {
        Ok(output) => {
            let (body, raw, diff, added, removed) = bound_tool_output(&tool, &ctx, &args, &output);
            ToolOutcome {
                output: body,
                raw_output: raw,
                executed: true,
                diff,
                added,
                removed,
                ..ToolOutcome::default()
            }
        }
        Err(err) => failure_outcome(err.as_ref()),
    }
}

/// One failed execution (Go: the error arm of `finishToolExecution`). A
/// tool that refused its own call never ran: blocked outcome, not an
/// execution failure.
fn failure_outcome(err: &(dyn std::error::Error + Send + Sync + 'static)) -> ToolOutcome {
    if let Some(message) = reasonix_tools::blocked_message(err) {
        return ToolOutcome {
            output: message.to_owned(),
            blocked: true,
            err_msg: message.to_owned(),
            executed: true,
            ..ToolOutcome::default()
        };
    }
    let refusal_code = err
        .downcast_ref::<reasonix_tools::Refusal>()
        .map(|r| r.code.clone())
        .unwrap_or_default();
    let raw_err = format!("error: {err}");
    let (body, raw) = bound_plain_output(&raw_err);
    ToolOutcome {
        output: body,
        raw_output: raw,
        executed: true,
        err_msg: err
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned(),
        refusal_code,
        ..ToolOutcome::default()
    }
}

fn malformed_arguments_outcome(detail: String) -> ToolOutcome {
    ToolOutcome {
        output: detail.clone(),
        err_msg: detail
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches("error: ")
            .to_owned(),
        ..ToolOutcome::default()
    }
}

fn refused_outcome(message: &str, code: &str) -> ToolOutcome {
    ToolOutcome {
        output: message.to_owned(),
        blocked: true,
        err_msg: message.lines().next().unwrap_or_default().to_owned(),
        refusal_code: code.to_owned(),
        ..ToolOutcome::default()
    }
}

/// Parses the model's raw arguments (Go: `prepareCallArguments` minus the
/// envelope unwrap, which runs once in `prepare_call_batch`). Malformed
/// arguments return the detail that echoes the tool's schema, so the retry
/// lands valid instead of repeating the broken shape (Go: the
/// malformed-arguments detail).
fn parse_arguments(call: &ToolCall, tool: &Arc<dyn Tool>) -> Result<Value, String> {
    let raw = call.execution_arguments();
    if raw.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    match reasonix_tools::parse_unique(raw) {
        Ok(value) => Ok(value),
        Err(err) => Err(format!(
            "error: invalid arguments: {err}\n{}",
            serde_json::to_string(&tool.schema()).unwrap_or_default()
        )),
    }
}

/// Bounds a successful output, computing the writer preview from the same
/// args execute received (Go: `boundToolOutput` + the preview arm).
fn bound_tool_output(
    tool: &Arc<dyn Tool>,
    ctx: &CallContext,
    args: &Value,
    output: &str,
) -> (String, String, String, i64, i64) {
    let mut diff = String::new();
    let mut added = 0;
    let mut removed = 0;
    if let Some(change) = reasonix_tools::preview_change(Some(tool), ctx, args) {
        diff = change.diff;
        added = change.added;
        removed = change.removed;
    }
    let (body, raw) = bound_plain_output(output);
    (body, raw, diff, added, removed)
}

/// The plain byte bound: over the cap, the head is kept and the cut named
/// (Go: `boundToolOutput`'s truncation arm; the spill-to-file arm is the
/// output-spill wave's machine and is not shaped here).
fn bound_plain_output(output: &str) -> (String, String) {
    if output.len() <= crate::config::MAX_TOOL_OUTPUT_BYTES {
        return (output.to_owned(), String::new());
    }
    let mut end = crate::config::MAX_TOOL_OUTPUT_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    let cut = output.len() - end;
    (
        format!(
            "{}\n[truncated {cut} bytes to fit the model context]",
            &output[..end]
        ),
        output.to_owned(),
    )
}

/// The eager dispatch card (Go: `emitEagerToolDispatch`).
fn emit_dispatch(sink: &dyn Sink, call: &ToolCall, registry: &reasonix_tools::Registry) {
    let read_only = resolve(registry, call).is_some_and(|t| t.read_only());
    sink.emit(Event {
        kind: kind::TOOL_DISPATCH.to_owned(),
        tool: Some(WireTool {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.execution_arguments().to_owned(),
            argument_normalization: call.argument_normalization.clone().map(|n| {
                reasonix_contract::eventwire::tool::ArgumentNormalization {
                    reason: n.reason,
                    original: n.original,
                    effective: n.effective,
                }
            }),
            read_only,
            issuer: issuer::MODEL.to_owned(),
            ..WireTool::default()
        }),
        ..Event::default()
    });
}

/// The result card, after the batch, in call order (Go: the
/// `executeBatch` tail).
fn emit_result(sink: &dyn Sink, call: &ToolCall, outcome: &ToolOutcome) {
    sink.emit(Event {
        kind: kind::TOOL_RESULT.to_owned(),
        tool: Some(WireTool {
            id: call.id.clone(),
            name: call.name.clone(),
            args: call.execution_arguments().to_owned(),
            output: outcome.output.clone(),
            err: outcome.err_msg.clone(),
            refusal_code: outcome.refusal_code.clone(),
            diff: outcome.diff.clone(),
            added: outcome.added,
            removed: outcome.removed,
            issuer: issuer::MODEL.to_owned(),
            ..WireTool::default()
        }),
        ..Event::default()
    });
}

/// The local metadata a failed or refused result carries into the transcript
/// (Go: `ToolFailure` on the tool message).
pub fn tool_failure(outcome: &ToolOutcome) -> Option<ToolFailure> {
    if outcome.err_msg.is_empty() && !outcome.blocked {
        return None;
    }
    Some(ToolFailure {
        refusal_code: outcome.refusal_code.clone(),
        blocked: outcome.blocked,
    })
}
