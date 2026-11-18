//! Event-record wire shape and the schema-version matrix
//! (session_events.go 121-150, composition_events.go).
//!
//! The wire record keeps the messages array encoded until the replay
//! budget has been checked; decoding straight into typed messages
//! would materialize the whole record before max_messages could refuse
//! it.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Timelike};
use reasonix_contract::go_struct;
use reasonix_contract::gojson::{go_raw, go_vec, is_zero_i64, null_default};
use reasonix_contract::provider::Message;
use serde::de::Visitor;
use serde_json::value::RawValue;
use std::fmt;

use super::{
    SESSION_EVENT_COMPOSITION_SCHEMA_VERSION, SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION,
    SESSION_EVENT_LIVE_COMPOSITION_SCHEMA_VERSION, SESSION_EVENT_SCHEMA_VERSION,
};

go_struct! {
    /// One image-blob slot of a record (Go: sessionImageSlot,
    /// session_image_blobs.go 48).
    pub struct SessionImageSlot : SessionImageSlotWire {
        #[serde(
            rename = "blob",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub blob: String,
        #[serde(
            rename = "url",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub url: String,
    }
}

go_struct! {
    /// The image_blobs section of a record: which stored message gets
    /// which slots (Go: sessionEventImages, session_image_blobs.go 55).
    pub struct SessionEventImages : SessionEventImagesWire {
        #[serde(rename = "message", default, deserialize_with = "null_default")]
        pub message: i64,
        #[serde(rename = "slots", default, deserialize_with = "go_vec")]
        pub slots: Vec<SessionImageSlot>,
    }
}

go_struct! {
    /// A raw event record. Go json.Unmarshal(null, &rec) is a no-op
    /// that leaves the zero record, so replay feeds a bare null line
    /// through Option and gets the zero value back (schema 0, which
    /// then fails the native-schema check as a hard error, exactly as
    /// in Go).
    pub struct SessionEventWireRecord : SessionEventWireRecordWire {
        #[serde(rename = "schema_version", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "type", default, deserialize_with = "null_default")]
        pub event_type: String,
        #[serde(
            rename = "revision",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub revision: i64,
        #[serde(
            rename = "base_revision",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub base_revision: i64,
        #[serde(
            rename = "message_index",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub message_index: i64,
        #[serde(
            rename = "messages",
            default,
            deserialize_with = "go_raw",
            skip_serializing_if = "Option::is_none"
        )]
        pub messages: Option<Box<RawValue>>,
        #[serde(
            rename = "content_digest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub content_digest: String,
        #[serde(
            rename = "writer_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub writer_id: String,
        #[serde(
            rename = "reason",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reason: String,
        #[serde(rename = "created_at", default, deserialize_with = "deserialize_created_at")]
        pub created_at: CreatedAt,
        #[serde(
            rename = "image_blobs",
            default,
            deserialize_with = "go_vec",
            skip_serializing_if = "Vec::is_empty"
        )]
        pub image_blobs: Vec<SessionEventImages>,
    }
}

/// Record timestamp with Go time.Time decode semantics for JSON:
/// missing, null, and empty decode to the zero time; anything else
/// must be RFC3339 with the grammar this build's writers emit. The
/// reference additionally accepts any-digit fractional seconds
/// through its RFC3339 fallback; writers never emit them, and the
/// strict grammar here rejects such values as damage, which replay
/// reports the same way (documented in
/// docs/contracts/session-format.md 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedAt(DateTime<FixedOffset>);

impl CreatedAt {
    /// The Go zero time, 0001-01-01T00:00:00Z.
    pub fn zero() -> Self {
        Self(default_time())
    }

    /// The underlying instant (Go: the time.Time value).
    pub fn get(self) -> DateTime<FixedOffset> {
        self.0
    }

    /// Whether this is the Go zero time (Go: `t.IsZero()`), the
    /// `omitzero` predicate for optional sidecar timestamps.
    pub fn is_zero(&self) -> bool {
        self.0 == default_time()
    }

    /// Wraps an existing instant (used by writers stamping "now").
    pub(crate) fn from_datetime(dt: DateTime<FixedOffset>) -> Self {
        Self(dt)
    }
}

impl Default for CreatedAt {
    fn default() -> Self {
        Self::zero()
    }
}

