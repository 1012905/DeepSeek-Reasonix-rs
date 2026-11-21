//! Typed transport seam for the deferred stdio/HTTP wave.
//!
//! The wire and lifecycle state are testable without a subprocess: a transport
//! is anything that can answer a call, send a notification, and report its
//! declared process/transport kind. No I/O implementation lives in this slice.

use alloc::string::String;

use serde_json::Value;

use super::version::Era;
use super::wire::RpcError;

/// Typed transport failures. A caller distinguishes an unanswered probe from
/// a typed RPC error without matching message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    NoReply,
    Cancelled,
    Rpc(RpcError),
    Io(String),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoReply => f.write_str("MCP server did not answer"),
            Self::Cancelled => f.write_str("transport call cancelled"),
            Self::Rpc(e) => core::fmt::Display::fmt(e, f),
            Self::Io(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for TransportError {}

/// Seam implemented by a concrete transport in a later wave.
pub trait Transport {
    /// Sends a request and returns its result value.
    fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError>;
    /// Sends a fire-and-forget notification.
    fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError>;
    /// The negotiated era on this connection, if any.
    fn era(&self) -> &Era;
}

/// A no-I/O transport used by wire and lifecycle tests. It records calls and
/// answers from a supplied table, so no subprocess is involved.
#[derive(Clone, Debug, Default)]
pub struct RecordingTransport {
    calls: alloc::vec::Vec<(String, Option<Value>)>,
    notifications: alloc::vec::Vec<(String, Option<Value>)>,
    era: Era,
}

impl RecordingTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_era(era: Era) -> Self {
        Self {
            era,
            ..Self::default()
        }
    }

    pub fn record_call(&mut self, method: &str, params: Option<Value>) {
        self.calls.push((method.to_string(), params));
    }

    pub fn record_notification(&mut self, method: &str, params: Option<Value>) {
        self.notifications.push((method.to_string(), params));
    }

    pub fn calls(&self) -> &[(String, Option<Value>)] {
        &self.calls
    }

    pub fn notifications(&self) -> &[(String, Option<Value>)] {
        &self.notifications
    }
}

impl Transport for RecordingTransport {
    fn call(&self, method: &str, _params: Option<&Value>) -> Result<Value, TransportError> {
        self.calls
            .iter()
            .find(|(m, _)| m == method)
            .map(|(_, p)| p.clone().unwrap_or(Value::Null))
            .ok_or(TransportError::NoReply)
    }

    fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
        let _ = (method, params);
        Ok(())
    }

    fn era(&self) -> &Era {
        &self.era
    }
}
