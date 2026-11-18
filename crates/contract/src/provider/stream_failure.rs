//! Stream failure cause and sentinel errors (Go: `stream_failure.go`).

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A function call in the Responses stream had an invalid wire shape (Go:
/// `StreamFailureInvalidFunctionCall`).
pub const STREAM_FAILURE_INVALID_FUNCTION_CALL: &str = "responses.invalid_function_call";

/// The Responses stream ended while a function call was still unclosed (Go:
/// `StreamFailureUnfinishedFunctionCall`).
pub const STREAM_FAILURE_UNFINISHED_FUNCTION_CALL: &str = "responses.unfinished_function_call";

/// The persisted cause of a refused response stream (Go:
/// `StreamFailureCause`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamFailureCause(Cow<'static, str>);

impl StreamFailureCause {
    /// `responses.invalid_function_call`.
    pub const INVALID_FUNCTION_CALL: StreamFailureCause =
        StreamFailureCause(Cow::Borrowed(STREAM_FAILURE_INVALID_FUNCTION_CALL));

    /// `responses.unfinished_function_call`.
    pub const UNFINISHED_FUNCTION_CALL: StreamFailureCause =
        StreamFailureCause(Cow::Borrowed(STREAM_FAILURE_UNFINISHED_FUNCTION_CALL));

    /// Builds a cause from any string.
    pub fn new(cause: &str) -> StreamFailureCause {
        StreamFailureCause(Cow::Owned(cause.to_owned()))
    }

    /// Cause text as persisted.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the cause is the empty zero value.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Fixed prose per cause, never incorporating provider payloads, tool
    /// arguments, or error prose (Go: `Description`).
    pub fn description(&self) -> &'static str {
        if self.0 == STREAM_FAILURE_INVALID_FUNCTION_CALL {
            "The Responses stream contained a function call with an invalid wire shape. The host refused the whole response; none of its tool calls ran or entered model history."
        } else if self.0 == STREAM_FAILURE_UNFINISHED_FUNCTION_CALL {
            "The Responses stream ended successfully while a function call was still unclosed. The host refused the whole response; none of its tool calls ran or entered model history."
        } else {
            ""
        }
    }
}

impl fmt::Display for StreamFailureCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Sentinel errors for refused streams (Go: `ErrInvalidFunctionCall`,
/// `ErrUnfinishedFunctionCall`).
#[derive(Debug, Error)]
pub enum StreamFailureError {
    #[error("responses: stream contains an invalid function call")]
    InvalidFunctionCall,
    #[error("responses: stream ended with an unfinished function call")]
    UnfinishedFunctionCall,
}

impl StreamFailureError {
    /// The persisted cause of this sentinel (Go: `StreamFailureCauseOf`).
    pub fn cause(&self) -> StreamFailureCause {
        match self {
            StreamFailureError::InvalidFunctionCall => StreamFailureCause::INVALID_FUNCTION_CALL,
            StreamFailureError::UnfinishedFunctionCall => {
                StreamFailureCause::UNFINISHED_FUNCTION_CALL
            }
        }
    }
}
