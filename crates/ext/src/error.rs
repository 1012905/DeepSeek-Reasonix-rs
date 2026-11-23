//! Typed transport and domain errors for the extension wire.
//!
//! Errors carry identity (a typed variant or code), never a sentence a
//! caller has to match against. The frozen reason table mirrors
//! `protocol/errors.go` byte-for-byte.

use std::fmt;

/// Standard JSON-RPC 2.0 error codes plus the transport-local busy code.
pub mod rpc_code {
    pub const PARSE: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;
    pub const SERVER_BUSY: i32 = -32099;
    /// Every extension domain error uses this code on the wire; the reason
    /// in the structured data distinguishes them.
    pub const DOMAIN: i32 = -32000;
}

/// A frame that violated the configured NDJSON byte budget.
///
/// `size` and `limit` include the trailing newline, matching wire bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTooLargeError {
    pub direction: Direction,
    pub size: usize,
    pub limit: usize,
}

/// Which direction a framing error occurred in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

impl fmt::Display for FrameTooLargeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rpcwire: {} frame is {} bytes; limit is {}",
            match self.direction {
                Direction::Inbound => "inbound",
                Direction::Outbound => "outbound",
            },
            self.size,
            self.limit
        )
    }
}

impl std::error::Error for FrameTooLargeError {}

/// A best-effort notification could not be enqueued without blocking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboundQueueFullError {
    pub limit: usize,
}

impl fmt::Display for OutboundQueueFullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rpcwire: outbound notification queue is full (limit {})",
            self.limit
        )
    }
}

impl std::error::Error for OutboundQueueFullError {}

/// An outbound write that made no progress within its bound.
///
/// A peer that holds the pipe open but stopped reading would otherwise block
/// every later caller; this is transport-fatal, so the connection ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteStallError {
    pub direction: Direction,
    /// The bound that elapsed, in milliseconds.
    pub stall_ms: u64,
}

impl fmt::Display for WriteStallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rpcwire: {} write made no progress for {}ms",
            match self.direction {
                Direction::Inbound => "inbound",
                Direction::Outbound => "outbound",
            },
            self.stall_ms
        )
    }
}

impl std::error::Error for WriteStallError {}

/// Why a connection call failed.
///
/// The variants stay apart so a caller can tell a caller-side deadline from a
/// transport that died, and a peer error object from a frame budget breach.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnError {
    /// A frame violated `max_inbound_bytes` or `max_outbound_bytes`.
    FrameTooLarge(FrameTooLargeError),
    /// A best-effort write found the bounded queue full.
    QueueFull(OutboundQueueFullError),
    /// An outbound write stalled past `max_write_stall`.
    WriteStall(WriteStallError),
    /// The peer answered with a JSON-RPC error object.
    Response(ResponseError),
    /// The transport failed; the payload is the operating-system message.
    Io(String),
    /// The caller's own deadline elapsed; the connection stays usable.
    TimedOut,
    /// The connection already ended; every later call observes this.
    Closed { name: String },
}

impl fmt::Display for ConnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge(e) => fmt::Display::fmt(e, f),
            Self::QueueFull(e) => fmt::Display::fmt(e, f),
            Self::WriteStall(e) => fmt::Display::fmt(e, f),
            Self::Response(e) => fmt::Display::fmt(e, f),
            Self::Io(message) => write!(f, "rpcwire: {message}"),
            Self::TimedOut => f.write_str("rpcwire: call deadline elapsed"),
            Self::Closed { name } => write!(f, "{name}: connection closed"),
        }
    }
}

impl std::error::Error for ConnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FrameTooLarge(e) => Some(e),
            Self::QueueFull(e) => Some(e),
            Self::WriteStall(e) => Some(e),
            Self::Response(e) => Some(e),
            Self::Io(_) | Self::TimedOut | Self::Closed { .. } => None,
        }
    }
}

impl From<FrameTooLargeError> for ConnError {
    fn from(err: FrameTooLargeError) -> Self {
        Self::FrameTooLarge(err)
    }
}

impl From<OutboundQueueFullError> for ConnError {
    fn from(err: OutboundQueueFullError) -> Self {
        Self::QueueFull(err)
    }
}

impl From<WriteStallError> for ConnError {
    fn from(err: WriteStallError) -> Self {
        Self::WriteStall(err)
    }
}

