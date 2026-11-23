//! JSON-RPC 2.0 over NDJSON framing (contract §1, §13).
//!
//! One complete JSON object per line on stdin/stdout. Frames are capped in
//! both directions; line reading trims surrounding whitespace and a trailing
//! CR/LF exactly as `rpcwire.readLine` does.

use crate::error::{FrameTooLargeError, OutboundQueueFullError, ResponseError, rpc_code};
use crate::rawjson::RawJson;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::sync::Mutex;

/// A decoded JSON-RPC request ID: string, integer, or null.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestId {
    /// A JSON string ID.
    String(String),
    /// An integer ID (matches Go's `int64` request IDs).
    Integer(i64),
    /// A JSON null ID.
    Null,
}

impl RequestId {
    fn from_raw(raw: &RawJson) -> Option<Self> {
        let text = raw.get();
        if text == "null" {
            return Some(Self::Null);
        }
        if let Ok(n) = serde_json::from_str::<i64>(text) {
            return Some(Self::Integer(n));
        }
        if let Ok(s) = serde_json::from_str::<String>(text) {
            return Some(Self::String(s));
        }
        None
    }

    fn is_safe(raw: &RawJson) -> bool {
        Self::from_raw(raw).is_some()
    }
}

impl Serialize for RequestId {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::String(s) => ser.serialize_str(s),
            Self::Integer(n) => ser.serialize_i64(*n),
            Self::Null => ser.serialize_none(),
        }
    }
}

/// A validated JSON-RPC error object on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<RawJson>,
}

/// One parsed inbound frame. Exactly one of request/response shapes is valid.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// A request: has method and id, may carry params.
    Request {
        jsonrpc: String,
        id: RequestId,
        method: String,
        params: Option<RawJson>,
    },
    /// A notification: has method, no id.
    Notification {
        jsonrpc: String,
        method: String,
        params: Option<RawJson>,
    },
    /// A response: has id and exactly one of result/error.
    Response {
        jsonrpc: String,
        id: RequestId,
        result: Option<RawJson>,
        error: Option<ErrorObject>,
    },
}

/// Why a frame failed validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("rpcwire: parse error")]
    Parse,
    #[error("rpcwire: invalid request: {0}")]
    InvalidRequest(&'static str),
    #[error("rpcwire: frame is empty")]
    Empty,
}

/// Wire shape used to decode and validate an inbound line.
#[derive(Debug, Deserialize)]
struct Inbound {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<RawJson>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<RawJson>,
    #[serde(default)]
    result: Option<RawJson>,
    #[serde(default)]
    error: Option<ErrorObject>,
}

/// Decode one NDJSON line into a strict frame.
///
/// Tolerant at the frame level (unknown members are ignored) but strict about
/// JSON-RPC shape, exactly like `rpcwire.validateStrictFrame`.
pub fn decode_frame(line: &str) -> Result<Frame, FrameError> {
    let inbound: Inbound = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Err(FrameError::Parse),
    };
    validate_strict(&inbound)?;
    let jsonrpc = inbound.jsonrpc;
    let has_method = inbound.method.as_ref().is_some_and(|m| !m.is_empty());
    match (has_method, inbound.id) {
        (true, Some(id)) => Ok(Frame::Request {
            jsonrpc,
            id: RequestId::from_raw(&id).expect("validated id"),
            method: inbound.method.expect("has method"),
            params: inbound.params,
        }),
        (true, None) => Ok(Frame::Notification {
            jsonrpc,
            method: inbound.method.expect("has method"),
            params: inbound.params,
        }),
        (false, Some(id)) => Ok(Frame::Response {
            jsonrpc,
            id: RequestId::from_raw(&id).expect("validated id"),
            result: inbound.result,
            error: inbound.error,
        }),
        (false, None) => Err(FrameError::InvalidRequest("missing method and id")),
    }
}

