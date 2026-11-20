//! Schema canonicalization (Go `provider.CanonicalizeSchema`): one stable
//! byte representation per logical schema, computed once at registration.

use serde_json::{Map, Value};

/// The strict empty-object schema an absent or null schema becomes: an empty
/// payload would fail the enclosing provider request and brick it whole.
pub fn empty_object_schema() -> Value {
    serde_json::json!({"properties": {}, "required": [], "type": "object"})
}

/// Recursively stabilizes a JSON Schema so the same logical schema always
/// produces the same bytes. Keys are sorted; `required` arrays sort
/// lexically; the root gains `type: "object"`, `properties` and a `required`
/// array where an MCP server omitted them.
pub fn canonicalize_schema(raw: &Value) -> Value {
    let mut v = raw.clone();
    if v.is_null() {
        return empty_object_schema();
    }
    canonicalize_value(&mut v);
    ensure_root_object_properties(&mut v);
    v
}

fn ensure_root_object_properties(v: &mut Value) {
    let Some(map) = v.as_object_mut() else {
        return;
    };
    if !map.contains_key("type") {
        // Tool arguments are always JSON objects; an omitted root type can
        // only mean an object schema. Making it explicit keeps a usable tool
        // out of validation quarantine.
        map.insert("type".to_owned(), Value::String("object".to_owned()));
    }
    if map.get("type").and_then(Value::as_str) != Some("object") {
        return;
    }
    if !map.contains_key("properties") {
        map.insert("properties".to_owned(), Value::Object(Map::new()));
    }
    // A relay re-serializing an absent "required" sends null, which upstream
    // validators reject; the root object always carries an array.
    if !map.contains_key("required") {
        map.insert("required".to_owned(), Value::Array(Vec::new()));
    }
}

/// Object keys whose values are name-keyed schema maps (Go
/// `canonicalizeNamedSchemas`).
const NAMED_SCHEMA_KEYS: [&str; 5] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];

fn canonicalize_value(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for key in NAMED_SCHEMA_KEYS {
                if let Some(inner) = map.get_mut(key) {
                    if let Some(inner_map) = inner.as_object_mut() {
                        for schema in inner_map.values_mut() {
                            canonicalize_value(schema);
                        }
                    } else {
                        canonicalize_value(inner);
                    }
                }
            }
            if let Some(inner) = map.get_mut("dependentRequired")
                && let Some(inner_map) = inner.as_object_mut()
            {
                let bad_keys: Vec<String> = inner_map
                    .iter()
                    .filter(|(_, v)| !v.is_array())
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in bad_keys {
                    inner_map.remove(&key);
                }
                for arr in inner_map.values_mut() {
                    if let Some(items) = arr.as_array_mut() {
                        sort_schema_array(items);
                    }
                }
            }
            if let Some(req) = map.get_mut("required") {
                match req {
                    Value::Array(items) => sort_schema_array(items),
                    // OpenAPI-style {"required": true} property metadata is
                    // not JSON Schema's array form; dropping it keeps the
                    // whole tool list from being rejected with HTTP 400.
                    _ => {
                        map.remove("required");
                    }
                }
            }
            match map.get("dependentRequired") {
                Some(Value::Object(_)) => {}
                Some(_) => {
                    map.remove("dependentRequired");
                }
                None => {}
            }
            for (key, inner) in map.iter_mut() {
                if NAMED_SCHEMA_KEYS.contains(&key.as_str()) || key == "dependentRequired" {
                    continue;
                }
                canonicalize_value(inner);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                canonicalize_value(item);
            }
        }
        _ => {}
    }
}

fn sort_schema_array(arr: &mut [Value]) {
    arr.sort_by_key(|v| v.to_string());
}
