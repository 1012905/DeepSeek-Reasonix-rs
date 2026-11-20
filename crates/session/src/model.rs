//! Wire DTOs for the hub read side (OPT-018): every shape the hub answers
//! with is declared here and nowhere else, so serve serializes the same
//! structs the read model builds.
//!
//! Field names and optionality follow the Go `encoding/json` tags they are
//! frozen from: `SessionRow` is http.md §7's `/sessions` entry
//! (serve sessions_list.go), `WorkspaceNode` is serve hub_tree_types.go's
//! `treeWorkspace`, `RuntimeSummary` is serve hub_contract.go's
//! `RuntimeView`.

use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Timelike, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::time::SystemTime;

/// Go `time.Time` JSON: RFC3339 with every trailing fractional zero
/// trimmed (`0.5s` renders `.5`, not `.500`) and the zero time rendered
/// `0001-01-01T00:00:00Z`. Same grammar as reasonix-state's event-log
/// record stamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(DateTime<FixedOffset>);

impl Timestamp {
    /// The Go zero time (Go: `time.Time{}`).
    pub fn zero() -> Self {
        let tz = FixedOffset::east_opt(0).expect("UTC offset is representable");
        let epoch = tz
            .with_ymd_and_hms(1, 1, 1, 0, 0, 0)
            .single()
            .expect("year 1 with a zero offset is representable");
        Self(epoch)
    }

    /// Wraps a wall-clock instant (Go: a `time.Time` value).
    pub fn from_datetime(at: DateTime<FixedOffset>) -> Self {
        Self(at)
    }

    /// The instant a file's metadata reports (Go: `info.ModTime()`).
    pub fn from_system_time(at: SystemTime) -> Self {
        Self(DateTime::<Utc>::from(at).fixed_offset())
    }

    /// The underlying instant.
    pub fn get(self) -> DateTime<FixedOffset> {
        self.0
    }

    /// Whether this is the Go zero time (Go: `t.IsZero()`).
    pub fn is_zero(&self) -> bool {
        self.0 == Self::zero().0
    }

    /// Go `t.UnixNano()`.
    pub fn unix_nanos(self) -> i64 {
        self.0
            .timestamp_nanos_opt()
            .unwrap_or_else(|| self.0.timestamp() * 1_000_000_000)
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render_go_time(self.0))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&render_go_time(self.0))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.trim().is_empty() {
            return Ok(Self::zero());
        }
        DateTime::parse_from_rfc3339(&text)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// Renders `at` the way Go `encoding/json` renders a `time.Time`:
/// whole seconds, plus a trimmed fraction only when nanos are nonzero.
fn render_go_time(at: DateTime<FixedOffset>) -> String {
    let secs = at.to_rfc3339_opts(SecondsFormat::Secs, true);
    let nanos = at.nanosecond() % 1_000_000_000;
    if nanos == 0 {
        return secs;
    }
    let mut frac = format!("{nanos:09}");
    while frac.ends_with('0') {
        frac.pop();
    }
    // Splice ".frac" in front of the zone designator ("Z" is 1 char, a
    // numeric offset like +05:30 is 6).
    let zone_len = if secs.ends_with('Z') { 1 } else { 6 };
    let split = secs.len() - zone_len;
    let mut out = String::with_capacity(secs.len() + frac.len() + 1);
    out.push_str(&secs[..split]);
    out.push('.');
    out.push_str(&frac);
    out.push_str(&secs[split..]);
    out
}

/// One row of `GET /sessions` (http.md §7): `{name, path, title?, turns?,
/// current?, unread?, modified}` — the `?` fields are omitted when empty
/// exactly as Go's `omitempty` tags omit them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub turns: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub current: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unread: bool,
    pub modified: Timestamp,
}

/// One folder of `GET /tree` (serve hub_tree_types.go `treeWorkspace`).
/// The progressive-listing fields (`listingState`, `countsComplete`, ...)
/// stay unlisted until that machine migrates; they are `omitempty` on the
/// wire, so their absence is shape-compatible.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceNode {
    pub root: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub isolated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub remembered: bool,
    pub total: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub archived: i64,
    pub remaining: i64,
    pub sessions: Vec<SessionRow>,
}

