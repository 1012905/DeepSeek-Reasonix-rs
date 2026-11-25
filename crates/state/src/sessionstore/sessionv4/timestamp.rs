use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Timelike, Utc};
use serde::{Deserialize, Deserializer, Serializer};

pub(crate) fn zero() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .and_then(|zone| zone.with_ymd_and_hms(1, 1, 1, 0, 0, 0).single())
        .expect("the Go zero time is representable")
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<FixedOffset>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value {
        None => Ok(zero()),
        Some(value) if value.is_empty() => Ok(zero()),
        Some(value) => DateTime::parse_from_rfc3339(&value).map_err(serde::de::Error::custom),
    }
}

pub(crate) fn serialize<S>(value: &DateTime<FixedOffset>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let seconds = value.to_rfc3339_opts(SecondsFormat::Secs, true);
    let nanos = value.nanosecond();
    if nanos == 0 {
        return serializer.serialize_str(&seconds);
    }
    let mut fraction = format!("{nanos:09}");
    while fraction.ends_with('0') {
        fraction.pop();
    }
    let zone_len = if seconds.ends_with('Z') { 1 } else { 6 };
    let split = seconds.len() - zone_len;
    serializer.serialize_str(&format!(
        "{}.{fraction}{}",
        &seconds[..split],
        &seconds[split..]
    ))
}

pub(crate) fn utc_nano(value: DateTime<FixedOffset>) -> String {
    serialize_to_string(value.with_timezone(&Utc).fixed_offset())
}

fn serialize_to_string(value: DateTime<FixedOffset>) -> String {
    let seconds = value.to_rfc3339_opts(SecondsFormat::Secs, true);
    let nanos = value.nanosecond();
    if nanos == 0 {
        return seconds;
    }
    let mut fraction = format!("{nanos:09}");
    while fraction.ends_with('0') {
        fraction.pop();
    }
    let split = seconds.len() - 1;
    format!("{}.{fraction}{}", &seconds[..split], &seconds[split..])
}
