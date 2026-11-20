//! The OpenAI-compatible chat client (Go: `openai/client`): request build,
//! headers, transport, and the decoded chunk stream.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use hyper::body::Incoming;

use crate::completion::{CompletionResponse, decode_completion};
use crate::error::{ProviderError, stall_cut};
use crate::request::{ChatCompletionRequest, Dialect, build_chat_request};
use crate::stream::{Chunk, StreamDecoder};
use crate::transport::{self, AuthMeta, Transport};
use crate::wire::ChatRequest;

/// The OpenAI-compatible chat-completions client for one resolved endpoint.
#[derive(Clone)]
pub struct Client {
    dialect: Dialect,
    chat_url: String,
    prefix_chat_url: Option<String>,
    headers: BTreeMap<String, String>,
    api_key: String,
    key_env: String,
    key_source: String,
    idle_timeout: Duration,
    transport: Transport,
}

impl Client {
    /// Assembles a client from resolved facts (Go: `New`); build one through
    /// [`crate::config::build_client`], which owns transport construction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        dialect: Dialect,
        chat_url: String,
        prefix_chat_url: Option<String>,
        headers: BTreeMap<String, String>,
        api_key: String,
        key_env: String,
        key_source: String,
        idle_timeout: Duration,
        transport: Transport,
    ) -> Client {
        Client {
            dialect,
            chat_url,
            prefix_chat_url,
            headers,
            api_key,
            key_env,
            key_source,
            idle_timeout,
            transport,
        }
    }

    /// The provider instance name (Go: `Name`).
    pub fn name(&self) -> &str {
        &self.dialect.name
    }

    /// The POST target for chat completions.
    pub fn chat_url(&self) -> &str {
        &self.chat_url
    }

    /// The official DeepSeek Beta endpoint for prefix continuation, when the
    /// endpoint is DeepSeek's own (Go: `prefixChatURL`); continuation itself
    /// is a later slice.
    pub fn prefix_chat_url(&self) -> Option<&str> {
        self.prefix_chat_url.as_deref()
    }

    /// The depth levels a per-request effort override may take (Go:
    /// `RequestEfforts`); empty means overrides are ignored.
    pub fn request_efforts(&self) -> &[String] {
        &self.dialect.request_efforts
    }

    /// Whether thinking-mode tool_calls turns must carry the reasoning key
    /// (Go: `RequiresToolCallReasoning`).
    pub fn requires_tool_call_reasoning(&self) -> bool {
        crate::request::requires_tool_call_reasoning(&self.dialect)
    }

    /// Whether complete assistant reasoning replays on multi-turn requests
    /// (Go: `RequiresReasoningRoundTrip`).
    pub fn requires_reasoning_round_trip(&self) -> bool {
        crate::request::requires_reasoning_round_trip(&self.dialect)
    }

    /// Builds the wire request (Go: `buildRequest`), exposed for parity tests
    /// and the future continuation wrapper.
    pub fn build_request(&self, req: &ChatCompletionRequest) -> ChatRequest {
        build_chat_request(&self.dialect, req)
    }

    /// The exact HTTP body one request would send, newline included (Go:
    /// `openStream`'s marshalled body).
    pub fn request_bytes(&self, req: &ChatCompletionRequest) -> Result<Vec<u8>, serde_json::Error> {
        self.build_request(req).encode_body()
    }

    /// The per-provider stream idle window (Go: `idleTimeout`).
    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// Streams one chat completion: OK response or a typed pre-stream error
    /// (Go: `Stream` minus the prefix-continuation wrapper).
    pub async fn stream(&self, req: &ChatCompletionRequest) -> Result<ChunkStream, ProviderError> {
        let wire = self.build_request(req);
        let body = wire.encode_body().map_err(|err| ProviderError::Config {
            message: format!("openai: {}: marshal request: {err}", self.dialect.name),
        })?;
        let headers = transport::chat_headers(
            &self.dialect.base_url,
            &self.chat_url,
            &self.api_key,
            &self.key_env,
            &self.key_source,
            &self.headers,
            true,
        )?;
        let auth = AuthMeta {
            key_env: self.key_env.clone(),
            key_source: self.key_source.clone(),
            has_key: !self.api_key.is_empty(),
        };
        let incoming = self
            .transport
            .post(
                &self.dialect.name,
                &self.chat_url,
                headers,
                body,
                self.idle_timeout,
                &auth,
            )
            .await?;
        Ok(ChunkStream {
            provider: self.dialect.name.clone(),
            body: Some(incoming),
            decoder: StreamDecoder::new(&self.dialect.name),
            idle_timeout: self.idle_timeout,
            queue: VecDeque::new(),
            final_error: None,
            done: false,
        })
    }

    /// One non-streaming completion (extension beyond the Go adapter, which
    /// always streams): the same dialect and usage normalization against the
    /// one-shot body.
    pub async fn complete(
        &self,
        req: &ChatCompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let mut wire = self.build_request(req);
        wire.stream = false;
        wire.stream_options = None;
        let body = wire.encode_body().map_err(|err| ProviderError::Config {
            message: format!("openai: {}: marshal request: {err}", self.dialect.name),
        })?;
        let headers = transport::chat_headers(
            &self.dialect.base_url,
            &self.chat_url,
            &self.api_key,
            &self.key_env,
            &self.key_source,
            &self.headers,
            false,
        )?;
        let auth = AuthMeta {
            key_env: self.key_env.clone(),
            key_source: self.key_source.clone(),
            has_key: !self.api_key.is_empty(),
        };
        let raw = self
            .transport
            .post_once(
                &self.dialect.name,
                &self.chat_url,
                headers,
                body,
                self.idle_timeout,
                &auth,
            )
            .await?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        decode_completion(&text).map_err(|err| ProviderError::StreamDecode {
            provider: self.dialect.name.clone(),
            payload: crate::error::payload_snippet(&text),
            message: err.to_string(),
        })
    }
}

