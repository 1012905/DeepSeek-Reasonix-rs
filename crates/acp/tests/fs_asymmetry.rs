//! ACP-006: fs proxy degradation asymmetry.
//!
//! Reads fall back locally on any miss; writes fall back only when the
//! capability is absent, and surface a present-capability client error instead
//! of double-applying.

use reasonix_acp::caps::{ClientCapabilities, FsCapabilities};
use reasonix_acp::fs_api::{
    FsReadTextFileParams, FsReadTextFileResult, ReadFailure, ReadOutcome, WriteOutcome,
    resolve_read, resolve_write,
};

fn caps(read: bool, write: bool) -> ClientCapabilities {
    ClientCapabilities {
        fs: Some(FsCapabilities {
            read_text_file: read,
            write_text_file: write,
        }),
        terminal: false,
        elicitation: None,
        meta: None,
    }
}

#[test]
fn read_degrades_on_missing_capability_transport_or_parse() {
    let no_cap = caps(false, false);
    assert!(matches!(
        resolve_read(&no_cap, Ok("ignored")),
        ReadOutcome::LocalFallback(ReadFailure::CapabilityAbsent)
    ));

    let with_cap = caps(true, false);
    assert!(matches!(
        resolve_read(&with_cap, Err(ReadFailure::Transport)),
        ReadOutcome::LocalFallback(ReadFailure::Transport)
    ));
    assert!(matches!(
        resolve_read(&with_cap, Err(ReadFailure::BadReply)),
        ReadOutcome::LocalFallback(ReadFailure::BadReply)
    ));
    assert_eq!(
        resolve_read(&with_cap, Ok("client view")),
        ReadOutcome::Client("client view".to_owned())
    );
}

#[test]
fn write_falls_back_only_when_capability_absent() {
    let no_cap = caps(false, false);
    assert_eq!(resolve_write(&no_cap, true), WriteOutcome::LocalFallback);
    assert_eq!(resolve_write(&no_cap, false), WriteOutcome::LocalFallback);

    let with_cap = caps(false, true);
    assert_eq!(resolve_write(&with_cap, false), WriteOutcome::Written);
    assert_eq!(resolve_write(&with_cap, true), WriteOutcome::SurfacedError);
}

#[test]
fn read_params_omit_line_limit_and_result_carries_content_only() {
    let params = FsReadTextFileParams {
        session_id: "s1".to_owned(),
        path: "src/main.rs".to_owned(),
        line: None,
        limit: None,
    };
    let v = serde_json::to_value(&params).unwrap();
    assert!(v.get("line").is_none());
    assert!(v.get("limit").is_none());

    let result = FsReadTextFileResult {
        content: "hello".to_owned(),
    };
    let v = serde_json::to_value(&result).unwrap();
    assert_eq!(v, serde_json::json!({"content": "hello"}));
    assert!(v.get("truncated").is_none());
}
