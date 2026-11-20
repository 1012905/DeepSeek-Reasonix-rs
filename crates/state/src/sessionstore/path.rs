//! Session path allocation.
//!
//! Parity target: `reference/.../internal/state/sessionstore/session_path.go`
//! (docs/contracts/session-format.md 2).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

/// Characters Windows forbids in a file name; each is replaced by `-`
/// in the model slug (session-format.md 2).
const UNSAFE_MODEL_CHARS: [char; 9] = ['/', '\\', ':', '<', '>', '"', '|', '?', '*'];

/// Fallback slug when the sanitized model name is empty.
const FALLBACK_MODEL_SLUG: &str = "session";

/// Allocates `<dir>/<UTC timestamp>-<safe model>.jsonl` for a new
/// session (`NewSessionPath`, session-format.md 2).
pub fn new_session_path(dir: impl AsRef<Path>, model: &str) -> PathBuf {
    new_session_path_at(dir, model, SystemTime::now())
}

/// Time-injectable core of [`new_session_path`]; the timestamp uses
/// the Go layout `20060102-150405.000000000`
/// (date-hhmmss.nanoseconds, UTC).
pub fn new_session_path_at(dir: impl AsRef<Path>, model: &str, at: SystemTime) -> PathBuf {
    let file_name = format!(
        "{}-{}.jsonl",
        format_utc_timestamp(at),
        sanitize_model(model),
    );
    dir.as_ref().join(file_name)
}

/// Replaces every [`UNSAFE_MODEL_CHARS`] entry with `-`; an
/// empty result becomes `session` (session-format.md 2).
fn sanitize_model(model: &str) -> String {
    let sanitized = model.replace(UNSAFE_MODEL_CHARS.as_slice(), "-");
    if sanitized.is_empty() {
        FALLBACK_MODEL_SLUG.to_owned()
    } else {
        sanitized
    }
}

/// Go layout `20060102-150405.000000000` in UTC.
fn format_utc_timestamp(at: SystemTime) -> String {
    let at: DateTime<Utc> = at.into();
    at.format("%Y%m%d-%H%M%S%.9f").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use chrono::TimeZone;

    #[test]
    fn sanitizes_every_forbidden_character() {
        assert_eq!(sanitize_model("gpt/4:mini"), "gpt-4-mini");
        assert_eq!(sanitize_model(r"a\b"), "a-b");
        assert_eq!(sanitize_model(r#"a<b>c"d|e?f*g"#), "a-b-c-d-e-f-g");
        // Unicode model names survive untouched.
        assert_eq!(sanitize_model("模型-4"), "模型-4");
        // The empty model becomes the fallback slug.
        assert_eq!(sanitize_model(""), "session");
    }

    #[test]
    fn new_session_path_uses_utc_timestamp_and_safe_model() {
        // 100 ns-aligned nanoseconds: `SystemTime` on Windows is FILETIME
        // backed and cannot represent finer granularity (Go shares this
        // limit on Windows), so the expectation holds on every platform.
        let at = fixed_time(123_456_700);
        assert_eq!(
            new_session_path_at("sessions", "gpt/4:mini", at),
            PathBuf::from("sessions/20261115-100503.123456700-gpt-4-mini.jsonl")
        );
        assert_eq!(
            new_session_path_at("sessions", "", at),
            PathBuf::from("sessions/20261115-100503.123456700-session.jsonl")
        );
    }

    #[test]
    fn new_session_path_smoke() {
        let path = new_session_path("sessions", "m");
        assert!(path.starts_with(Path::new("sessions")));
        assert!(path.to_string_lossy().ends_with("-m.jsonl"));
    }

    fn fixed_time(nanos: u32) -> SystemTime {
        let utc = Utc
            .with_ymd_and_hms(2026, 11, 15, 10, 5, 3)
            .single()
            .expect("valid instant");
        SystemTime::UNIX_EPOCH + Duration::new(utc.timestamp().unsigned_abs(), nanos)
    }
}
