//! Protocol versions, eras, and modern-era negotiation.
//!
//! Oracle: `internal/ext/plugin/protocol_version.go`, `protocol_modern.go`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde_json::{Map, Value};

use super::naming::mcp_roots_from_absolute;
use super::wire::RpcError;

/// Revision advertised by the legacy initialize handshake.
pub const PROTOCOL_VERSION: &str = "2025-11-25";
/// Server answer older than the field runs on the oldest revision.
pub const OLDEST_PROTOCOL_VERSION: &str = "2024-11-05";
/// Revisions this client speaks, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] = [
    PROTOCOL_VERSION,
    "2025-06-18",
    "2025-03-26",
    OLDEST_PROTOCOL_VERSION,
];

/// Revision whose requests carry their era in `_meta`; no initialize.
pub const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
/// Revisions spoken without a handshake.
pub const MODERN_VERSIONS: [&str; 1] = [MODERN_PROTOCOL_VERSION];
/// Method a client uses to ask which era a server speaks.
pub const DISCOVER_METHOD: &str = "server/discover";

pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const TASKS_EXTENSION: &str = "io.modelcontextprotocol/tasks";

/// JSON-RPC implementation-defined era codes.
pub const CODE_HEADER_MISMATCH: i64 = -32020;
pub const CODE_MISSING_CLIENT_CAPABILITY: i64 = -32021;
pub const CODE_UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
/// All era codes, for transport-side classification.
pub const ERA_CODES: [i64; 3] = [
    CODE_HEADER_MISMATCH,
    CODE_MISSING_CLIENT_CAPABILITY,
    CODE_UNSUPPORTED_PROTOCOL_VERSION,
];

pub const MAX_INPUT_ROUNDS: usize = 4;
pub const MAX_INPUT_REQUESTS: usize = 16;
pub const MAX_REQUEST_STATE_SZ: usize = 1 << 20;
/// Bound on a discovery probe against a legacy server (milliseconds).
pub const MODERN_PROBE_WAIT_MS: u64 = 3_000;

/// Typed failures from version negotiation and modern input rounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionError {
    UnsupportedProtocolVersion { detail: Option<String> },
    ProbeUnanswered,
    InputRequired { method: String },
    InputOverBounds,
    InputRoundsExhausted,
    Cancelled,
    Other(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProtocolVersion { detail } => match detail {
                Some(d) => write!(
                    f,
                    "MCP server chose a protocol version this client does not support ({d})"
                ),
                None => {
                    f.write_str("MCP server chose a protocol version this client does not support")
                }
            },
            Self::ProbeUnanswered => f.write_str("MCP server did not answer server/discover"),
            Self::InputRequired { method } => {
                write!(
                    f,
                    "MCP server needs input this client does not provide: {method}"
                )
            }
            Self::InputOverBounds => f.write_str("MCP input round over this client's bounds"),
            Self::InputRoundsExhausted => {
                f.write_str("MCP server still needed input after 4 rounds")
            }
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for VersionError {}

/// Which era a connection runs in. Empty version means legacy initialize.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Era(pub String);

impl Era {
    pub fn legacy() -> Self {
        Self(String::new())
    }

    pub fn modern(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    pub fn is_modern(&self) -> bool {
        !self.0.is_empty()
    }

    pub fn is_legacy(&self) -> bool {
        self.0.is_empty()
    }
}

/// Capabilities the server reported in a modern discovery result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoverResult {
    pub supported_versions: Vec<String>,
    pub capabilities: Map<String, Value>,
    pub instructions: String,
}

/// Typed outcome of a discovery probe, independent of a concrete transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EraPlan {
    Modern {
        era: Era,
        discover: Option<DiscoverResult>,
    },
    Legacy,
    Unanswered,
    Failed(VersionError),
}

/// Input a probe drew: a JSON-RPC result, an RPC error, or no reply at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeReply {
    Result(Value),
    RpcError(RpcError),
    NoReply,
    Cancelled,
}

/// Reads the server's initialize answer: empty runs on the oldest revision.
pub fn negotiated_version(reply: &str) -> Result<String, VersionError> {
    if reply.is_empty() {
        return Ok(OLDEST_PROTOCOL_VERSION.to_string());
    }
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&reply) {
        return Ok(reply.to_string());
    }
    Err(VersionError::UnsupportedProtocolVersion {
        detail: Some(reply.to_string()),
    })
}

fn elicit_capability() -> Value {
    serde_json::json!({"form": {}, "url": {}})
}

/// Modern client capabilities. `roots` is present only with a workspace root.
pub fn client_capabilities(workspace_root: &str) -> Value {
    let has_roots = !mcp_roots_from_absolute(workspace_root).is_empty();
    let mut caps = Map::new();
    caps.insert("elicitation".into(), elicit_capability());
    let mut extensions = Map::new();
    extensions.insert(TASKS_EXTENSION.into(), Value::Object(Map::new()));
    caps.insert("extensions".into(), Value::Object(extensions));
    if has_roots {
        caps.insert("roots".into(), Value::Object(Map::new()));
    }
    Value::Object(caps)
}

fn client_info() -> Value {
    serde_json::json!({"name": "reasonix", "version": "dev"})
}