impl From<ResponseError> for ConnError {
    fn from(err: ResponseError) -> Self {
        Self::Response(err)
    }
}

/// An error returned by a `Conn::request` when the peer sends a JSON-RPC error.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    /// Raw structured data, left for the protocol adapter to decode.
    pub data: Option<crate::rawjson::RawJson>,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ResponseError {}

/// The frozen reason identity of a domain error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorReason {
    ProtocolError,
    UnknownMethod,
    InvalidParams,
    FrameTooLarge,
    ContentRefExpired,
    UnsupportedVersion,
    CapabilityNotDeclared,
    ShutdownTimeout,
    StreamGap,
    StreamCancelled,
    ProviderFailed,
    ProviderInterrupted,
    InterceptTimeout,
    DependencyUnsatisfied,
    DependencyCycle,
    SchemaMismatch,
    ActivationFailed,
    StaleGeneration,
    CleanupFailed,
    Internal,
}

impl ErrorReason {
    /// Parses a frozen reason from its wire string.
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "protocol_error" => Self::ProtocolError,
            "unknown_method" => Self::UnknownMethod,
            "invalid_params" => Self::InvalidParams,
            "frame_too_large" => Self::FrameTooLarge,
            "content_ref_expired" => Self::ContentRefExpired,
            "unsupported_version" => Self::UnsupportedVersion,
            "capability_not_declared" => Self::CapabilityNotDeclared,
            "shutdown_timeout" => Self::ShutdownTimeout,
            "stream_gap" => Self::StreamGap,
            "stream_cancelled" => Self::StreamCancelled,
            "provider_failed" => Self::ProviderFailed,
            "provider_interrupted" => Self::ProviderInterrupted,
            "intercept_timeout" => Self::InterceptTimeout,
            "dependency_unsatisfied" => Self::DependencyUnsatisfied,
            "dependency_cycle" => Self::DependencyCycle,
            "schema_mismatch" => Self::SchemaMismatch,
            "activation_failed" => Self::ActivationFailed,
            "stale_generation" => Self::StaleGeneration,
            "cleanup_failed" => Self::CleanupFailed,
            "internal" => Self::Internal,
            _ => return None,
        })
    }

    /// Returns the frozen wire string for this reason.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::ProtocolError => "protocol_error",
            Self::UnknownMethod => "unknown_method",
            Self::InvalidParams => "invalid_params",
            Self::FrameTooLarge => "frame_too_large",
            Self::ContentRefExpired => "content_ref_expired",
            Self::UnsupportedVersion => "unsupported_version",
            Self::CapabilityNotDeclared => "capability_not_declared",
            Self::ShutdownTimeout => "shutdown_timeout",
            Self::StreamGap => "stream_gap",
            Self::StreamCancelled => "stream_cancelled",
            Self::ProviderFailed => "provider_failed",
            Self::ProviderInterrupted => "provider_interrupted",
            Self::InterceptTimeout => "intercept_timeout",
            Self::DependencyUnsatisfied => "dependency_unsatisfied",
            Self::DependencyCycle => "dependency_cycle",
            Self::SchemaMismatch => "schema_mismatch",
            Self::ActivationFailed => "activation_failed",
            Self::StaleGeneration => "stale_generation",
            Self::CleanupFailed => "cleanup_failed",
            Self::Internal => "internal",
        }
    }

    /// The frozen spec: JSON-RPC code, generic message, retryable flag.
    pub fn spec(self) -> ErrorSpec {
        let (code, message, retryable) = match self {
            Self::ProtocolError => (
                rpc_code::INVALID_REQUEST,
                "The extension protocol frame or envelope is invalid.",
                false,
            ),
            Self::UnknownMethod => (
                rpc_code::METHOD_NOT_FOUND,
                "The method is not registered in Extension Protocol v2.",
                false,
            ),
            Self::InvalidParams => (
                rpc_code::INVALID_PARAMS,
                "The params do not match the registered method schema.",
                false,
            ),
            Self::FrameTooLarge => (
                rpc_code::DOMAIN,
                "The frame exceeds the frozen protocol size limit.",
                false,
            ),
            Self::ContentRefExpired => (
                rpc_code::DOMAIN,
                "The referenced content has expired.",
                true,
            ),
            Self::UnsupportedVersion => (
                rpc_code::DOMAIN,
                "The peer's extension protocol version is not supported.",
                false,
            ),
            Self::CapabilityNotDeclared => (
                rpc_code::DOMAIN,
                "The extension used a capability its manifest did not declare.",
                false,
            ),
            Self::ShutdownTimeout => (
                rpc_code::DOMAIN,
                "The extension did not shut down within the requested timeout.",
                true,
            ),
            Self::StreamGap => (
                rpc_code::DOMAIN,
                "A provider stream chunk is missing from the ordered sequence.",
                true,
            ),
            Self::StreamCancelled => (
                rpc_code::DOMAIN,
                "The provider stream was cancelled.",
                false,
            ),
            Self::ProviderFailed => (
                rpc_code::DOMAIN,
                "The extension provider stream failed.",
                true,
            ),
            Self::ProviderInterrupted => (
                rpc_code::DOMAIN,
                "The extension provider stream was interrupted.",
                true,
            ),
            Self::InterceptTimeout => (
                rpc_code::DOMAIN,
                "The extension did not answer an intercept within its timeout.",
                true,
            ),
            Self::DependencyUnsatisfied => (
                rpc_code::DOMAIN,
                "A required dependency is missing or not version-compatible.",
                false,
            ),
            Self::DependencyCycle => (
                rpc_code::DOMAIN,
                "The dependency graph contains a required cycle.",
                false,
            ),
            Self::SchemaMismatch => (
                rpc_code::DOMAIN,
                "A capability schema hash does not match the expected pin.",
                false,
            ),
            Self::ActivationFailed => (
                rpc_code::DOMAIN,
                "Component activation failed; the new generation was not published.",
                false,
            ),
            Self::StaleGeneration => (
                rpc_code::DOMAIN,
                "The message belongs to a superseded runtime generation.",
                false,
            ),
            Self::CleanupFailed => (
                rpc_code::DOMAIN,
                "Component cleanup failed while disposing scoped effects.",
                true,
            ),
            Self::Internal => (
                rpc_code::INTERNAL,
                "An internal extension protocol error occurred.",
                true,
            ),
        };
        ErrorSpec {
            reason: self,
            code,
            message,
            retryable,
        }
    }

    /// All 20 frozen reasons, sorted by wire string.
    pub const ALL: &'static [ErrorReason] = &[
        Self::ActivationFailed,
        Self::CapabilityNotDeclared,
        Self::CleanupFailed,
        Self::ContentRefExpired,
        Self::DependencyCycle,
        Self::DependencyUnsatisfied,
        Self::FrameTooLarge,
        Self::InterceptTimeout,
        Self::Internal,
        Self::InvalidParams,
        Self::ProtocolError,
        Self::ProviderFailed,
        Self::ProviderInterrupted,
        Self::SchemaMismatch,
        Self::ShutdownTimeout,
        Self::StaleGeneration,
        Self::StreamCancelled,
        Self::StreamGap,
        Self::UnknownMethod,
        Self::UnsupportedVersion,
    ];
}