/// The whole `GET /tree` body: an array of workspace nodes, never null.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceTreeSnapshot(pub Vec<WorkspaceNode>);

/// One entry of `GET /runtimes` (serve hub_contract.go `RuntimeView`; the
/// agent-composition attachment joins when that machine migrates).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSummary {
    pub id: String,
    /// Route prefix of the runtime's own mux (Go `runtimePrefix + ID`).
    pub base: String,
    pub root: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
}

/// A runtime handle name. Go mints `"r"+seq`; only the write side mints
/// one, so a read-side build never produces a fresh ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeId(pub String);

impl fmt::Display for RuntimeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for RuntimeId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

/// `POST /runtimes` body (serve hub_contract.go `OpenRequest`; the
/// agent-composition slot joins with that machine).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRuntimeRequest {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub session_path: String,
    #[serde(default)]
    pub model: String,
}

/// Typed refusal of a hub request (docs/contract.md §4.1: no machine, no
/// fake success). Serve maps each variant to its own refusal envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HubRefusal {
    /// The runtime open/publish machine is not migrated in this build.
    UnsupportedInThisBuild,
}

impl fmt::Display for HubRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HubRefusal::UnsupportedInThisBuild => {
                f.write_str("runtime open is not part of this build")
            }
        }
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_row_omits_empty_optional_fields() {
        let row = SessionRow {
            name: "s".into(),
            path: "/d/s.jsonl".into(),
            modified: Timestamp::zero(),
            ..SessionRow::default()
        };
        let json = serde_json::to_value(&row).unwrap();
        let map = json.as_object().unwrap();
        let keys: std::collections::HashSet<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(keys, ["name", "path", "modified"].into_iter().collect());
        assert_eq!(map["modified"], "0001-01-01T00:00:00Z");
    }

    #[test]
    fn session_row_carries_the_seven_frozen_fields() {
        let row = SessionRow {
            name: "s".into(),
            path: "/d/s.jsonl".into(),
            title: "t".into(),
            turns: 3,
            current: true,
            unread: true,
            modified: Timestamp::from_datetime(
                chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05.5Z").unwrap(),
            ),
        };
        let json = serde_json::to_value(&row).unwrap();
        let map = json.as_object().unwrap();
        for key in [
            "name", "path", "title", "turns", "current", "unread", "modified",
        ] {
            assert!(map.contains_key(key), "missing {key}");
        }
        assert_eq!(map["modified"], "2026-01-02T03:04:05.5Z");
    }

    #[test]
    fn workspace_node_sessions_is_never_null() {
        let node = WorkspaceNode::default();
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains(r#""sessions":[]"#), "{json}");
        assert!(json.contains(r#""total":0"#), "{json}");
        assert!(json.contains(r#""remaining":0"#), "{json}");
        assert!(!json.contains("workspaceRoots"), "{json}");
    }

    #[test]
    fn workspace_tree_snapshot_serializes_as_an_array() {
        let json = serde_json::to_string(&WorkspaceTreeSnapshot(vec![WorkspaceNode {
            root: "/r".into(),
            name: "r".into(),
            ..WorkspaceNode::default()
        }]))
        .unwrap();
        assert!(json.starts_with('[') && json.ends_with(']'), "{json}");
    }

    #[test]
    fn runtime_summary_uses_go_field_names() {
        let json = serde_json::to_value(RuntimeSummary {
            id: "r1".into(),
            base: "/rt/r1".into(),
            root: "/r".into(),
            name: "r".into(),
            session_path: "/d/s.jsonl".into(),
            ..RuntimeSummary::default()
        })
        .unwrap();
        let map = json.as_object().unwrap();
        for key in ["id", "base", "root", "name", "sessionPath"] {
            assert!(map.contains_key(key), "missing {key}");
        }
        assert!(!map.contains_key("host") && !map.contains_key("readOnly"));
    }

    #[test]
    fn timestamp_trims_fractional_zeros_like_go() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05.500Z").unwrap();
        assert_eq!(
            Timestamp::from_datetime(at).to_string(),
            "2026-01-02T03:04:05.5Z"
        );
        let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05.000000001Z").unwrap();
        assert_eq!(
            Timestamp::from_datetime(at).to_string(),
            "2026-01-02T03:04:05.000000001Z"
        );
    }
}
