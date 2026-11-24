//! JSON-RPC 2.0 envelope and dispatch classification (server.go:80-105,
//! 205-230).
//!
//! The envelope is direction-agnostic: the same frame shape carries inbound
//! requests/notifications/responses and outbound ones. Classification keys on
//! the presence of `method` and `id`, matching the reference's dispatch table.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{RpcError, RpcErrorBody};
use crate::frame::encode_frame;
use crate::methods::CANCEL_REQUEST;

/// Protocol version marker written on every outbound frame.
pub const JSONRPC_VERSION: &str = "2.0";

/// Monotonic outbound request id allocator. The reference starts its counter at
/// zero and increments before issuing, so the first outbound request carries
/// id 1 (server.go:309).
#[derive(Debug, Default)]
pub struct OutboundIdAllocator {
    next: i64,
}

impl OutboundIdAllocator {
    /// Returns the next wire id as a JSON number.
    pub fn next_id(&mut self) -> Value {
        self.next += 1;
        Value::from(self.next)
    }

    /// Number of ids handed out so far, for diagnostics and tests.
    pub fn issued(&self) -> i64 {
        self.next
    }
}

/// One decoded JSON-RPC frame. `id` keeps an explicit `null` (which the
/// reference treats as present) distinguishable from a missing field.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Envelope {
    #[serde(rename = "jsonrpc", default, skip_serializing_if = "String::is_empty")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorBody>,
}

impl Envelope {
    /// Parses one frame line. A parse failure is the caller's `-32700`
    /// `parse error` path; this function does not build a response frame.
    pub fn from_slice(line: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(line)
    }

    /// True when the frame carries an id field, explicit `null` included
    /// (server.go:211).
    pub fn has_id(&self) -> bool {
        self.id.is_some()
    }
}

/// Where a decoded frame belongs in the dispatch table.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchFrame {
    /// `method` + `id`: a request routed to its registered handler.
    Request {
        id: Value,
        method: String,
        params: Option<Value>,
    },
    /// `$/cancel_request` without `id`: routed to the in-flight set, never the
    /// notification table.
    CancelRequest { params: Option<Value> },
    /// `method` without `id`: a notification, dropped when unregistered.
    Notification {
        method: String,
        params: Option<Value>,
    },
    /// `id` without `method`: a response to an earlier outbound request.
    Response {
        id: Value,
        result: Option<Value>,
        error: Option<RpcErrorBody>,
    },
    /// Neither request, notification, nor response: `-32600`.
    InvalidRequest,
}

/// Classifies a decoded envelope per server.go:212-229.
pub fn classify(envelope: &Envelope) -> DispatchFrame {
    let has_id = envelope.has_id();
    if !envelope.method.is_empty() && has_id {
        DispatchFrame::Request {
            id: envelope.id.clone().expect("id present after check"),
            method: envelope.method.clone(),
            params: envelope.params.clone(),
        }
    } else if envelope.method == CANCEL_REQUEST && !has_id {
        DispatchFrame::CancelRequest {
            params: envelope.params.clone(),
        }
    } else if !envelope.method.is_empty() && !has_id {
        DispatchFrame::Notification {
            method: envelope.method.clone(),
            params: envelope.params.clone(),
        }
    } else if envelope.method.is_empty() && has_id {
        DispatchFrame::Response {
            id: envelope.id.clone().expect("id present after check"),
            result: envelope.result.clone(),
            error: envelope.error.clone(),
        }
    } else {
        DispatchFrame::InvalidRequest
    }
}

// ---- Outbound frame builders ----------------------------------------------

/// Serializes an outbound request frame: `{jsonrpc, id, method, params?}`.
pub fn request_frame(id: &Value, method: &str, params: Option<Value>) -> Vec<u8> {
    let env = Envelope {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: Some(id.clone()),
        method: method.to_owned(),
        params,
        result: None,
        error: None,
    };
    encode_frame(&env)
}

