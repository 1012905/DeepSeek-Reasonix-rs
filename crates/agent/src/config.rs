//! Agent construction knobs (Go: `agentConfig` — the spine's subset).

/// Caps a single tool result before it goes into the model's context. ~32KB
/// is roughly 8K tokens — enough for a full file read or a busy grep, while
/// preventing one accidental "read this 5 MB log" from blowing the window
/// before the next compaction runs (Go: `MaxToolOutputBytes`).
pub const MAX_TOOL_OUTPUT_BYTES: usize = 32 * 1024;

/// Consecutive empty finals the host tolerates before failing the turn (Go:
/// `maxEmptyFinalBlocks`).
pub const MAX_EMPTY_FINAL_BLOCKS: i64 = 3;

/// Body-phase stream retries after the initial sampling attempt (Go:
/// `maxStreamRecoveries`; Codex-aligned 1 + 5 = 6 attempts total).
pub const MAX_STREAM_RECOVERIES: u32 = 5;
pub const MAX_SAMPLING_ATTEMPTS: u32 = MAX_STREAM_RECOVERIES + 1;

/// How the turn names its round ceiling in host notices (Go: `maxStepsKey`).
pub const DEFAULT_MAX_STEPS_KEY: &str = "max_steps";

/// Construction-time knobs. Field semantics mirror the Go options they
/// descend from; unset machines degrade rather than fake success.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// The model identity attached to assistant messages and usage events
    /// (Go: `modelRef`).
    pub model_ref: String,
    /// The cache-stable system prompt the default composer leads every
    /// request with (Go: the composed base prompt; the full composition is
    /// the prompt-composition wave's seam).
    pub system_prompt: String,
    /// Positive value caps tool rounds; the last one is a grace round asked
    /// to finalize. 0 is unbounded (Go: `maxSteps` — bounding is the host's
    /// call, as a budget rather than a round count, whenever it prefers).
    pub max_steps: i64,
    /// The name `max_steps` is reported under (Go: `maxStepsKey`).
    pub max_steps_key: String,
    /// Sampling temperature; `None` omits the field from the request.
    pub temperature: Option<f64>,
    /// Resolved total output budget; 0 lets the provider dialect decide.
    pub max_tokens: i64,
    /// Source label on emitted events (Go: `eventSource`).
    pub event_source: String,
    /// The backoff before replaying failed attempt n; tests pin it to zero
    /// (Go: the `streamRetrySleep` seam). `None` keeps the production
    /// ladder: 0.5s, 1s, 2s, 4s, 8s with jitter.
    pub retry_backoff: Option<std::time::Duration>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            model_ref: String::new(),
            system_prompt: String::new(),
            max_steps: 0,
            max_steps_key: DEFAULT_MAX_STEPS_KEY.to_owned(),
            temperature: None,
            max_tokens: 0,
            event_source: String::new(),
            retry_backoff: None,
        }
    }
}
