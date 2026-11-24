//! Streamable HTTP transport (MCP-010): every JSON-RPC message is one HTTP
//! POST to the server endpoint.
//!
//! Oracle: `internal/ext/plugin/transport_http.go`. The server answers either
//! `application/json` (one response) or `text/event-stream` (the response plus
//! interleaved notifications and server requests). The `Mcp-Session-Id` the
//! server assigns is captured and echoed on every later request; a 404 to a
//! request that carried a session means that session is gone.
//!
//! This slice owns transport framing only: the initialize handshake, OAuth
//! refresh, and progress-sink registration live with the lifecycle layer.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::http::{EventStream, HttpClient, HttpClientConfig, HttpError, HttpReply};
use super::http_headers::modern_request_headers;
use super::seam::{Transport, TransportError};
use super::version::Era;
use super::wire::{
    InboundMessage, RpcRequest, decode_inbound_message, decode_rpc_response, is_notification_id,
    server_request_reply,
};
use crate::redact::redact_credentials;

/// Bounded reply deadline for a server-originated request answered mid-stream.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Generation when no explicit call timeout was configured.
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_millis(300_000);

/// Options for one Streamable HTTP MCP server.
#[derive(Clone, Debug)]
pub struct StreamableHttpConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// Era fixed at connect time; `Transport::era` reports this same value.
    pub era: Era,
    pub call_timeout: Option<Duration>,
}

/// One live Streamable HTTP connection.
pub struct StreamableHttpTransport {
    client: HttpClient,
    session: Arc<Mutex<Option<String>>>,
    protocol_version: Mutex<String>,
    era: Era,
    next_id: AtomicI64,
    default_timeout: Duration,
    static_headers: BTreeMap<String, String>,
}

impl StreamableHttpTransport {
    /// Builds the client and opens the endpoint. No initialize handshake runs
    /// here: era and protocol version are supplied by the lifecycle caller.
    pub fn connect(config: StreamableHttpConfig) -> Result<Self, TransportError> {
        let default_timeout = config.call_timeout.unwrap_or(DEFAULT_CALL_TIMEOUT);
        let client = HttpClient::new(HttpClientConfig {
            url: config.url,
            headers: config.headers.clone(),
            call_timeout: default_timeout,
        })
        .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        Ok(Self {
            client,
            session: Arc::new(Mutex::new(None)),
            protocol_version: Mutex::new(String::new()),
            era: config.era,
            next_id: AtomicI64::new(0),
            default_timeout,
            static_headers: config.headers,
        })
    }

    /// The session id the server last issued, if any.
    pub fn session_id(&self) -> Option<String> {
        self.session.lock().expect("session mutex").clone()
    }

    /// Records the negotiated protocol revision, echoed as
    /// `MCP-Protocol-Version` until cleared.
    pub fn set_protocol_version(&self, version: &str) {
        *self.protocol_version.lock().expect("protocol mutex") = version.to_string();
    }

    /// Ends the server session with a best-effort DELETE, then stops the client.
    pub fn close(&self) {
        if let Some(session) = self.session_id() {
            let mut headers = self.static_header_pairs();
            headers.push(("Mcp-Session-Id".to_string(), session));
            let _ = self
                .client
                .delete(&Self::header_refs(&headers), REPLY_TIMEOUT);
        }
        self.client.shutdown();
    }

    fn static_header_pairs(&self) -> Vec<(String, String)> {
        self.static_headers
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
        headers
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }

    fn outgoing_headers(&self, method: &str, params: Option<&Value>) -> Vec<(String, String)> {
        let mut headers = self.static_header_pairs();
        if let Some(session) = self.session_id() {
            headers.push(("Mcp-Session-Id".to_string(), session));
        }
        let version = self
            .protocol_version
            .lock()
            .expect("protocol mutex")
            .clone();
        if !version.is_empty() {
            headers.push(("MCP-Protocol-Version".to_string(), version));
        }
        headers.extend(modern_request_headers(method, params));
        headers
    }

    fn sync_session(&self) {
        *self.session.lock().expect("session mutex") = self.client.session_id();
    }

