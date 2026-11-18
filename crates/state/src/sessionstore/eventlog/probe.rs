//! Event-log classification probe (session_event_probe.go).
//!
//! A cheap 4KiB header sniff decides how a log can be consumed:
//! native (this build's schema family and a known record type), DAG
//! (schema 2), future (a newer schema), or unknown. Files that are
//! absent or empty probe as native; oversized or missing sidecar names
//! make the probe give up and report nothing.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::record::native_session_event_schema;
use super::{
    SESSION_EVENT_DAG_SCHEMA_VERSION, SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION,
    SESSION_EVENT_PROBE_MAX_BYTES, SESSION_EVENT_TYPE_APPEND, SESSION_EVENT_TYPE_REPLACE,
    SessionReplayLimits,
};
use reasonix_contract::go_struct;

/// Sidecar-name budget shared by all session files (Go: nameMaxBytes).
pub(crate) const NAME_MAX_BYTES: usize = 255;

/// Classification of one event log (Go: sessionEventLogProbe).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionEventLogProbe {
    /// File size in bytes, when the file was seen.
    pub size: i64,
    /// True when the log is absent, empty, or decodable by this build.
    pub native: bool,
    /// True when the log uses a schema newer than this build knows.
    pub future_schema: bool,
    /// True when the log is the 1.x DAG format (schema 2).
    pub dag: bool,
    /// Schema version reported in the header, when found.
    pub schema_version: i64,
}

/// Probes the event log next to a session path with default limits
/// (Go: probeSessionEventLog).
pub fn probe_session_event_log(session_path: &Path) -> io::Result<SessionEventLogProbe> {
    probe_session_event_log_with_limits(session_path, &SessionReplayLimits::default())
}

/// Probes with explicit limits (Go: probeSessionEventLogWithLimits).
pub fn probe_session_event_log_with_limits(
    session_path: &Path,
    limits: &SessionReplayLimits,
) -> io::Result<SessionEventLogProbe> {
    let log_path = crate::store::session_event_log(session_path);
    let log_str = log_path.to_string_lossy().into_owned();
    if log_str.is_empty() {
        return Ok(SessionEventLogProbe {
            native: true,
            ..Default::default()
        });
    }
    if !sidecars_fit(session_path) {
        return Ok(SessionEventLogProbe::default());
    }
    let meta = match std::fs::metadata(&log_path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(SessionEventLogProbe {
                native: true,
                ..Default::default()
            });
        }
        Err(err) => return Err(err),
    };
    if meta.is_dir() {
        return Ok(SessionEventLogProbe::default());
    }
    let size = meta.len() as i64;
    if size == 0 {
        return Ok(SessionEventLogProbe {
            native: true,
            ..Default::default()
        });
    }
    let mut probe = SessionEventLogProbe {
        size,
        ..Default::default()
    };
    let mut file = File::open(&log_path)?;
    let (mut schema, mut event_type, mut ok) = probe_session_event_header_bytes(&mut file);
    if !ok && size <= limits.max_bytes {
        file.seek(SeekFrom::Start(0))?;
        let mut limited = file.take(limits.max_bytes as u64 + 1);
        let mut raw = Vec::new();
        if limited.read_to_end(&mut raw).is_ok()
            && let Some(header) = decode_header_minimal(&raw)
        {
            schema = header.schema_version;
            event_type = header.event_type;
            ok = true;
        }
    }
    if !ok {
        return Ok(probe);
    }
    probe.schema_version = schema;
    probe.dag = schema == SESSION_EVENT_DAG_SCHEMA_VERSION;
    probe.native = native_session_event_schema(schema)
        && (event_type == SESSION_EVENT_TYPE_REPLACE || event_type == SESSION_EVENT_TYPE_APPEND);
    if !probe.native && schema > SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION {
        probe.future_schema = true;
    }
    Ok(probe)
}