fn validate_strict(inbound: &Inbound) -> Result<(), FrameError> {
    if inbound.jsonrpc != "2.0" {
        return Err(FrameError::InvalidRequest("jsonrpc must be 2.0"));
    }
    let has_id = inbound.id.is_some();
    if has_id && !inbound.id.as_ref().is_some_and(RequestId::is_safe) {
        return Err(FrameError::InvalidRequest(
            "id must be a string, integer, or null",
        ));
    }
    if inbound.method.as_ref().is_some_and(|m| !m.is_empty()) {
        if inbound.result.is_some() || inbound.error.is_some() {
            return Err(FrameError::InvalidRequest("invalid request shape"));
        }
        if let Some(params) = &inbound.params {
            let text = params.get().trim();
            if !text.starts_with('{') && !text.starts_with('[') {
                return Err(FrameError::InvalidRequest("params must be object or array"));
            }
        }
        return Ok(());
    }
    if !has_id || inbound.params.is_some() || inbound.result.is_some() == inbound.error.is_some() {
        return Err(FrameError::InvalidRequest("invalid response shape"));
    }
    Ok(())
}

/// Returns an ID safe to place in an error response.
///
/// Invalid IDs become null so the error response itself stays valid; valid
/// string, integer, and null IDs are retained verbatim.
pub fn response_id_for_error(raw: &str) -> String {
    let trimmed = raw.trim();
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return "null".into(),
    };
    let safe = match &value {
        serde_json::Value::Null => true,
        serde_json::Value::String(_) => true,
        serde_json::Value::Number(n) => n.is_i64(),
        _ => false,
    };
    if safe {
        trimmed.to_owned()
    } else {
        "null".into()
    }
}

