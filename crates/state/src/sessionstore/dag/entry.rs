use reasonix_contract::provider::Role;
use serde::de::IgnoredAny;
use serde_json::Error;

use crate::sessionstore::eventlog::CreatedAt;
use crate::sessionstore::eventlog::record::deserialize_created_at;

use super::json::{nullable, object_fields, string};
use super::replay::PayloadRef;

#[derive(Debug, Default)]
pub(crate) struct Entry {
    pub schema_version: i64,
    pub kind: String,
    pub id: String,
    pub head: String,
    pub at: CreatedAt,
    pub parent: String,
    pub message: Option<PayloadRef>,
    pub target: String,
    pub new_head: String,
    pub from: String,
    pub to: String,
    pub targets: Vec<(String, PayloadRef)>,
}

impl Entry {
    pub fn metadata_bytes(&self) -> i64 {
        let fields = [
            &self.id,
            &self.head,
            &self.parent,
            &self.target,
            &self.new_head,
            &self.from,
            &self.to,
        ];
        let bytes = fields.iter().map(|field| field.len()).sum::<usize>()
            + self.targets.iter().map(|(id, _)| id.len()).sum::<usize>();
        i64::try_from(bytes).unwrap_or(i64::MAX)
    }
}

pub(crate) fn parse_entry(raw: &[u8], start: i64) -> Result<Entry, Error> {
    serde_json::from_slice::<IgnoredAny>(raw)?;
    if raw.first() == Some(&b'n') {
        return Ok(Entry::default());
    }
    let mut entry = Entry::default();
    object_fields(raw, 0, |key, begin, end| {
        let value = &raw[begin..end];
        match key {
            _ if key.eq_ignore_ascii_case("schema_version") => {
                if let Some(value) = nullable(value)? {
                    entry.schema_version = value;
                }
            }
            _ if key.eq_ignore_ascii_case("type") => {
                if let Some(value) = string(value)? {
                    entry.kind = value;
                }
            }
            _ if key.eq_ignore_ascii_case("id") => {
                if let Some(value) = string(value)? {
                    entry.id = value;
                }
            }
            _ if key.eq_ignore_ascii_case("head") => {
                if let Some(value) = string(value)? {
                    entry.head = value;
                }
            }
            _ if key.eq_ignore_ascii_case("at") => {
                if let Some(value) = created_at(value)? {
                    entry.at = value;
                }
            }
            _ if key.eq_ignore_ascii_case("parent") => {
                if let Some(value) = string(value)? {
                    entry.parent = value;
                }
            }
            _ if key.eq_ignore_ascii_case("target") => {
                if let Some(value) = string(value)? {
                    entry.target = value;
                }
            }
            _ if key.eq_ignore_ascii_case("new_head") => {
                if let Some(value) = string(value)? {
                    entry.new_head = value;
                }
            }
            _ if key.eq_ignore_ascii_case("from") => {
                if let Some(value) = string(value)? {
                    entry.from = value;
                }
            }
            _ if key.eq_ignore_ascii_case("to") => {
                if let Some(value) = string(value)? {
                    entry.to = value;
                }
            }
            _ if key.eq_ignore_ascii_case("msgs") => {
                entry.message = Some(PayloadRef {
                    start: start.saturating_add(begin as i64),
                    end: start.saturating_add(end as i64),
                    ..PayloadRef::default()
                });
            }
            _ if key.eq_ignore_ascii_case("targets") => {
                parse_targets(raw, begin, start, &mut entry.targets)?
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(entry)
}

#[derive(serde::Deserialize)]
struct Timestamp(#[serde(deserialize_with = "deserialize_created_at")] CreatedAt);

fn created_at(raw: &[u8]) -> Result<Option<CreatedAt>, Error> {
    if raw == b"null" {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice::<Timestamp>(raw)?.0))
}

fn parse_targets(
    raw: &[u8],
    begin: usize,
    start: i64,
    targets: &mut Vec<(String, PayloadRef)>,
) -> Result<(), Error> {
    if raw.get(begin) == Some(&b'n') {
        targets.clear();
        return Ok(());
    }
    object_fields(raw, begin, |id, begin, end| {
        let payload = PayloadRef {
            start: start.saturating_add(begin as i64),
            end: start.saturating_add(end as i64),
            role: Role::default(),
        };
        if let Some((_, existing)) = targets.iter_mut().find(|(known, _)| known == id) {
            *existing = payload;
        } else {
            targets.push((id.to_owned(), payload));
        }
        Ok(())
    })
}