impl serde::Serialize for CreatedAt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Go time.Time JSON is RFC3339Nano with every trailing fractional
        // zero trimmed (0.5s renders ".5", not ".500"); chrono's AutoSi
        // only picks milli/micro/nano units, so hand-roll the trim.
        let secs = self.0.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let nanos = self.0.nanosecond() % 1_000_000_000;
        if nanos == 0 {
            return serializer.serialize_str(&secs);
        }
        let mut frac = format!("{nanos:09}");
        while frac.ends_with('0') {
            frac.pop();
        }
        // Splice ".frac" in front of the zone designator ("Z" is 1 char,
        // a numeric offset like +05:30 is 6).
        let zone_len = if secs.ends_with('Z') { 1 } else { 6 };
        let split = secs.len() - zone_len;
        let mut out = String::with_capacity(secs.len() + frac.len() + 1);
        out.push_str(&secs[..split]);
        out.push('.');
        out.push_str(&frac);
        out.push_str(&secs[split..]);
        serializer.serialize_str(&out)
    }
}

pub(crate) fn default_time() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .and_then(|tz| tz.with_ymd_and_hms(1, 1, 1, 0, 0, 0).single())
        .expect("year 1 with a zero offset is representable")
}

/// The current wall clock (Go: time.Now().UTC(), the stamp event-log
/// writers give records without one).
pub(crate) fn now_utc() -> CreatedAt {
    CreatedAt::from_datetime(chrono::Utc::now().fixed_offset())
}

/// Field deserializer used by the wire records (Go:
/// time.Time.UnmarshalJSON restricted to the writer grammar).
pub fn deserialize_created_at<'de, D>(deserializer: D) -> Result<CreatedAt, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct CreatedAtVisitor;

    impl<'de> Visitor<'de> for CreatedAtVisitor {
        type Value = CreatedAt;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an RFC3339 timestamp string or null")
        }

        fn visit_str<E>(self, value: &str) -> Result<CreatedAt, E>
        where
            E: serde::de::Error,
        {
            if value.is_empty() {
                return Ok(CreatedAt::zero());
            }
            parse_go_rfc3339(value).map(CreatedAt).ok_or_else(|| {
                E::custom(format!("parsing time {value:?}: invalid RFC3339 timestamp"))
            })
        }

        fn visit_unit<E>(self) -> Result<CreatedAt, E>
        where
            E: serde::de::Error,
        {
            Ok(CreatedAt::zero())
        }
    }

    deserializer.deserialize_any(CreatedAtVisitor)
}

/// Parses the RFC3339 subset Go accepts for JSON timestamps written by
/// this family of writers: four-digit year, bounded calendar fields,
/// optional fraction with at least one digit, and either Z or a
/// signed hh:mm offset. Returns None for anything else.
pub(crate) fn parse_go_rfc3339(s: &str) -> Option<DateTime<FixedOffset>> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let mut n: i64 = 0;
        for &c in b.get(range)? {
            if !c.is_ascii_digit() {
                return None;
            }
            n = n * 10 + i64::from(c - b'0');
        }
        Some(n)
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let year = digits(0..4)?;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    let hour = digits(11..13)?;
    let minute = digits(14..16)?;
    let second = digits(17..19)?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut rest = &b[19..];
    let mut nanos: u32 = 0;
    if rest.first() == Some(&b'.') {
        rest = &rest[1..];
        let mut digits_seen = 0usize;
        let mut value: u64 = 0;
        while let Some(&c) = rest.first() {
            if !c.is_ascii_digit() {
                break;
            }
            if digits_seen < 9 {
                value = value * 10 + u64::from(c - b'0');
            }
            digits_seen += 1;
            rest = &rest[1..];
        }
        if digits_seen == 0 {
            return None;
        }
        nanos = (value * 10u64.pow(9 - u32::try_from(digits_seen.min(9)).ok()?)) as u32;
    }
    let offset_secs: i32 = match rest {
        [b'Z'] => 0,
        [sign, h1, h2, b':', m1, m2] if *sign == b'+' || *sign == b'-' => {
            let h = i32::from(*h1 - b'0') * 10 + i32::from(*h2 - b'0');
            let m = i32::from(*m1 - b'0') * 10 + i32::from(*m2 - b'0');
            if h > 23 || m > 59 {
                return None;
            }
            let total = h * 3600 + m * 60;
            if *sign == b'-' { -total } else { total }
        }
        _ => return None,
    };
    let tz = FixedOffset::east_opt(offset_secs)?;
    let naive = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)?.and_hms_nano_opt(
        hour as u32,
        minute as u32,
        second as u32,
        nanos,
    )?;
    tz.from_local_datetime(&naive).single()
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Whether this build writes and replays records of schema v; 2 is the
/// 1.x DAG log, replayed by its own reader (Go:
/// nativeSessionEventSchema, session_events.go 231).
pub fn native_session_event_schema(version: i64) -> bool {
    version == SESSION_EVENT_SCHEMA_VERSION
        || version == SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION
        || version == SESSION_EVENT_COMPOSITION_SCHEMA_VERSION
        || version == SESSION_EVENT_LIVE_COMPOSITION_SCHEMA_VERSION
}

