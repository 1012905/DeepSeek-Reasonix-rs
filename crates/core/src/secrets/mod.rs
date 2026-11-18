//! Secrets projection helpers shared across the kernel: redaction of
//! credentials from endpoints, configuration values and command
//! arguments (reference: internal/base/secrets).

mod endpoint;
mod url;

pub use endpoint::{
    ENDPOINT_REDACTED, credential_key, credential_value, redact_args, redact_config_map,
    redact_config_value, redact_endpoint,
};