/// Decoded chunks of one streamed completion. The final item is
/// `Err(ProviderError)` when the stream failed, mirroring Go's terminal
/// `ChunkError`; a clean stream ends after [`Chunk::Done`].
pub struct ChunkStream {
    provider: String,
    body: Option<Incoming>,
    decoder: StreamDecoder,
    idle_timeout: Duration,
    queue: VecDeque<Chunk>,
    final_error: Option<ProviderError>,
    done: bool,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("name", &self.dialect.name)
            .field("chat_url", &self.chat_url)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ChunkStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkStream")
            .field("provider", &self.provider)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl ChunkStream {
    /// The next chunk, or None when the stream ended.
    pub async fn next(&mut self) -> Option<Result<Chunk, ProviderError>> {
        loop {
            if let Some(chunk) = self.queue.pop_front() {
                return Some(Ok(chunk));
            }
            if self.done {
                if let Some(err) = self.final_error.take() {
                    return Some(Err(err));
                }
                return None;
            }
            if self.decoder.is_failed() {
                // The decoder already named the failure; the tail stays
                // unflushed, exactly as Go returns from readStream.
                self.done = true;
                self.final_error = self.decoder.outcome();
                continue;
            }
            let Some(body) = self.body.as_mut() else {
                self.done = true;
                continue;
            };
            let idle = self.idle_timeout;
            match tokio::time::timeout(idle, transport::next_body_chunk(body)).await {
                Err(_) => {
                    // A body-phase cut the watchdog named; the Agent may
                    // replay the frozen request.
                    self.done = true;
                    self.final_error = Some(stall_cut(&self.provider, idle));
                    self.body = None;
                }
                Ok(None) => {
                    let tail = self.decoder.finish();
                    self.queue.extend(tail);
                    self.done = true;
                    self.final_error = self.decoder.outcome();
                    self.body = None;
                }
                Ok(Some(Err(err))) => {
                    self.done = true;
                    self.final_error = Some(err);
                    self.body = None;
                }
                Ok(Some(Ok(bytes))) => {
                    let chunks = self.decoder.feed(&bytes);
                    self.queue.extend(chunks);
                }
            }
        }
    }
}