/// `_meta` for a modern request. Object keys sort, matching Go's `encoding/json`.
pub fn client_meta(version: &str, workspace_root: &str) -> Value {
    let mut meta = Map::new();
    meta.insert(
        META_PROTOCOL_VERSION.into(),
        Value::String(version.to_string()),
    );
    meta.insert(META_CLIENT_INFO.into(), client_info());
    meta.insert(
        META_CLIENT_CAPABILITIES.into(),
        client_capabilities(workspace_root),
    );
    Value::Object(meta)
}

/// Merges `meta` into `params._meta` without mutating the caller's params.
pub fn with_meta(params: Option<&Value>, meta: &Value) -> Result<Value, VersionError> {
    let mut out = match params {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(other) => match other {
            Value::Object(m) => m.clone(),
            _ => {
                return Err(VersionError::Other("params is not a JSON object".into()));
            }
        },
    };
    let merged_meta = out
        .get("_meta")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut merged = merged_meta;
    let additions = meta
        .as_object()
        .ok_or_else(|| VersionError::Other("meta is not a JSON object".into()))?;
    for (k, v) in additions {
        merged.insert(k.clone(), v.clone());
    }
    out.insert("_meta".into(), Value::Object(merged));
    Ok(Value::Object(out))
}

/// Parses a discovery result. Unparseable or empty answers are not modern.
pub fn discover_result(value: &Value) -> Option<DiscoverResult> {
    let obj = value.as_object()?;
    let versions = obj.get("supportedVersions")?.as_array()?;
    let supported: Vec<String> = versions
        .iter()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect();
    if supported.is_empty() {
        return None;
    }
    let capabilities = obj
        .get("capabilities")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let instructions = obj
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(DiscoverResult {
        supported_versions: supported,
        capabilities,
        instructions,
    })
}

/// Chooses a modern revision shared by both sides, else a legacy one, else fails.
pub fn pick_era(
    supported: &[String],
    discover: Option<DiscoverResult>,
) -> Result<EraPlan, VersionError> {
    for v in MODERN_VERSIONS {
        if supported.iter().any(|s| s == v) {
            return Ok(EraPlan::Modern {
                era: Era::modern(v),
                discover,
            });
        }
    }
    for v in SUPPORTED_PROTOCOL_VERSIONS {
        if supported.iter().any(|s| s == v) {
            return Ok(EraPlan::Legacy);
        }
    }
    Err(VersionError::UnsupportedProtocolVersion {
        detail: Some(supported.join(", ")),
    })
}

/// Classifies a probe reply into an era plan.
pub fn classify_probe_result(reply: ProbeReply) -> EraPlan {
    match reply {
        ProbeReply::Cancelled => EraPlan::Failed(VersionError::Cancelled),
        ProbeReply::NoReply => EraPlan::Unanswered,
        ProbeReply::Result(value) => match discover_result(&value) {
            Some(d) => {
                let supported = d.supported_versions.clone();
                pick_era(&supported, Some(d)).unwrap_or_else(EraPlan::Failed)
            }
            None => EraPlan::Legacy,
        },
        ProbeReply::RpcError(rpc) => {
            if rpc.code != CODE_UNSUPPORTED_PROTOCOL_VERSION {
                return EraPlan::Legacy;
            }
            let supported = rpc
                .data
                .as_ref()
                .and_then(|d| d.get("supported"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if supported.is_empty() {
                return EraPlan::Legacy;
            }
            pick_era(&supported, None).unwrap_or_else(EraPlan::Failed)
        }
    }
}

/// Maps a plan onto the era a connection uses. SSE never speaks modern.
pub fn connect_era(
    transport: super::lifecycle::TransportKind,
    plan: EraPlan,
) -> Result<Era, VersionError> {
    if transport == super::lifecycle::TransportKind::Sse {
        return Ok(Era::legacy());
    }
    match plan {
        EraPlan::Modern { era, .. } => Ok(era),
        EraPlan::Legacy | EraPlan::Unanswered => Ok(Era::legacy()),
        EraPlan::Failed(e) => Err(e),
    }
}

/// Params for the legacy initialize request.
pub fn initialize_params(workspace_root: &str) -> Value {
    let mut capabilities = Map::new();
    if !mcp_roots_from_absolute(workspace_root).is_empty() {
        let mut roots = Map::new();
        roots.insert("listChanged".into(), Value::Bool(false));
        capabilities.insert("roots".into(), Value::Object(roots));
    }
    capabilities.insert("elicitation".into(), elicit_capability());
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": Value::Object(capabilities),
        "clientInfo": client_info(),
    })
}

/// Bounds one modern input round. `round` is zero-based.
pub fn check_input_bounds(
    round: usize,
    input_requests: usize,
    request_state_size: usize,
) -> Result<(), VersionError> {
    if round + 1 >= MAX_INPUT_ROUNDS {
        return Err(VersionError::InputRoundsExhausted);
    }
    if input_requests > MAX_INPUT_REQUESTS || request_state_size > MAX_REQUEST_STATE_SZ {
        return Err(VersionError::InputOverBounds);
    }
    Ok(())
}
