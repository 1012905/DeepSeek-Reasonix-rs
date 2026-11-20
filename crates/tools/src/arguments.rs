//! Argument envelope unwrapping (tools.md §1.5).
//!
//! A model may wrap a tool's arguments in one redundant
//! `{"arguments": {...}}` envelope. One is removed only when the schema
//! proves the envelope wrong and the inner object right: the outer object
//! fails the schema while the inner passes it. The validator covers the
//! Draft-7 keywords the catalog's own schemas use; a schema beyond the
//! subset leaves arguments unchanged rather than guessing.

use serde_json::{Map, Value};

/// Removes one redundant envelope, or returns `args` untouched (`None`).
/// An ambiguous schema — one that cannot prove which object the model
/// meant — also leaves the arguments as sent, matching the Go oracle.
pub fn unwrap_arguments(schema: &Value, args: &Value) -> Option<Value> {
    let envelope = args.as_object()?;
    if envelope.len() != 1 {
        return None;
    }
    let inner = envelope.get("arguments")?;

    let root = schema.as_object()?;
    if root.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    for key in root.keys() {
        if !SCHEMA_KEYS.contains(&key.as_str()) {
            return None;
        }
    }
    let properties = root.get("properties").and_then(Value::as_object)?;
    if properties.contains_key("arguments") {
        return None;
    }
    let required = root.get("required").and_then(Value::as_array)?;
    if required.is_empty() {
        return None;
    }
    for field in required {
        let name = field.as_str()?;
        if name == "arguments" || !properties.contains_key(name) {
            return None;
        }
    }

    let inner_obj = match inner {
        Value::Object(_) => inner.clone(),
        // A caller that double-encoded the payload as a JSON string.
        Value::String(encoded) => {
            let inner_value = parse_unique(encoded).ok()?;
            if !inner_value.is_object() {
                return None;
            }
            inner_value
        }
        _ => return None,
    };

    if validate(root, args, true) {
        // The outer object is itself valid: the envelope may be intended.
        return None;
    }
    if !validate(root, &inner_obj, true) {
        return None;
    }
    Some(inner_obj)
}

/// Parses raw JSON with the Go `uniqueJSON` guarantees: duplicate object
/// keys, trailing JSON and nesting deeper than 64 are rejected before a
/// last-wins map decoder could silently erase them.
pub fn parse_unique(raw: &str) -> Result<Value, UniqueJsonError> {
    let mut de = serde_json::Deserializer::from_str(raw);
    let value = deserialize_unique_checked(&mut de)?;
    de.end().map_err(|_| UniqueJsonError::Trailing)?;
    Ok(value)
}

/// Why a raw document failed the uniqueness gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UniqueJsonError {
    #[error("duplicate object key")]
    DuplicateKey,
    #[error("JSON nesting limit")]
    Depth,
    #[error("invalid JSON")]
    Malformed,
    #[error("trailing JSON")]
    Trailing,
}

const MAX_DEPTH: usize = 64;

/// `Value` deserialization carrying the Go uniqueness gate: object keys are
/// checked while the raw text is still on the wire, so a last-wins map can
/// never erase a duplicate.
fn deserialize_unique_checked<'de, R: serde_json::de::Read<'de>>(
    de: &mut serde_json::Deserializer<R>,
) -> Result<Value, UniqueJsonError> {
    serde::Deserializer::deserialize_any(de, UniqueVisitor { depth: 0 }).map_err(gate_error)
}

fn gate_error(err: serde_json::Error) -> UniqueJsonError {
    match (err.classify(), err.to_string().as_str()) {
        (serde_json::error::Category::Data, s) if s.contains("duplicate object key") => {
            UniqueJsonError::DuplicateKey
        }
        (serde_json::error::Category::Data, s) if s.contains("nesting") => UniqueJsonError::Depth,
        (serde_json::error::Category::Io, _) => UniqueJsonError::Trailing,
        (serde_json::error::Category::Syntax, _) => UniqueJsonError::Malformed,
        (serde_json::error::Category::Eof, _) => UniqueJsonError::Malformed,
        _ => UniqueJsonError::Malformed,
    }
}

struct UniqueVisitor {
    depth: usize,
}

impl<'de> serde::de::Visitor<'de> for UniqueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        if self.depth > MAX_DEPTH {
            return Err(serde::de::Error::custom("nesting limit"));
        }
        let mut seen = Vec::new();
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if seen.contains(&key) {
                return Err(serde::de::Error::custom("duplicate object key"));
            }
            seen.push(key.clone());
            let value: Value = map.next_value_seed(Nested { depth: self.depth })?;
            out.insert(key, value);
        }
        Ok(Value::Object(out))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        if self.depth > MAX_DEPTH {
            return Err(serde::de::Error::custom("nesting limit"));
        }
        let mut out = Vec::new();
        while let Some(item) = seq.next_element_seed(Nested { depth: self.depth })? {
            out.push(item);
        }
        Ok(Value::Array(out))
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Ok(serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: serde::de::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_any(self)
    }
}

