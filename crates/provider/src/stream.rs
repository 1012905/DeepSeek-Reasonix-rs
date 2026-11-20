//! SSE stream decoding (Go: `openai/openai.go` `readStream` minus the channel
//! and watchdog plumbing, which live in [`crate::transport`]).
//!
//! Text deltas stream live, tool-call fragments accumulate by index and emit
//! complete on `[DONE]`, and a start fires the moment a call's name is known.

use std::collections::BTreeMap;

use reasonix_contract::provider::ToolCall;

use crate::error::{
    EMPTY_STREAM_MESSAGE, ProviderError, STREAM_INTERRUPT_UPSTREAM_ERROR, payload_snippet,
    premature_cut, stream_interrupt,
};
use crate::think::ThinkSplitter;
use crate::usage::{Usage, normalize_usage};
use crate::wire::{ChatToolCall, StreamResponse, decode_stream_frame};

/// bufio.Scanner's upper bound (Go: `scanner.Buffer(64KiB, 1MiB)`); a longer
/// line is a read cut, not a hang.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Progress tick bucket while large arguments stream (Go: `argBucket` — one
/// chunk per 2KB so consumers see liveness without per-delta spam).
const ARG_PROGRESS_BUCKET: usize = 2048;

/// One streamed increment (Go: `provider.Chunk`; error rides the final
/// [`ChunkStream`] item instead of a chunk variant).
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Text(String),
    Reasoning(String),
    /// A tool call has begun; args still streaming.
    ToolCallStart {
        id: String,
        name: String,
    },
    /// Liveness while a call's arguments stream; arg_chars is cumulative.
    ToolCallArgsDelta {
        id: String,
        name: String,
        arg_chars: usize,
    },
    /// One complete tool call, in index order.
    ToolCall(ToolCall),
    Usage(Usage),
    /// The completion finished normally.
    Done,
}

#[derive(Debug, Default)]
struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
    thought_signature: String,
}

/// Pull-based decoder for one SSE response body. Feed bytes as they arrive,
/// then [`StreamDecoder::finish`] at EOF; [`StreamDecoder::outcome`] names
/// what the finished read means (Go: `streamOutcome`).
#[derive(Debug)]
pub struct StreamDecoder {
    provider: String,
    line: Vec<u8>,
    saw_done: bool,
    last_finish_reason: String,
    emitted_any: bool,
    error: Option<ProviderError>,
    think: ThinkSplitter,
    acc: BTreeMap<i64, ToolCallAccum>,
    started: BTreeMap<i64, bool>,
    arg_bucket: BTreeMap<i64, usize>,
    finished: bool,
}

impl StreamDecoder {
    /// A decoder for one stream of `provider`.
    pub fn new(provider: &str) -> StreamDecoder {
        StreamDecoder {
            provider: provider.to_owned(),
            line: Vec::new(),
            saw_done: false,
            last_finish_reason: String::new(),
            emitted_any: false,
            error: None,
            think: ThinkSplitter::default(),
            acc: BTreeMap::new(),
            started: BTreeMap::new(),
            arg_bucket: BTreeMap::new(),
            finished: false,
        }
    }

    /// Whether a fatal error already stopped this stream.
    pub fn is_failed(&self) -> bool {
        self.error.is_some()
    }

