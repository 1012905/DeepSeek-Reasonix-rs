//! Schema canonicalization oracle (Go `schema_canonicalize_test.go`).

use reasonix_tools::canonicalize_schema;
use serde_json::json;

#[test]
fn canonicalize_drops_non_array_required() {
    let raw = json!({
        "type":"object",
        "properties":{
            "query":{"type":"string","required":true},
            "nested":{"type":"object","required":false,"properties":{"x":{"type":"string"}}}
        },
        "required":["query","nested"]
    });
    let got = canonicalize_schema(&raw);
    assert_eq!(
        got,
        json!({
            "properties":{
                "nested":{"properties":{"x":{"type":"string"}},"type":"object"},
                "query":{"type":"string"}
            },
            "required":["nested","query"],
            "type":"object"
        })
    );
}

#[test]
fn canonicalize_adds_empty_properties_for_no_argument_object() {
    let want = json!({"properties":{},"required":[],"type":"object"});
    assert_eq!(canonicalize_schema(&serde_json::Value::Null), want);
    assert_eq!(canonicalize_schema(&json!({"type":"object"})), want);
}

#[test]
fn canonicalize_adds_missing_root_type() {
    for (raw, want) in [
        (
            json!({}),
            json!({"properties":{},"required":[],"type":"object"}),
        ),
        (
            json!({"properties":{"q":{"type":"string"}}}),
            json!({"properties":{"q":{"type":"string"}},"required":[],"type":"object"}),
        ),
        // Explicit non-object root types are preserved verbatim; validation
        // quarantines them instead of rewriting declared semantics.
        (json!({"type":"string"}), json!({"type":"string"})),
        (
            json!({"type":["object","null"]}),
            json!({"type":["object","null"]}),
        ),
    ] {
        assert_eq!(canonicalize_schema(&raw), want, "raw = {raw}");
    }
}

#[test]
fn canonicalize_root_object_always_has_required_array() {
    for (raw, want) in [
        (
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            json!({"additionalProperties":false,"properties":{},"required":[],"type":"object"}),
        ),
        (
            json!({"type":"object","required":null}),
            json!({"properties":{},"required":[],"type":"object"}),
        ),
        (
            json!({"type":"object","required":true}),
            json!({"properties":{},"required":[],"type":"object"}),
        ),
        (
            json!({"type":"object","properties":{"a":{"type":"object"}},"required":["a"]}),
            json!({"properties":{"a":{"type":"object"}},"required":["a"],"type":"object"}),
        ),
    ] {
        assert_eq!(canonicalize_schema(&raw), want, "raw = {raw}");
    }
}

#[test]
fn canonicalize_preserves_property_named_required() {
    let raw = json!({
        "type":"object",
        "properties":{
            "required":{"type":"boolean","description":"Whether the item is required"},
            "normal":{"type":"string"}
        },
        "required":["required"]
    });
    assert_eq!(
        canonicalize_schema(&raw),
        json!({
            "properties":{
                "normal":{"type":"string"},
                "required":{"description":"Whether the item is required","type":"boolean"}
            },
            "required":["required"],
            "type":"object"
        })
    );
}

#[test]
fn canonicalize_dependent_required() {
    let raw = json!({
        "type":"object",
        "dependentRequired":{
            "cc":["billing_address","name"],
            "bad":true
        }
    });
    assert_eq!(
        canonicalize_schema(&raw),
        json!({
            "dependentRequired":{"cc":["billing_address","name"]},
            "properties":{},
            "required":[],
            "type":"object"
        })
    );
}

#[test]
fn canonicalize_preserves_dependent_required_property_name() {
    let raw = json!({
        "type":"object",
        "properties":{
            "dependentRequired":{"type":"string"},
            "x":{"type":"string"}
        },
        "dependentRequired":{"dependentRequired":["x"]}
    });
    assert_eq!(
        canonicalize_schema(&raw),
        json!({
            "dependentRequired":{"dependentRequired":["x"]},
            "properties":{"dependentRequired":{"type":"string"},"x":{"type":"string"}},
            "required":[],
            "type":"object"
        })
    );
}

#[test]
fn canonicalize_preserves_legacy_tuple_items() {
    let raw = json!({"type":"object","properties":{"pair":{"type":"array","items":[{"type":"string"},{"type":"number"}],"additionalItems":false}}});
    assert_eq!(
        canonicalize_schema(&raw),
        json!({
            "properties":{"pair":{"additionalItems":false,"items":[{"type":"string"},{"type":"number"}],"type":"array"}},
            "required":[],
            "type":"object"
        })
    );
}