/// Why reading a frame failed. I/O failure and budget violation stay distinct
/// so a caller can tell a broken pipe from an oversized frame by identity.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("rpcwire: read failed: {0}")]
    Io(#[source] io::Error),
    #[error(transparent)]
    TooLarge(#[from] FrameTooLargeError),
}

/// Read frames from any `BufRead`, enforcing the inbound byte budget.
///
/// Returns `Ok(None)` on clean EOF. The first string excludes the trailing
/// CR/LF and surrounding space/tab; the second retains the exact consumed
/// bytes for a bootstrap proxy to forward unchanged.
pub fn read_frame(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<(String, String)>, ReadError> {
    let mut buf = Vec::new();
    loop {
        let mut chunk = Vec::new();
        let n = reader
            .read_until(b'\n', &mut chunk)
            .map_err(ReadError::Io)?;
        buf.extend_from_slice(&chunk);
        if max_bytes > 0 && buf.len() > max_bytes {
            return Err(ReadError::TooLarge(FrameTooLargeError {
                direction: crate::error::Direction::Inbound,
                size: buf.len(),
                limit: max_bytes,
            }));
        }
        if n == 0 || chunk.last() == Some(&b'\n') {
            break;
        }
    }
    if buf.is_empty() {
        return Ok(None);
    }
    let mut end = buf.len();
    while end > 0 && (buf[end - 1] == b'\n' || buf[end - 1] == b'\r') {
        end -= 1;
    }
    let mut start = 0;
    while start < end && is_space(buf[start]) {
        start += 1;
    }
    while end > start && is_space(buf[end - 1]) {
        end -= 1;
    }
    let raw = String::from_utf8_lossy(&buf[start..end]).into_owned();
    Ok(Some((raw, String::from_utf8_lossy(&buf).into_owned())))
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// Encode a serializable value as one NDJSON frame.
///
/// Go's `json.Encoder` appends a newline and disables HTML escaping on the
/// extension transport; serde_json matches the compact form. The returned
/// bytes include the trailing newline and are checked against `max_bytes`.
pub fn encode_frame<T: Serialize>(
    value: &T,
    max_bytes: usize,
) -> Result<Vec<u8>, FrameTooLargeError> {
    let mut bytes = serde_json::to_vec(value).expect("frame values are serializable");
    bytes.push(b'\n');
    if max_bytes > 0 && bytes.len() > max_bytes {
        return Err(FrameTooLargeError {
            direction: crate::error::Direction::Outbound,
            size: bytes.len(),
            limit: max_bytes,
        });
    }
    Ok(bytes)
}

/// Owns a writer, serializing frames so two callers can never interleave.
///
/// A mutex stands in for Go's single writer goroutine; frames are written
/// whole and in order. Writing is infallible at the framing layer beyond
/// `io::Error`, and the last I/O error is retained as the connection's
/// terminal cause.
#[derive(Debug)]
pub struct FrameWriter<W: Write> {
    inner: Mutex<WriterState<W>>,
    max_bytes: usize,
    best_effort_limit: usize,
    best_effort_used: Mutex<usize>,
}

#[derive(Debug)]
struct WriterState<W: Write> {
    writer: W,
    terminal_error: Option<io::ErrorKind>,
}

impl<W: Write> FrameWriter<W> {
    /// Creates a writer with a per-frame byte budget.
    pub fn new(writer: W, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(WriterState {
                writer,
                terminal_error: None,
            }),
            max_bytes,
            best_effort_limit: 16,
            best_effort_used: Mutex::new(0),
        }
    }

    /// Writes one frame atomically, returning the number of bytes written.
    pub fn write_frame<T: Serialize>(&self, value: &T) -> Result<usize, WriteError> {
        let bytes = encode_frame(value, self.max_bytes)?;
        let mut state = self.inner.lock().expect("frame writer mutex poisoned");
        if let Some(kind) = state.terminal_error {
            return Err(WriteError::Io(io::Error::new(kind, "connection failed")));
        }
        state.writer.write_all(&bytes).map_err(|e| {
            state.terminal_error = Some(e.kind());
            WriteError::Io(e)
        })?;
        state.writer.flush().map_err(WriteError::Io)?;
        Ok(bytes.len())
    }

    /// Writes one already-encoded frame (trailing newline included).
    ///
    /// The byte budget was enforced by whichever encoder produced the bytes, so
    /// a queued frame's size is not re-checked here; the caller owns it.
    pub fn write_encoded(&self, bytes: &[u8]) -> Result<usize, WriteError> {
        let mut state = self.inner.lock().expect("frame writer mutex poisoned");
        if let Some(kind) = state.terminal_error {
            return Err(WriteError::Io(io::Error::new(kind, "connection failed")));
        }
        state.writer.write_all(bytes).map_err(|e| {
            state.terminal_error = Some(e.kind());
            WriteError::Io(e)
        })?;
        state.writer.flush().map_err(WriteError::Io)?;
        Ok(bytes.len())
    }

    /// Enqueues a best-effort frame without blocking on the physical write.
    ///
    /// In this synchronous port the write still happens inline, but the
    /// bounded slot pool preserves the drop-on-saturation contract: once
    /// `limit` frames are outstanding the caller gets `OutboundQueueFullError`
    /// and may drop the observation rather than stall the hot path.
    pub fn try_write_frame<T: Serialize>(&self, value: &T) -> Result<usize, WriteError> {
        {
            let mut used = self.best_effort_used.lock().expect("queue mutex poisoned");
            if *used >= self.best_effort_limit {
                return Err(WriteError::QueueFull(OutboundQueueFullError {
                    limit: self.best_effort_limit,
                }));
            }
            *used += 1;
        }
        let result = self.write_frame(value);
        {
            let mut used = self.best_effort_used.lock().expect("queue mutex poisoned");
            *used -= 1;
        }
        result
    }
}

/// Why writing a frame failed.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(transparent)]
    FrameTooLarge(#[from] FrameTooLargeError),
    #[error(transparent)]
    QueueFull(#[from] OutboundQueueFullError),
    #[error("rpcwire: write failed: {0}")]
    Io(#[source] io::Error),
}

/// One JSON-RPC 2.0 outbound envelope.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutboundEnvelope<'a> {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<&'a RawJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<&'a RawJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'a ErrorObject>,
}

impl<'a> OutboundEnvelope<'a> {
    /// Builds a request envelope.
    pub fn request(id: RequestId, method: &'a str, params: &'a RawJson) -> Self {
        Self {
            jsonrpc: "2.0",
            id: Some(id),
            method: Some(method),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    /// Builds a notification envelope.
    pub fn notification(method: &'a str, params: Option<&'a RawJson>) -> Self {
        Self {
            jsonrpc: "2.0",
            id: None,
            method: Some(method),
            params,
            result: None,
            error: None,
        }
    }

    /// Builds a result response envelope.
    pub fn result(id: RequestId, result: &'a RawJson) -> Self {
        Self {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    /// Builds an error response envelope.
    pub fn error(id: RequestId, error: &'a ErrorObject) -> Self {
        Self {
            jsonrpc: "2.0",
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }
}

/// Decodes strict DTO params from a frame, rejecting unknown members.
///
/// This mirrors the protocol layer's strict DTO decoder: the frame tolerates
/// unknown members but the DTO does not, so typos surface immediately.
pub fn decode_params<T: DeserializeOwned>(frame: &Frame) -> Result<T, serde_json::Error> {
    let raw = match frame {
        Frame::Request { params, .. } | Frame::Notification { params, .. } => params,
        _ => return serde_json::from_str("null"),
    };
    match raw {
        Some(raw) => serde_json::from_str(raw.get()),
        None => serde_json::from_str("{}"),
    }
}

/// Builds a JSON-RPC error object from a code, message, and optional data.
pub fn error_object(code: i32, message: impl Into<String>, data: Option<&str>) -> ErrorObject {
    ErrorObject {
        code,
        message: message.into(),
        data: data.map(|d| RawJson::parse(d).expect("data is valid JSON")),
    }
}

/// Converts a decoded error response into a typed `ResponseError`.
pub fn response_error(frame: Frame) -> Option<ResponseError> {
    match frame {
        Frame::Response { error: Some(e), .. } => Some(ResponseError {
            code: e.code,
            message: e.message,
            data: e.data,
        }),
        _ => None,
    }
}

/// Returns the standard invalid-request error code, exposed for adapters.
pub fn invalid_request_code() -> i32 {
    rpc_code::INVALID_REQUEST
}

/// Reads one strict request frame while retaining the reader's remainder.
///
/// Bootstrap-proxy support: keeps the exact raw bytes (including line ending
/// and harmless whitespace) so a proxy can forward the frame unchanged.
pub fn read_strict_request_frame(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<StrictRequestFrame>, StrictFrameError> {
    let Some((payload, raw)) = read_frame(reader, max_bytes)? else {
        return Ok(None);
    };
    let decoded: Inbound = serde_json::from_str(&payload).map_err(|_| StrictFrameError::Parse)?;
    validate_strict(&decoded).map_err(|_| StrictFrameError::Invalid)?;
    if decoded.method.as_deref().unwrap_or("").is_empty() || decoded.id.is_none() {
        return Err(StrictFrameError::NotARequest);
    }
    Ok(Some(StrictRequestFrame {
        raw,
        id: decoded.id.expect("checked").to_string(),
        method: decoded.method.expect("checked"),
        params: decoded.params.map(|p| p.to_string()),
    }))
}

/// A validated bootstrap request with its exact wire bytes retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictRequestFrame {
    /// Exact consumed bytes, including line ending and whitespace.
    pub raw: String,
    pub id: String,
    pub method: String,
    pub params: Option<String>,
}

/// Why reading a strict bootstrap frame failed.
#[derive(Debug, thiserror::Error)]
pub enum StrictFrameError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("rpcwire: parse error")]
    Parse,
    #[error("rpcwire: invalid request")]
    Invalid,
    #[error("rpcwire: bootstrap frame must be a request")]
    NotARequest,
}

#[cfg(test)]
mod tests;