/// Serializes an outbound notification: `{jsonrpc, method, params?}`.
pub fn notification_frame(method: &str, params: Option<Value>) -> Vec<u8> {
    let env = Envelope {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: None,
        method: method.to_owned(),
        params,
        result: None,
        error: None,
    };
    encode_frame(&env)
}

/// Serializes a success response: `{jsonrpc, id, result}`.
pub fn success_frame(id: &Value, result: &Value) -> Vec<u8> {
    let env = Envelope {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: Some(id.clone()),
        method: String::new(),
        params: None,
        result: Some(result.clone()),
        error: None,
    };
    encode_frame(&env)
}

/// Serializes an error response: `{jsonrpc, id, error:{code,message,data?}}`.
pub fn error_frame(id: Option<&Value>, err: &RpcError) -> Vec<u8> {
    let env = Envelope {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id: id.cloned(),
        method: String::new(),
        params: None,
        result: None,
        error: Some(RpcErrorBody::from(err)),
    };
    encode_frame(&env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn classifies_request_notification_response_cancel_invalid() {
        let request = Envelope {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(1)),
            method: "session/new".into(),
            params: Some(Value::Null),
            result: None,
            error: None,
        };
        assert_eq!(
            classify(&request),
            DispatchFrame::Request {
                id: serde_json::json!(1),
                method: "session/new".into(),
                params: Some(Value::Null),
            }
        );

        let notification = Envelope {
            jsonrpc: "2.0".into(),
            id: None,
            method: "session/cancel".into(),
            params: None,
            result: None,
            error: None,
        };
        assert_eq!(
            classify(&notification),
            DispatchFrame::Notification {
                method: "session/cancel".into(),
                params: None,
            }
        );

        let cancel = Envelope {
            jsonrpc: "2.0".into(),
            id: None,
            method: CANCEL_REQUEST.into(),
            params: Some(serde_json::json!({"requestId": 7})),
            result: None,
            error: None,
        };
        assert!(matches!(
            classify(&cancel),
            DispatchFrame::CancelRequest { .. }
        ));

        let response = Envelope {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(7)),
            method: String::new(),
            params: None,
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        assert!(matches!(
            classify(&response),
            DispatchFrame::Response { .. }
        ));

        let neither = Envelope {
            jsonrpc: "2.0".into(),
            id: None,
            method: String::new(),
            params: None,
            result: None,
            error: None,
        };
        assert_eq!(classify(&neither), DispatchFrame::InvalidRequest);
    }

    #[test]
    fn outbound_ids_start_at_one_and_increase_monotonically() {
        let mut ids = OutboundIdAllocator::default();
        assert_eq!(ids.next_id(), serde_json::json!(1));
        assert_eq!(ids.next_id(), serde_json::json!(2));
        assert_eq!(ids.issued(), 2);
    }

    #[test]
    fn explicit_null_id_counts_as_present() {
        let env = Envelope {
            jsonrpc: "2.0".into(),
            id: Some(Value::Null),
            method: "initialize".into(),
            params: None,
            result: None,
            error: None,
        };
        assert!(env.has_id());
        assert!(matches!(classify(&env), DispatchFrame::Request { .. }));
    }

    #[test]
    fn outbound_frames_use_declared_field_order() {
        let id = serde_json::json!(1);
        let req = request_frame(&id, "session/new", Some(serde_json::json!({"cwd":"/tmp"})));
        let text = String::from_utf8(req).unwrap();
        assert!(text.starts_with(r#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":"#));
        assert!(text.ends_with("}\n"));

        let ok = success_frame(&id, &serde_json::json!({"sessionId":"x"}));
        let text = String::from_utf8(ok).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"x"}}"#.to_owned() + "\n"
        );

        let err = RpcError::new(RpcErrorCode::MethodNotFound, "method not found: bog");
        let frame = error_frame(Some(&id), &err);
        let text = String::from_utf8(frame).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found: bog"}}"#
                .to_owned()
                + "\n"
        );
    }
}
