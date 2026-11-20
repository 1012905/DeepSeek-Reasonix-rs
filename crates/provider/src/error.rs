//! Typed provider failures (Go: `contract/provider` `streaminterrupt.go`,
//! `stream_payload.go`, `retry.go` error types, and `openai/stream_failure.go`).
//!
//! Identity is the variant plus the fixed interrupt cause, never the message:
//! a caller must tell auth, provider refusal, transport cut, and malformed
//! wire apart without matching text.

/// Fixed stream-interrupt reasons (Go: `StreamInterrupt*` constants). Closed
/// enum; never carries URLs, tool arguments, file paths, or raw error text.
pub const STREAM_INTERRUPT_CONNECTION_RESET: &str = "connection_reset";
pub const STREAM_INTERRUPT_PREMATURE_EOF: &str = "premature_eof";
pub const STREAM_INTERRUPT_IDLE_TIMEOUT: &str = "idle_timeout";
pub const STREAM_INTERRUPT_UPSTREAM_ERROR: &str = "upstream_error";

/// The far side failed before it said anything (Go: `errEmptyStream`).
pub(crate) const EMPTY_STREAM_MESSAGE: &str = "the upstream sent an empty stream";

/// Every failure a provider call can surface. Variants are the identity;
/// `Display` strings are diagnostics for humans.
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    /// A 401/403: the key was rejected or is missing (Go: `AuthError`).
    /// `body` is intentionally out of `Display` — servers echo masked key
    /// fragments in auth bodies and those must not reach logs.
    Auth {
        provider: String,
        key_env: String,
        key_source: String,
        status: u16,
        has_key: bool,
        body: String,
    },
    /// Any other non-OK status (Go: `APIError`).
    Api {
        provider: String,
        status: u16,
        body: String,
        trace_id: String,
        tool_context: String,
    },
    /// The endpoint answered 200 and then reported its own failure inside the
    /// stream (Go: `StreamPayloadError`); `message` stays the gateway's own
    /// sentence, `code`/`kind`/`param` its typed envelope fields.
    StreamPayload {
        provider: String,
        message: String,
        code: String,
        kind: String,
        param: String,
    },
    /// A `data:` frame failed to parse (Go: `StreamDecodeError`).
    StreamDecode {
        provider: String,
        payload: String,
        message: String,
    },
    /// The stream stopped before a clean terminal (Go:
    /// `StreamInterruptedError`). `reason` is one of the
    /// `STREAM_INTERRUPT_*` constants.
    StreamInterrupted { reason: String, message: String },
    /// The caller cancelled the attempt (Go: `context.Canceled`).
    Cancelled,
    /// One attempt waited past its deadline — response headers never arrived
    /// (Go: a `net.Error` timeout; the retry ladder owns those, and is a
    /// later slice).
    Timeout { message: String },
    /// Configuration failed validation before any request was built.
    Config { message: String },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Auth {
                provider,
                key_env,
                key_source,
                status,
                ..
            } => {
                let mut key = "the API key".to_owned();
                if !key_env.is_empty() {
                    key = key_env.clone();
                    if !key_source.is_empty() {
                        key = format!("{key} from {key_source}");
                    }
                }
                write!(
                    f,
                    "authentication failed for provider {provider:?} (HTTP {status}): {key} is \
                     invalid or expired — update it (in .env or your environment) and retry, or \
                     run `reasonix setup`"
                )
            }
            ProviderError::Api {
                provider,
                status,
                body,
                tool_context,
                ..
            } => {
                let base = if body.is_empty() {
                    format!("{provider}: status {status}")
                } else {
                    format!("{provider}: status {status}: {body}")
                };
                if tool_context.is_empty() {
                    write!(f, "{base}")
                } else {
                    write!(f, "{base}\n{tool_context}")
                }
            }
            ProviderError::StreamPayload {
                provider, message, ..
            } => {
                write!(f, "{provider}: {message}")
            }
            ProviderError::StreamDecode {
                provider,
                payload,
                message,
            } => write!(
                f,
                "{provider}: decode stream: {message} (payload {payload})"
            ),
            ProviderError::StreamInterrupted { message, .. } => write!(f, "{message}"),
            ProviderError::Cancelled => write!(f, "cancelled"),
            ProviderError::Timeout { message } => write!(f, "{message}"),
            ProviderError::Config { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    /// The fixed interrupt reason when this failure is a stream interruption
    /// (Go: `StreamInterruptReason`); empty otherwise.
    pub fn stream_interrupt_reason(&self) -> &str {
        match self {
            ProviderError::StreamInterrupted { reason, .. } => reason,
            _ => "",
        }
    }

    /// Whether the attempt never reached a clean provider terminal and the
    /// exact request may be replayed (Go: `IsStreamInterrupted`).
    pub fn is_stream_interrupted(&self) -> bool {
        matches!(self, ProviderError::StreamInterrupted { .. })
    }

    /// The HTTP status when this failure came from a status ladder decision
    /// (Go: `APIError.Status` / `AuthError.Status`); 0 otherwise.
    pub fn status(&self) -> u16 {
        match self {
            ProviderError::Auth { status, .. } | ProviderError::Api { status, .. } => *status,
            _ => 0,
        }
    }

    /// Whether the endpoint rejected the configured effort (Go:
    /// `ErrEffortRefused` — a provider refusal, not a config typo).
    pub fn is_effort_refused(&self) -> bool {
        matches!(self, ProviderError::Config { .. })
    }

    /// The machine-readable code of an OpenAI-shaped error envelope carried
    /// in a status-error body (Go: `APIError.ErrorCode`).
    pub fn error_code(&self) -> String {
        let ProviderError::Api { body, status, .. } = self else {
            return String::new();
        };
        // A retryable status is a busy server whatever code rides along.
        if retryable_status(*status) {
            return String::new();
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            return String::new();
        };
        let code = value
            .pointer("/error/code")
            .or_else(|| value.get("code"))
            .unwrap_or(&serde_json::Value::Null);
        match code {
            serde_json::Value::String(text) => text.clone(),
            _ => String::new(),
        }
    }
}