/// Record schema implied by an event body: 3 when it carries image
/// blobs, else 1 (Go: sessionEventRecordSchema, composition_events.go).
pub fn session_event_record_schema(images: &[SessionEventImages]) -> i64 {
    if images.is_empty() {
        SESSION_EVENT_SCHEMA_VERSION
    } else {
        SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION
    }
}

/// Schema of the event a writer emits for msgs: the first message
/// carrying an agent composition decides 5 (live) or 4; without one,
/// the image-blobs rule applies (Go: compositionEventSchema,
/// composition_events.go; the event-type argument is ignored there).
pub fn composition_event_schema(msgs: &[Message], images: &[SessionEventImages]) -> i64 {
    for msg in msgs {
        if let Some(binding) = &msg.agent_composition {
            return if binding.live() {
                SESSION_EVENT_LIVE_COMPOSITION_SCHEMA_VERSION
            } else {
                SESSION_EVENT_COMPOSITION_SCHEMA_VERSION
            };
        }
    }
    session_event_record_schema(images)
}

/// Whether a removal event drops a composition event: true when prev
/// was a composition schema and the removal body is not one (Go:
/// compositionRemoved, composition_events.go; Go always passes nil
/// images, matched here).
pub fn composition_removed(previous: i64, msgs: &[Message]) -> bool {
    (previous == SESSION_EVENT_COMPOSITION_SCHEMA_VERSION
        || previous == SESSION_EVENT_LIVE_COMPOSITION_SCHEMA_VERSION)
        && composition_event_schema(msgs, &[]) != previous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_full_record() {
        let raw = br#"{"schema_version":3,"type":"append","revision":7,"message_index":2,"messages":[{"role":"user","content":"hi"}],"created_at":"2026-11-16T03:04:05Z"}"#;
        let rec: SessionEventWireRecord = serde_json::from_slice(raw).unwrap();
        assert_eq!(rec.schema_version, 3);
        assert_eq!(rec.event_type, "append");
        assert_eq!(rec.revision, 7);
        assert_eq!(rec.message_index, 2);
        assert_eq!(rec.content_digest, "");
        assert!(rec.image_blobs.is_empty());
        let got = rec.messages.as_deref().unwrap().get();
        assert_eq!(got, r#"[{"role":"user","content":"hi"}]"#);
        let t = rec.created_at.get();
        assert_eq!(
            t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "2026-11-16T03:04:05Z"
        );
    }

    #[test]
    fn null_line_decodes_to_zero_record() {
        let rec: Option<SessionEventWireRecord> = serde_json::from_slice(b"null").unwrap();
        let rec = rec.unwrap_or_default();
        assert_eq!(rec.schema_version, 0);
        assert_eq!(rec.event_type, "");
        assert_eq!(rec.created_at, CreatedAt::zero());
    }

    #[test]
    fn record_must_be_an_object() {
        assert!(serde_json::from_slice::<SessionEventWireRecord>(b"[1]").is_err());
        assert!(serde_json::from_slice::<SessionEventWireRecord>(br#""x""#).is_err());
    }

    #[test]
    fn created_at_missing_null_and_empty_are_zero() {
        for raw in [
            br#"{"schema_version":1,"type":"replace"}"#.as_slice(),
            br#"{"schema_version":1,"type":"replace","created_at":null}"#.as_slice(),
            br#"{"schema_version":1,"type":"replace","created_at":""}"#.as_slice(),
        ] {
            let rec: SessionEventWireRecord = serde_json::from_slice(raw).unwrap();
            assert_eq!(rec.created_at, CreatedAt::zero(), "{raw:?}");
        }
    }

    #[test]
    fn created_at_accepts_offset_and_fraction() {
        let raw = br#"{"schema_version":1,"type":"replace","created_at":"2026-11-16T03:04:05.250+05:30"}"#;
        let rec: SessionEventWireRecord = serde_json::from_slice(raw).unwrap();
        let t = rec.created_at.get();
        assert_eq!(t.timestamp_subsec_millis(), 250);
        assert_eq!(
            t.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2026-11-16T03:04:05"
        );
    }

    #[test]
    fn created_at_rejects_invalid_values() {
        for bad in [
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-11-16T24:00:00Z",
            "2026-11-16t03:04:05Z",
            "2026-11-16T03:04:05",
            "2026-11-16T03:04:05.",
            "2026-11-16T03:04:05+25:00",
            "not-a-time",
        ] {
            assert!(parse_go_rfc3339(bad).is_none(), "{bad}");
        }
        let raw = r#"{"schema_version":1,"type":"replace","created_at":"2026-13-01T00:00:00Z"}"#;
        assert!(serde_json::from_slice::<SessionEventWireRecord>(raw.as_bytes()).is_err());
    }

    #[test]
    fn created_at_number_is_rejected() {
        let raw = br#"{"schema_version":1,"type":"replace","created_at":123}"#;
        assert!(serde_json::from_slice::<SessionEventWireRecord>(raw).is_err());
    }

    #[test]
    fn schema_matrix() {
        for ok in [1, 3, 4, 5] {
            assert!(native_session_event_schema(ok), "{ok}");
        }
        for bad in [0, 2, 6] {
            assert!(!native_session_event_schema(bad), "{bad}");
        }
        let empty: Vec<SessionEventImages> = Vec::new();
        assert_eq!(session_event_record_schema(&empty), 1);
        assert_eq!(session_event_record_schema(&empty[..]), 1);
        let with_images = vec![SessionEventImages::default()];
        assert_eq!(session_event_record_schema(&with_images), 3);
    }

    #[test]
    fn composition_schema_picks_first_binding() {
        let plain = Message::default();
        assert_eq!(
            composition_event_schema(std::slice::from_ref(&plain), &[]),
            1
        );
        let live: Message = serde_json::from_str(
            r#"{"agent_composition":{"formatVersion":2,"resourceMode":"live"}}"#,
        )
        .unwrap();
        assert_eq!(
            composition_event_schema(std::slice::from_ref(&live), &[]),
            5
        );
        let pinned: Message =
            serde_json::from_str(r#"{"agent_composition":{"formatVersion":1}}"#).unwrap();
        assert_eq!(
            composition_event_schema(std::slice::from_ref(&pinned), &[]),
            4
        );
        assert_eq!(
            composition_event_schema(&[plain.clone(), pinned.clone()], &[]),
            4
        );
        assert!(composition_removed(4, std::slice::from_ref(&plain)));
        assert!(!composition_removed(4, std::slice::from_ref(&pinned)));
        assert!(!composition_removed(1, std::slice::from_ref(&plain)));
    }

    #[test]
    fn slot_and_images_decode() {
        let raw = br#"{"message":2,"slots":[{"blob":"sha256:abc"},{"url":"data:image/png;base64,AA=="},{"blob":""}]}"#;
        let images: SessionEventImages = serde_json::from_slice(raw).unwrap();
        assert_eq!(images.message, 2);
        assert_eq!(images.slots.len(), 3);
        assert_eq!(images.slots[0].blob, "sha256:abc");
        assert_eq!(images.slots[1].url, "data:image/png;base64,AA==");
        assert_eq!(images.slots[2].blob, "");
    }

    #[test]
    fn created_at_serializes_rfc3339() {
        let t = parse_go_rfc3339("2026-11-16T03:04:05Z").unwrap();
        let s = serde_json::to_string(&CreatedAt(t)).unwrap();
        assert_eq!(s, r#""2026-11-16T03:04:05Z""#);
    }

    #[test]
    fn created_at_serializes_go_rfc3339_nano_trim() {
        // Go time.Time JSON trims every trailing fractional zero: .500
        // renders ".5", .120 renders ".12" (chrono AutoSi would emit
        // ".500"/".120").
        let half = parse_go_rfc3339("2026-11-16T03:04:05.500Z").unwrap();
        assert_eq!(
            serde_json::to_string(&CreatedAt(half)).unwrap(),
            r#""2026-11-16T03:04:05.5Z""#
        );
        let hundredths = parse_go_rfc3339("2026-11-16T03:04:05.120Z").unwrap();
        assert_eq!(
            serde_json::to_string(&CreatedAt(hundredths)).unwrap(),
            r#""2026-11-16T03:04:05.12Z""#
        );
        let nanos = parse_go_rfc3339("2026-11-16T03:04:05.123456789Z").unwrap();
        assert_eq!(
            serde_json::to_string(&CreatedAt(nanos)).unwrap(),
            r#""2026-11-16T03:04:05.123456789Z""#
        );
        let offset = parse_go_rfc3339("2026-11-16T03:04:05.250+05:30").unwrap();
        assert_eq!(
            serde_json::to_string(&CreatedAt(offset)).unwrap(),
            r#""2026-11-16T03:04:05.25+05:30""#
        );
    }
}