/// The immutable spec of one frozen error reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorSpec {
    pub reason: ErrorReason,
    pub code: i32,
    pub message: &'static str,
    pub retryable: bool,
}

/// Structured JSON-RPC error data for extension domain errors.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProtocolErrorData {
    pub reason: String,
    pub retryable: bool,
}

impl ProtocolErrorData {
    /// Builds the frozen structured data for a reason.
    pub fn for_reason(reason: ErrorReason) -> Self {
        Self {
            reason: reason.as_wire().to_owned(),
            retryable: reason.spec().retryable,
        }
    }

    /// Validates against the frozen table: known reason, matching retryable.
    pub fn validate(&self) -> Result<(), DataValidationError> {
        let reason = ErrorReason::from_wire(&self.reason)
            .ok_or_else(|| DataValidationError::UnknownReason(self.reason.clone()))?;
        if self.retryable != reason.spec().retryable {
            return Err(DataValidationError::RetryableMismatch(self.reason.clone()));
        }
        Ok(())
    }
}

/// Why structured error data failed validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DataValidationError {
    #[error("unknown extension error reason {0:?}")]
    UnknownReason(String),
    #[error("retryable must match the frozen error table for {0:?}")]
    RetryableMismatch(String),
}

/// A frozen protocol error with a stable reason and generic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolError {
    pub reason: ErrorReason,
}