/// Whether a backoff can plausibly recover from status `s` (Go:
/// `RetryableStatus`): 408, 429 and 5xx. Other 4xx are caller/config problems.
pub fn retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

/// Wraps a failure as a stream interruption with a fixed reason (Go:
/// `StreamInterrupt`).
pub(crate) fn stream_interrupt(reason: &str, message: impl Into<String>) -> ProviderError {
    ProviderError::StreamInterrupted {
        reason: reason.to_owned(),
        message: message.into(),
    }
}

/// The idle watchdog fired: the only failure this read loop can name (Go:
/// `stallCut`).
pub(crate) fn stall_cut(provider: &str, idle: std::time::Duration) -> ProviderError {
    stream_interrupt(
        STREAM_INTERRUPT_IDLE_TIMEOUT,
        format!(
            "{provider}: stream stalled — no data for {idle:?}, connection likely dropped; \
             raise idle_timeout_seconds to allow a longer silence: unexpected EOF"
        ),
    )
}

/// A clean close before any terminal the protocol accepts (Go:
/// `prematureCut`).
pub(crate) fn premature_cut(provider: &str) -> ProviderError {
    stream_interrupt(
        STREAM_INTERRUPT_PREMATURE_EOF,
        format!("{provider}: stream ended before completion: unexpected EOF"),
    )
}

/// A read that failed after the response headers succeeded (Go: `ReadCut`).
pub(crate) fn read_cut(provider: &str, detail: &str, connection_reset: bool) -> ProviderError {
    let reason = if connection_reset {
        STREAM_INTERRUPT_CONNECTION_RESET
    } else {
        STREAM_INTERRUPT_PREMATURE_EOF
    };
    stream_interrupt(
        reason,
        format!("{provider}: read stream: {detail}: unexpected EOF"),
    )
}

/// Bounds a malformed-payload excerpt (Go: `streamPayloadSnippet`, 120 bytes).
pub(crate) fn payload_snippet(payload: &str) -> String {
    let payload = payload.trim();
    if payload.is_empty() {
        return "(empty)".to_owned();
    }
    if payload.len() > 120 {
        let mut end = 120;
        while end > 0 && !payload.is_char_boundary(end) {
            end -= 1;
        }
        return format!("{:?}…", &payload[..end]);
    }
    format!("{payload:?}")
}