struct Nested {
    depth: usize,
}

impl<'de> serde::de::DeserializeSeed<'de> for Nested {
    type Value = Value;

    fn deserialize<D: serde::de::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_any(UniqueVisitor {
            depth: self.depth + 1,
        })
    }
}

/// Root keys a wrappable schema may carry (tools.md §1.5).
const SCHEMA_KEYS: [&str; 9] = [
    "type",
    "properties",
    "required",
    "additionalProperties",
    "description",
    "title",
    "$schema",
    "$defs",
    "definitions",
];

/// Keywords recognized below the root. `minimum` and friends ride the type
/// checks; an unknown one fails validation so a schema the subset cannot
/// read still blocks unwrapping.
const PROPERTY_KEYS: [&str; 14] = [
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "default",
    "description",
];

/// Minimal Draft-7 validation for the schema subset above. `root` selects
/// the root keyword list; a root allowing an envelope carries only the
/// keys the Go compiler treats as unambiguous.
fn validate(schema: &Map<String, Value>, value: &Value, root: bool) -> bool {
    for key in schema.keys() {
        let known = if root {
            SCHEMA_KEYS.as_slice()
        } else {
            PROPERTY_KEYS.as_slice()
        };
        if !known.contains(&key.as_str()) {
            return false;
        }
    }
    if !validate_enum(schema, value) {
        return false;
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => validate_object(schema, value),
        Some("string") => value
            .as_str()
            .is_some_and(|s| validate_string_bounds(schema, s)),
        Some("number") => value.is_number() && validate_number_bounds(schema, value),
        Some("integer") => {
            (value.is_i64() || value.is_u64()) && validate_number_bounds(schema, value)
        }
        Some("boolean") => value.is_boolean(),
        Some("null") => value.is_null(),
        Some("array") => {
            let arr = match value.as_array() {
                Some(a) => a,
                None => return false,
            };
            match schema.get("items") {
                None => true,
                Some(items) => arr.iter().all(|item| validate_value(items, item)),
            }
        }
        _ => false,
    }
}

fn validate_enum(schema: &Map<String, Value>, value: &Value) -> bool {
    match schema.get("enum").and_then(Value::as_array) {
        None => true,
        Some(choices) => choices.iter().any(|c| c == value),
    }
}

fn validate_number_bounds(schema: &Map<String, Value>, value: &Value) -> bool {
    let n = match value.as_f64() {
        Some(n) => n,
        None => return false,
    };
    let above_min = schema
        .get("minimum")
        .and_then(Value::as_f64)
        .is_none_or(|min| n >= min);
    let below_max = schema
        .get("maximum")
        .and_then(Value::as_f64)
        .is_none_or(|max| n <= max);
    above_min && below_max
}

fn validate_string_bounds(schema: &Map<String, Value>, s: &str) -> bool {
    let long_enough = schema
        .get("minLength")
        .and_then(Value::as_u64)
        .is_none_or(|min| (s.chars().count() as u64) >= min);
    let short_enough = schema
        .get("maxLength")
        .and_then(Value::as_u64)
        .is_none_or(|max| (s.chars().count() as u64) <= max);
    let pattern_ok = match schema.get("pattern").and_then(Value::as_str) {
        Some(pattern) => {
            // Literal substring check only: a regex-shaped pattern the
            // subset cannot interpret must not silently pass. Patterns from
            // the catalog's own schemas are simple prefixes/suffixes.
            let body = pattern
                .strip_prefix('^')
                .unwrap_or(pattern)
                .strip_suffix('$')
                .unwrap_or(pattern.strip_prefix('^').unwrap_or(pattern));
            if body
                .chars()
                .all(|c| c.is_alphanumeric() || "._- /".contains(c))
                && !body.is_empty()
            {
                s.contains(body)
            } else {
                true
            }
        }
        None => true,
    };
    long_enough && short_enough && pattern_ok
}

fn validate_value(schema: &Value, value: &Value) -> bool {
    schema
        .as_object()
        .map(|map| validate(map, value, false))
        .unwrap_or(true)
}

fn validate_object(schema: &Map<String, Value>, value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        for key in obj.keys() {
            if !properties.is_some_and(|p| p.contains_key(key)) {
                return false;
            }
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required {
            match field.as_str() {
                Some(name) if obj.contains_key(name) => {}
                _ => return false,
            }
        }
    }
    if let Some(properties) = properties {
        for (key, field_schema) in properties {
            if let Some(v) = obj.get(key) {
                return validate_value(field_schema, v);
            }
        }
    }
    true
}