impl ProtocolError {
    /// Builds the frozen error for a reason.
    pub fn new(reason: ErrorReason) -> Self {
        Self { reason }
    }

    /// The frozen generic message for this error.
    pub fn message(self) -> &'static str {
        self.reason.spec().message
    }

    /// Structured data carried in the JSON-RPC error object.
    pub fn data(self) -> ProtocolErrorData {
        ProtocolErrorData::for_reason(self.reason)
    }

    /// JSON-RPC code: standard for the four mapped reasons, domain otherwise.
    pub fn code(self) -> i32 {
        self.reason.spec().code
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ProtocolError {}

/// Validates a peer's handshake identity.
///
/// The ID must match exactly and the version must parse as an integer whose
/// major equals 2; mismatches return the frozen typed reason.
pub fn compare_protocol_version(peer_id: &str, peer_version: &str) -> Result<(), ProtocolError> {
    if peer_id != crate::enums::PROTOCOL_ID {
        return Err(ProtocolError::new(ErrorReason::UnsupportedVersion));
    }
    let major: i32 = peer_version
        .parse()
        .map_err(|_| ProtocolError::new(ErrorReason::ProtocolError))?;
    if major != crate::enums::PROTOCOL_MAJOR {
        return Err(ProtocolError::new(ErrorReason::UnsupportedVersion));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_table_has_twenty_frozen_reasons() {
        assert_eq!(ErrorReason::ALL.len(), 20);
        for pair in ErrorReason::ALL.windows(2) {
            assert!(pair[0].as_wire() < pair[1].as_wire());
        }
        for reason in ErrorReason::ALL {
            assert_eq!(ErrorReason::from_wire(reason.as_wire()), Some(*reason));
            let spec = reason.spec();
            assert!(!spec.message.is_empty());
            assert!(matches!(
                spec.code,
                rpc_code::DOMAIN
                    | rpc_code::INVALID_REQUEST
                    | rpc_code::METHOD_NOT_FOUND
                    | rpc_code::INVALID_PARAMS
                    | rpc_code::INTERNAL
            ));
        }
    }

    #[test]
    fn standard_code_mapping_matches_go() {
        let expected = [
            (ErrorReason::ProtocolError, rpc_code::INVALID_REQUEST),
            (ErrorReason::UnknownMethod, rpc_code::METHOD_NOT_FOUND),
            (ErrorReason::InvalidParams, rpc_code::INVALID_PARAMS),
            (ErrorReason::Internal, rpc_code::INTERNAL),
        ];
        for (reason, code) in expected {
            assert_eq!(ProtocolError::new(reason).code(), code);
        }
        assert_eq!(
            ProtocolError::new(ErrorReason::StreamGap).code(),
            rpc_code::DOMAIN
        );
    }

    #[test]
    fn protocol_error_data_round_trips_and_validates() {
        let data = ProtocolErrorData::for_reason(ErrorReason::ShutdownTimeout);
        assert_eq!(data.reason, "shutdown_timeout");
        assert!(data.retryable);
        assert!(data.validate().is_ok());
        let bad = ProtocolErrorData {
            reason: "shutdown_timeout".into(),
            retryable: false,
        };
        assert_eq!(
            bad.validate(),
            Err(DataValidationError::RetryableMismatch(
                "shutdown_timeout".into()
            ))
        );
        assert!(
            ProtocolErrorData {
                reason: "bogus".into(),
                retryable: false
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn compare_version_oracle() {
        assert!(compare_protocol_version("reasonix.extension.v2", "2").is_ok());
        for (id, version, want) in [
            (
                "reasonix.extension.v1",
                "2",
                ErrorReason::UnsupportedVersion,
            ),
            ("", "2", ErrorReason::UnsupportedVersion),
            (
                "reasonix.extension.v2",
                "1",
                ErrorReason::UnsupportedVersion,
            ),
            (
                "reasonix.extension.v2",
                "0",
                ErrorReason::UnsupportedVersion,
            ),
            ("reasonix.extension.v2", "v2", ErrorReason::ProtocolError),
            ("reasonix.extension.v2", "", ErrorReason::ProtocolError),
        ] {
            let err = compare_protocol_version(id, version).unwrap_err();
            assert_eq!(err.reason, want, "id={id:?} version={version:?}");
        }
    }
}