    /// Feeds body bytes and drains the chunks they completed. Feeding after a
    /// fatal error or finish is a no-op.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Chunk> {
        if self.error.is_some() || self.finished {
            return Vec::new();
        }
        let mut out = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.handle_line(&line, &mut out);
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    self.error = Some(crate::error::read_cut(
                        &self.provider,
                        "token too long",
                        false,
                    ));
                    return out;
                }
                self.line.push(*byte);
            }
            if self.saw_done || self.error.is_some() {
                break;
            }
        }
        out
    }

    /// Ends the stream: flushes the tail line and the think splitter, then
    /// emits complete tool calls in index order and `Done` — or nothing when
    /// a terminal is missing (Go: the tail of `readStream`).
    pub fn finish(&mut self) -> Vec<Chunk> {
        if self.error.is_some() || self.finished {
            return Vec::new();
        }
        let mut out = Vec::new();
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.handle_line(&line, &mut out);
        }
        if self.error.is_some() {
            self.finished = true;
            return out;
        }
        if !self.saw_done && self.last_finish_reason.is_empty() {
            // A proxy that idle-closes with a clean FIN ends the scan with no
            // error; without this the turn would be committed as complete —
            // including half-streamed tool-call arguments (#3953).
            self.error = Some(premature_cut(&self.provider));
            self.finished = true;
            return out;
        }
        let (reasoning, text) = self.think.flush();
        if !reasoning.is_empty() {
            self.emitted_any = true;
            out.push(Chunk::Reasoning(reasoning));
        }
        if !text.is_empty() {
            self.emitted_any = true;
            out.push(Chunk::Text(text));
        }
        // BTreeMap iterates in index order, the sorted order Go emits.
        for (idx_value, acc) in self.acc.iter_mut() {
            if acc.id.is_empty() {
                // Gateways that stream tool calls by index with no id get a
                // stable synthesized one: an empty tool_call_id collapses
                // multi-tool turns downstream.
                acc.id = format!("call_{idx_value}");
            }
            let acc = std::mem::take(acc);
            out.push(Chunk::ToolCall(ToolCall {
                id: acc.id,
                name: acc.name,
                arguments: acc.arguments,
                thought_signature: acc.thought_signature,
                ..ToolCall::default()
            }));
        }
        out.push(Chunk::Done);
        self.finished = true;
        out
    }

    /// The typed outcome of the finished read (Go: `streamOutcome` +
    /// `streamFailure`). None means the stream reached a clean terminal.
    pub fn outcome(&self) -> Option<ProviderError> {
        if let Some(err) = &self.error {
            return Some(classify(self.emitted_any, err.clone()));
        }
        if !self.emitted_any {
            // Nothing carried at all is the far side's failure, not the model
            // answering with nothing.
            return Some(stream_interrupt(
                STREAM_INTERRUPT_UPSTREAM_ERROR,
                format!("{}: {EMPTY_STREAM_MESSAGE}", self.provider),
            ));
        }
        None
    }

    fn handle_line(&mut self, raw: &[u8], out: &mut Vec<Chunk>) {
        if self.error.is_some() {
            return;
        }
        let line = String::from_utf8_lossy(raw);
        let line = line.trim();
        if line.is_empty() || !line.starts_with("data:") {
            return;
        }
        let data = line["data:".len()..].trim();
        if data == "[DONE]" {
            self.saw_done = true;
            return;
        }
        if data.is_empty() {
            return;
        }
        let sr: StreamResponse = match decode_stream_frame(data) {
            Ok(sr) => sr,
            Err(err) => {
                self.error = Some(ProviderError::StreamDecode {
                    provider: self.provider.clone(),
                    payload: payload_snippet(data),
                    message: err.to_string(),
                });
                return;
            }
        };
        if let Some(wire_error) = &sr.error {
            // A gateway reporting its own failure inside a stream it already
            // answered 200 to travels as a type, not by its wording.
            self.error = Some(ProviderError::StreamPayload {
                provider: self.provider.clone(),
                message: wire_error.message.clone(),
                code: wire_error.code.clone(),
                kind: wire_error.kind.clone(),
                param: wire_error.param.clone(),
            });
            return;
        }
        if let Some(choice) = sr.choices.first()
            && let Some(reason) = choice.finish_reason.as_ref().filter(|r| !r.is_empty())
        {
            self.last_finish_reason = reason.clone();
        }
        if let Some(usage) = &sr.usage {
            let mut u = normalize_usage(usage);
            u.finish_reason = self.last_finish_reason.clone();
            self.emitted_any = true;
            out.push(Chunk::Usage(u));
        }
        let Some(choice) = sr.choices.first() else {
            return;
        };
        let delta = &choice.delta;
        let mut reasoning_delta = delta.reasoning_content.clone();
        if reasoning_delta.is_empty() {
            reasoning_delta = delta.reasoning.clone();
        }
        if !reasoning_delta.is_empty() {
            self.emitted_any = true;
            out.push(Chunk::Reasoning(reasoning_delta));
        }
        if !delta.content.is_empty() {
            let (reasoning, text) = self.think.push(&delta.content);
            if !reasoning.is_empty() {
                self.emitted_any = true;
                out.push(Chunk::Reasoning(reasoning));
            }
            if !text.is_empty() {
                self.emitted_any = true;
                out.push(Chunk::Text(text));
            }
        }
        for tc in &delta.tool_calls {
            self.accumulate(tc, out);
        }
    }

    fn accumulate(&mut self, tc: &ChatToolCall, out: &mut Vec<Chunk>) {
        let entry = self.acc.entry(tc.index).or_default();
        if !tc.id.is_empty() {
            entry.id = tc.id.clone();
        }
        if !tc.function.name.is_empty() {
            entry.name = tc.function.name.clone();
        }
        entry.arguments.push_str(&tc.function.arguments);
        let mut thought_signature = tc.extra_content.as_ref().map_or(String::new(), |extra| {
            extra.google.thought_signature.clone()
        });
        if thought_signature.is_empty() {
            // Early Gemini responses placed the field in function; accept
            // that shape when replaying older sessions (decode-only).
            thought_signature = tc.function.thought_signature.clone();
        }
        if !thought_signature.is_empty() {
            entry.thought_signature = thought_signature;
        }
        let started = self.started.entry(tc.index).or_default();
        if !*started && !entry.name.is_empty() {
            // Signal the call's start the moment its name is known.
            *started = true;
            self.emitted_any = true;
            out.push(Chunk::ToolCallStart {
                id: entry.id.clone(),
                name: entry.name.clone(),
            });
        }
        if *started {
            let bucket = entry.arguments.len() / ARG_PROGRESS_BUCKET;
            let last = self.arg_bucket.entry(tc.index).or_default();
            if bucket > *last {
                *last = bucket;
                self.emitted_any = true;
                out.push(Chunk::ToolCallArgsDelta {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    arg_chars: entry.arguments.len(),
                });
            }
        }
    }
}

/// Names what a finished read means (Go: `streamOutcome` + `streamFailure`):
/// a payload failure before anything was said replays like a transport cut;
/// after output it stays terminal.
pub(crate) fn classify(emitted_any: bool, err: ProviderError) -> ProviderError {
    match err {
        ProviderError::StreamPayload {
            message, provider, ..
        } if !emitted_any => stream_interrupt(
            STREAM_INTERRUPT_UPSTREAM_ERROR,
            format!("{provider}: {message}"),
        ),
        other => other,
    }
}