/// Sidecar basenames must fit the shared name budget (Go:
/// sessionEventSidecarsFit).
fn sidecars_fit(session_path: &Path) -> bool {
    let event_log = crate::store::session_event_log(session_path);
    let index = crate::store::session_event_index(session_path);
    let name = |p: &std::path::PathBuf| -> usize {
        p.file_name()
            .map(|n| n.as_encoded_bytes().len())
            .unwrap_or(0)
    };
    name(&event_log) <= NAME_MAX_BYTES && name(&index) <= NAME_MAX_BYTES
}

/// Header sniff over the first 4KiB: extracts schema_version and type
/// early, tolerating truncation after both are seen (Go:
/// probeSessionEventHeader). Returns (schema, type, ok).
fn probe_session_event_header_bytes(file: &mut File) -> (i64, String, bool) {
    let mut head = [0u8; SESSION_EVENT_PROBE_MAX_BYTES as usize];
    let n = match file.read(&mut head) {
        Ok(0) => return (0, String::new(), false),
        Ok(n) => n,
        Err(_) => return (0, String::new(), false),
    };
    let raw = &head[..n];
    let mut pos = 0usize;
    skip_ws(raw, &mut pos);
    if raw.get(pos) != Some(&b'{') {
        return (0, String::new(), false);
    }
    pos += 1;
    let mut schema: i64 = 0;
    let mut event_type = String::new();
    let mut have_schema = false;
    let mut have_type = false;
    loop {
        skip_ws(raw, &mut pos);
        match raw.get(pos) {
            None | Some(b'}') => break,
            Some(b'"') => {}
            Some(_) => return (0, String::new(), false),
        }
        let Some(key_end) = json_string_extent(raw, pos) else {
            return (0, String::new(), false);
        };
        let Ok(key) = serde_json::from_slice::<String>(&raw[pos..key_end]) else {
            return (0, String::new(), false);
        };
        pos = key_end;
        skip_ws(raw, &mut pos);
        if raw.get(pos) != Some(&b':') {
            return (0, String::new(), false);
        }
        pos += 1;
        skip_ws(raw, &mut pos);
        match key.as_str() {
            "schema_version" => {
                let Some(value_end) = value_extent(raw, pos) else {
                    return (0, String::new(), false);
                };
                if let Ok(v) = serde_json::from_slice::<Option<i64>>(&raw[pos..value_end]) {
                    if let Some(v) = v {
                        schema = v;
                    }
                    have_schema = true;
                } else {
                    return (0, String::new(), false);
                }
                pos = value_end;
            }
            "type" => {
                let Some(value_end) = value_extent(raw, pos) else {
                    return (0, String::new(), false);
                };
                if let Ok(v) = serde_json::from_slice::<Option<String>>(&raw[pos..value_end]) {
                    if let Some(v) = v {
                        event_type = v;
                    }
                    have_type = true;
                } else {
                    return (0, String::new(), false);
                }
                pos = value_end;
            }
            _ => {
                let Some(value_end) = value_extent(raw, pos) else {
                    return (0, String::new(), false);
                };
                if serde_json::from_slice::<serde::de::IgnoredAny>(&raw[pos..value_end]).is_err() {
                    return (0, String::new(), false);
                }
                pos = value_end;
            }
        }
        if have_schema && have_type {
            // Both fields seen: stop scanning even if the buffer ends
            // mid-record (Go returns early once both are decoded).
            return (schema, event_type, true);
        }
        skip_ws(raw, &mut pos);
        match raw.get(pos) {
            Some(b',') => pos += 1,
            Some(b'}') => break,
            _ => return (0, String::new(), false),
        }
    }
    if have_schema && have_type {
        return (schema, event_type, true);
    }
    (0, String::new(), false)
}

/// Minimal header decode fallback (Go: the anonymous header struct
/// decoded over a LimitedReader).
fn decode_header_minimal(raw: &[u8]) -> Option<HeaderPartial> {
    serde_json::from_slice::<HeaderPartial>(raw).ok()
}

