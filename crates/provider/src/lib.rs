//! Model provider adapters.
//!
//! Parity target: `reference/.../internal/model` — the OpenAI-compatible,
//! Anthropic, Gemini, and Responses protocol adapters: streaming, incremental
//! tool calls, retries, timeouts, usage accounting, and error classification.
//! TLS is rustls only (AGENTS.md 3.3).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