    fn map_http(&self, err: HttpError, held_session: bool) -> TransportError {
        match &err {
            HttpError::Status {
                status: _,
                rpc: Some(rpc),
                ..
            } => TransportError::Rpc(rpc.clone()),
            HttpError::Status { status, .. } if held_session && *status == 404 => {
                self.client.clear_session();
                *self.session.lock().expect("session mutex") = None;
                TransportError::Io("MCP session expired".into())
            }
            HttpError::SessionExpired { .. } => {
                self.client.clear_session();
                *self.session.lock().expect("session mutex") = None;
                TransportError::Io("MCP session expired".into())
            }
            HttpError::Timeout => TransportError::NoReply,
            other => TransportError::Io(redact_credentials(&other.to_string())),
        }
    }

    fn decode_result(bytes: &[u8]) -> Result<Value, TransportError> {
        let response = decode_rpc_response(bytes)
            .ok_or_else(|| TransportError::Io("MCP response is not a JSON-RPC reply".into()))?;
        match response.error {
            Some(rpc) => Err(TransportError::Rpc(rpc)),
            None => Ok(response.result),
        }
    }

    /// Reads an SSE stream for the response matching `id`, dispatching
    /// notifications and answering server requests along the way.
    fn read_sse(&self, mut stream: EventStream, id: i64) -> Result<Value, TransportError> {
        while let Some(event) = stream.next_event() {
            let event =
                event.map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
            if !event.event.is_empty() && event.event != "message" {
                continue;
            }
            let Some(message) = decode_inbound_message(event.data.as_bytes()) else {
                continue;
            };
            if !message.method.is_empty() {
                let message_id = message.id.clone().unwrap_or(Value::Null);
                if is_notification_id(&message_id) {
                    // Registered progress/notification handlers attach in a
                    // later slice; here the frame is skipped, not routed.
                    continue;
                }
                self.reply_server_request(&message)?;
                continue;
            }
            let Some(response) = decode_rpc_response(event.data.as_bytes()) else {
                continue;
            };
            if response.id != id {
                continue;
            }
            return match response.error {
                Some(rpc) => Err(TransportError::Rpc(rpc)),
                None => Ok(response.result),
            };
        }
        Err(TransportError::Io(format!(
            "SSE stream ended without a response to id {id}"
        )))
    }

    /// Answers one server-originated request on a separate POST while the
    /// original response stream stays open.
    fn reply_server_request(&self, message: &InboundMessage) -> Result<(), TransportError> {
        let reply = server_request_reply(
            message.id.clone().unwrap_or(Value::Null),
            &message.method,
            Vec::new(),
        );
        let body = reply.to_json().into_bytes();
        let headers = self.outgoing_headers(&message.method, message.params.as_ref());
        self.client
            .post(&body, &Self::header_refs(&headers), REPLY_TIMEOUT)
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        self.sync_session();
        Ok(())
    }
}

impl Transport for StreamableHttpTransport {
    fn call(&self, method: &str, params: Option<&Value>) -> Result<Value, TransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let request = RpcRequest::new(id, method, params.cloned());
        let body = serde_json::to_vec(&request)
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        let headers = self.outgoing_headers(method, params);
        let held_session = self.session_id().is_some();
        let reply = self
            .client
            .post(&body, &Self::header_refs(&headers), self.default_timeout)
            .map_err(|err| self.map_http(err, held_session))?;
        self.sync_session();
        match reply {
            HttpReply::Json(bytes) => Self::decode_result(&bytes),
            HttpReply::Events(stream) => self.read_sse(stream, id),
        }
    }

    fn notify(&self, method: &str, params: Option<&Value>) -> Result<(), TransportError> {
        let request = RpcRequest::notification(method, params.cloned());
        let body = serde_json::to_vec(&request)
            .map_err(|err| TransportError::Io(redact_credentials(&err.to_string())))?;
        let headers = self.outgoing_headers(method, params);
        let held_session = self.session_id().is_some();
        self.client
            .post(&body, &Self::header_refs(&headers), self.default_timeout)
            .map_err(|err| self.map_http(err, held_session))?;
        self.sync_session();
        Ok(())
    }

    fn era(&self) -> &Era {
        &self.era
    }
}

impl Drop for StreamableHttpTransport {
    fn drop(&mut self) {
        self.close();
    }
}
