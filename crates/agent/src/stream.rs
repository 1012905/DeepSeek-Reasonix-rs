//! One provider completion collected from the chunk stream (Go: `stream`
//! + `streamedTurn`).
//!
//! Deltas reach the sink live (reasoning, text, partial tool dispatches)
//! while the complete tool calls accumulate in index order. Keeping the
//! result together makes the missing-reasoning / retry paths explicit: a
//! failed attempt commits nothing until a clean replacement is available.

use std::time::Instant;

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::eventwire::tool::Tool as WireTool;
use reasonix_contract::provider::ToolCall;
use reasonix_provider::{Chunk, ChunkStream, Usage};

use crate::cancel::CancelToken;
use crate::sink::{Sink, issuer, kind};

/// One sampled attempt (Go: `streamedTurn`).
#[derive(Debug, Default)]
pub struct StreamedTurn {
    pub text: String,
    pub reasoning: String,
    /// Host-measured stream duration in milliseconds (Go: `thoughtMs`).
    pub thought_ms: i64,
    pub calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    /// Peak streaming tool-argument size (Go: `maxArgChars`).
    pub max_arg_chars: usize,
    /// Whether a tool call began before the stream stopped.
    pub partial_tool_started: bool,
    pub err: Option<reasonix_provider::ProviderError>,
}

/// Consumes one `ChunkStream` to its terminal, emitting live deltas (Go:
/// `streamWithFrozen`'s chunk loop minus the extension/hook machines).
/// A turn cancellation counts as a stream failure carrying the partial
/// output, exactly as Go returns `ctx.Err()` from the chunk loop.
pub async fn consume_stream(
    mut stream: ChunkStream,
    sink: &dyn Sink,
    attempt_id: &str,
    cancel: Option<&CancelToken>,
) -> StreamedTurn {
    let begin = Instant::now();
    let mut out = StreamedTurn::default();
    // One loop, three terminals: clean end, typed provider failure, caller
    // cancellation. A cancelled select arm answers None like a clean end
    // does, so the token's flag — not the wake — says which one happened.
    let mut terminal: Result<(), reasonix_provider::ProviderError> = Ok(());
    loop {
        let chunk = if let Some(token) = cancel {
            tokio::select! {
                biased;
                _ = token.wait() => None,
                chunk = stream.next() => chunk,
            }
        } else {
            stream.next().await
        };
        match chunk {
            None => {
                if cancel.is_some_and(|t| t.is_cancelled()) {
                    terminal = Err(reasonix_provider::ProviderError::Cancelled);
                }
                break;
            }
            Some(Err(err)) => {
                terminal = Err(err);
                break;
            }
            Some(Ok(Chunk::Text(t))) => {
                out.text.push_str(&t);
                sink.emit(Event {
                    kind: kind::TEXT.to_owned(),
                    text: t,
                    ..Event::default()
                });
            }
            Some(Ok(Chunk::Reasoning(t))) => {
                out.reasoning.push_str(&t);
                sink.emit(Event {
                    kind: kind::REASONING.to_owned(),
                    text: t,
                    ..Event::default()
                });
            }
            Some(Ok(Chunk::ToolCallStart { id, name })) => {
                out.partial_tool_started = true;
                sink.emit(Event {
                    kind: kind::TOOL_DISPATCH.to_owned(),
                    tool: Some(partial_dispatch(&id, &name, attempt_id, 0)),
                    ..Event::default()
                });
            }
            Some(Ok(Chunk::ToolCallArgsDelta {
                id,
                name,
                arg_chars,
            })) => {
                out.partial_tool_started = true;
                if arg_chars > out.max_arg_chars {
                    out.max_arg_chars = arg_chars;
                }
                sink.emit(Event {
                    kind: kind::TOOL_DISPATCH.to_owned(),
                    tool: Some(partial_dispatch(&id, &name, attempt_id, arg_chars)),
                    ..Event::default()
                });
            }
            Some(Ok(Chunk::ToolCall(call))) => {
                out.partial_tool_started = true;
                if call.arguments.len() > out.max_arg_chars {
                    out.max_arg_chars = call.arguments.len();
                }
                out.calls.push(call);
            }
            Some(Ok(Chunk::Usage(u))) => {
                out.usage = Some(u);
            }
            Some(Ok(Chunk::Done)) => break,
        }
    }
    out.thought_ms = begin.elapsed().as_millis().clamp(1, i64::MAX as u128) as i64;
    out.err = terminal.err();
    out
}

/// The early dispatch card (Go: the `ChunkToolCallStart` /
/// `ChunkToolCallArgsDelta` emit): id, name, liveness — never args.
fn partial_dispatch(id: &str, name: &str, attempt_id: &str, arg_chars: usize) -> WireTool {
    WireTool {
        id: id.to_owned(),
        name: name.to_owned(),
        read_only: false,
        partial: true,
        arg_chars: arg_chars as i64,
        attempt_id: attempt_id.to_owned(),
        issuer: issuer::MODEL.to_owned(),
        ..WireTool::default()
    }
}
