//! Typed transport seam for the deferred stdio/HTTP wave.
//!
//! The wire and lifecycle state are testable without a subprocess: a transport
//! is anything that can answer a call, send a notification, and report its
//! declared process/transport kind. No I/O implementation lives in this slice.

use alloc::string::String;
use std::time::Duration;

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
    /// The connection ended before the request was written: that request never
    /// reached the server, so a replacement connection can carry it without
    /// repeating a side effect. A death mid-request is deliberately unmarked —
    /// the server may already have run the call.
    Gone,
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoReply => f.write_str("MCP server did not answer"),
            Self::Cancelled => f.write_str("transport call cancelled"),
            Self::Rpc(e) => core::fmt::Display::fmt(e, f),
            Self::Io(msg) => f.write_str(msg),
            Self::Gone => f.write_str("mcp connection ended"),
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

/// A transport the reconnecting wrapper owns, so it can shut the connection
/// down when a replacement takes over or the handle closes.
pub trait OwnedTransport: Transport {
    /// Shuts the connection down; idempotent for the wrapper's callers.
    fn close(&self);
}

/// Produces one fresh replacement connection. The budget bounds this dial;
/// the replacement is handed over ready for the wrapper to rebind onto.
pub trait Dialer {
    /// Opens one replacement connection.
    fn dial(
        &self,
        budget: Duration,
    ) -> Result<Box<dyn OwnedTransport + Send + Sync>, TransportError>;
}

impl<F> Dialer for F
where
    F: Fn() -> Result<Box<dyn OwnedTransport + Send + Sync>, TransportError> + Send + Sync,
{
    fn dial(
        &self,
        _budget: Duration,
    ) -> Result<Box<dyn OwnedTransport + Send + Sync>, TransportError> {
        self()
    }
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
