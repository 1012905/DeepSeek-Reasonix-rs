//! Model provider adapters.
//!
//! Parity target: `reference/.../internal/model/openai` — the
//! OpenAI-compatible `/chat/completions` client: request encoding (byte-exact
//! per vendor dialect), SSE stream decoding, usage normalization, and typed
//! error classification. TLS is rustls only (AGENTS.md 3.3).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod client;
pub mod completion;
pub mod config;
pub mod effort;
pub mod error;
pub mod request;
pub mod stream;
pub mod think;
pub mod transport;
pub mod usage;
pub mod vendor;
pub mod wire;

pub use client::{ChunkStream, Client};
pub use completion::{CompletionResponse, decode_completion};
pub use config::{ClientConfig, DEFAULT_STREAM_IDLE_TIMEOUT, MAX_IDLE_TIMEOUT_SECONDS};
pub use error::{
    ProviderError, STREAM_INTERRUPT_CONNECTION_RESET, STREAM_INTERRUPT_IDLE_TIMEOUT,
    STREAM_INTERRUPT_PREMATURE_EOF, STREAM_INTERRUPT_UPSTREAM_ERROR,
};
pub use request::{ChatCompletionRequest, ToolSchema};
pub use stream::{Chunk, StreamDecoder};
pub use think::ThinkSplitter;
pub use usage::{Usage, WireUsage};
pub use vendor::{
    deepseek_prefix_chat_url, deepseek_takes_images, is_deepseek, is_gemini, is_kimi, is_mimo,
    is_minimax, is_minimax_flash, is_ollama_cloud, is_openai, is_zhipu, match_vendor_host,
    resolve_chat_url,
};