go_struct! {
    /// Header-only view used by the minimal decode fallback.
    pub struct HeaderPartial : HeaderPartialWire {
        #[serde(default)]
        pub schema_version: i64,
        #[serde(default, rename = "type")]
        pub event_type: String,
    }
}

fn skip_ws(raw: &[u8], pos: &mut usize) {
    while matches!(raw.get(*pos), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        *pos += 1;
    }
}

/// End offset (exclusive) of a JSON string starting at pos, or None.
fn json_string_extent(raw: &[u8], mut pos: usize) -> Option<usize> {
    if raw.get(pos) != Some(&b'"') {
        return None;
    }
    pos += 1;
    loop {
        match raw.get(pos)? {
            b'\\' => pos += 2,
            b'"' => return Some(pos + 1),
            _ => pos += 1,
        }
    }
}

/// End offset (exclusive) of any JSON value starting at pos, or None
/// when the value is truncated or malformed at the surface.
fn value_extent(raw: &[u8], mut pos: usize) -> Option<usize> {
    match *raw.get(pos)? {
        b'"' => json_string_extent(raw, pos),
        b'{' | b'[' => {
            let open = raw[pos];
            let close = if open == b'{' { b'}' } else { b']' };
            let mut depth = 0i64;
            let mut quoted = false;
            let mut escaped = false;
            loop {
                let &b = raw.get(pos)?;
                if quoted {
                    if escaped {
                        escaped = false;
                    } else if b == b'\\' {
                        escaped = true;
                    } else if b == b'"' {
                        quoted = false;
                    }
                } else {
                    match b {
                        b'"' => quoted = true,
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                debug_assert_eq!(b, close);
                                return Some(pos + 1);
                            }
                        }
                        _ => {}
                    }
                }
                pos += 1;
            }
        }
        b't' => {
            if raw[pos..].starts_with(b"true") {
                Some(pos + 4)
            } else {
                None
            }
        }
        b'f' => {
            if raw[pos..].starts_with(b"false") {
                Some(pos + 5)
            } else {
                None
            }
        }
        b'n' => {
            if raw[pos..].starts_with(b"null") {
                Some(pos + 4)
            } else {
                None
            }
        }
        b'-' | b'0'..=b'9' => {
            while matches!(
                raw.get(pos),
                Some(b'-' | b'+' | b'.' | b'0'..=b'9' | b'e' | b'E')
            ) {
                pos += 1;
            }
            Some(pos)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support;
    use std::io::Write;

    fn write_log(dir: &tempfile::TempDir, body: &[u8]) -> std::path::PathBuf {
        let session = dir.path().join("20260101-000000.000000000-model.jsonl");
        let log = crate::store::session_event_log(&session);
        let mut f = File::create(&log).unwrap();
        f.write_all(body).unwrap();
        session
    }

    #[test]
    fn missing_log_probes_native() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("missing.jsonl");
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
        assert_eq!(probe.size, 0);
    }

    #[test]
    fn empty_log_probes_native() {
        let tmp = test_support::tempdir();
        let session = write_log(&tmp, b"");
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
    }

    #[test]
    fn directory_probes_zero() {
        let tmp = test_support::tempdir();
        let session = tmp.path().join("d.jsonl");
        std::fs::create_dir_all(crate::store::session_event_log(&session)).unwrap();
        let probe = probe_session_event_log(&session).unwrap();
        assert_eq!(probe, SessionEventLogProbe::default());
    }

    #[test]
    fn garbage_probes_zero() {
        let tmp = test_support::tempdir();
        let session = write_log(&tmp, b"not json at all");
        let probe = probe_session_event_log(&session).unwrap();
        // Neither the 4KiB sniff nor the minimal decode can read the
        // file; the probe keeps only the observed size.
        assert!(!probe.native);
        assert!(!probe.future_schema);
        assert!(!probe.dag);
        assert_eq!(probe.size, 15);
    }

    #[test]
    fn dag_schema_probes_dag() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":2,"type":"replace","messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.dag);
        assert!(!probe.native);
        assert_eq!(probe.schema_version, 2);
    }

    #[test]
    fn future_schema_probes_future() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":6,"type":"replace","messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.future_schema);
        assert!(!probe.native);
        assert_eq!(probe.schema_version, 6);
    }

    #[test]
    fn native_schema_and_type_probes_native() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":1,"type":"replace","messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
        assert!(!probe.dag);
        assert!(!probe.future_schema);
        assert_eq!(probe.schema_version, 1);
    }

    #[test]
    fn native_schema_without_type_is_not_native() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":1,"messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(!probe.native);
        assert!(!probe.future_schema);
        assert_eq!(probe.schema_version, 1);
    }

    #[test]
    fn schema_three_with_unknown_type_is_not_future() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":3,"type":"weird","messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(!probe.native);
        assert!(!probe.future_schema);
    }

    #[test]
    fn minimal_decode_covers_large_header() {
        let tmp = test_support::tempdir();
        // The record is larger than the 4KiB sniff window, but the two
        // header fields sit inside it; the fast path still finds them.
        let big = "x".repeat(8192);
        let body =
            format!(r#"{{"schema_version":1,"type":"append","reason":"{big}"}}"#).into_bytes();
        let session = write_log(&tmp, &body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
        assert_eq!(probe.schema_version, 1);
    }

    #[test]
    fn header_fields_beyond_snip_need_minimal_decode() {
        let tmp = test_support::tempdir();
        // schema_version sits past the 4KiB window; only the minimal
        // fallback over the whole (small) file can see it.
        let pad = "0".repeat(5000);
        let body =
            format!(r#"{{"reason":"{pad}","schema_version":1,"type":"replace","messages":[]}}"#)
                .into_bytes();
        let session = write_log(&tmp, &body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
    }

    #[test]
    fn escaped_type_value_matches() {
        let tmp = test_support::tempdir();
        let body = br#"{"schema_version":1,"type":"repl\u0061ce","messages":[]}"#;
        let session = write_log(&tmp, body);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(probe.native);
    }

    #[test]
    fn null_header_values_are_no_ops() {
        // Go decodes JSON null into the header fields as a no-op: the
        // field counts as seen but keeps its zero value, so the probe
        // classifies the log by what the null left behind.
        let tmp = test_support::tempdir();
        let session = write_log(&tmp, br#"{"schema_version":null,"type":"replace"}"#);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(!probe.native);
        assert!(!probe.future_schema);
        assert_eq!(probe.schema_version, 0);

        let session = write_log(&tmp, br#"{"schema_version":1,"type":null}"#);
        let probe = probe_session_event_log(&session).unwrap();
        assert!(!probe.native);
        assert_eq!(probe.schema_version, 1);
    }

    #[test]
    fn blank_session_path_probes_native() {
        // Go returns the zero-cost native probe for "" instead of touching
        // the filesystem; the naming guards make the same true here.
        let probe = probe_session_event_log(Path::new("")).unwrap();
        assert!(probe.native);
        assert!(!probe.dag);
        assert!(!probe.future_schema);
    }

    #[test]
    fn oversized_sidecar_names_probe_zero() {
        // A session whose sidecar names would overflow the 255-byte file
        // name limit cannot be native; sidecar fit is checked before any
        // filesystem access, so no file is ever created.
        let name = "s".repeat(300);
        let session = std::path::PathBuf::from(format!("{name}.jsonl"));
        let probe = probe_session_event_log(&session).unwrap();
        assert_eq!(probe.size, 0);
        assert!(!probe.native);
        assert!(!probe.dag);
        assert!(!probe.future_schema);
        assert_eq!(probe.schema_version, 0);
    }
}
