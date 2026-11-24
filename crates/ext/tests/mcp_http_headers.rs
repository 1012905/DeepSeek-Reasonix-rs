//! Modern-era request header and `x-mcp-header` annotation tests (MCP-011).
//!
//! Covers header value encoding (plain vs base64 sentinel), scalar coercion
//! rules, annotation validity and the silent-tool exclusion rule, and the
//! modern headers derived from a body's `_meta`.

mod support;

use reasonix_ext::mcp::{
    HeaderAnnotationError, encode_header_value, header_scalar, modern_request_headers,
    param_headers_for_args, tool_param_headers,
};
use serde_json::{Value, json};

#[test]
fn encode_value_passes_visible_ascii_through() {
    assert_eq!(encode_header_value("hello world"), "hello world");
    assert_eq!(encode_header_value("a-b_c.1"), "a-b_c.1");
    assert_eq!(encode_header_value(""), "");
}

#[test]
fn encode_value_uses_base64_sentinel_for_non_ascii_or_edges() {
    let encoded = encode_header_value("héllo");
    assert!(encoded.starts_with("=?base64?"));
    assert!(encoded.ends_with("?="));
    assert!(encode_header_value(" abc").starts_with("=?base64?"));
    assert!(encode_header_value("abc ").starts_with("=?base64?"));
}

#[test]
fn scalar_coercion_accepts_string_bool_and_safe_integers() {
    assert_eq!(header_scalar(&json!("v")), Some("v".to_string()));
    assert_eq!(header_scalar(&json!(true)), Some("true".to_string()));
    assert_eq!(header_scalar(&json!(false)), Some("false".to_string()));
    assert_eq!(header_scalar(&json!(42)), Some("42".to_string()));
    assert_eq!(header_scalar(&json!(-7)), Some("-7".to_string()));
    assert_eq!(header_scalar(&json!(4.0)), Some("4".to_string()));
}

#[test]
fn scalar_coercion_rejects_non_integral_and_unsafe_numbers() {
    assert_eq!(header_scalar(&json!(4.5)), None);
    assert_eq!(header_scalar(&json!(9007199254740993u64)), None);
    assert_eq!(header_scalar(&json!(null)), None);
    assert_eq!(header_scalar(&json!([1])), None);
}

#[test]
fn valid_properties_path_annotations_are_extracted() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "x-mcp-header": "X-Name"},
            "age": {"type": "integer", "x-mcp-header": "X-Age"},
            "active": {"type": "boolean", "x-mcp-header": "X-Active"}
        }
    });
    let headers = tool_param_headers(&schema).expect("valid annotations");
    assert_eq!(headers.len(), 3);
    let by_name: std::collections::BTreeMap<&str, &[String]> = headers
        .iter()
        .map(|header| (header.name.as_str(), header.path.as_slice()))
        .collect();
    assert_eq!(by_name["X-Name"], vec!["name"]);
    assert_eq!(by_name["X-Age"], vec!["age"]);
    assert_eq!(by_name["X-Active"], vec!["active"]);
}

#[test]
fn nested_properties_path_is_recorded() {
    let schema = json!({
        "type": "object",
        "properties": {
            "outer": {
                "type": "object",
                "properties": {
                    "inner": {"type": "string", "x-mcp-header": "X-Inner"}
                }
            }
        }
    });
    let headers = tool_param_headers(&schema).expect("nested annotation");
    assert_eq!(headers[0].path, vec!["outer", "inner"]);
}

#[test]
fn annotation_off_the_properties_path_is_rejected() {
    let root = json!({"type": "object", "x-mcp-header": "X-Root"});
    assert!(tool_param_headers(&root).is_err());

    let inside_array = json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": {"type": "string", "x-mcp-header": "X-Tag"}
            }
        }
    });
    assert!(tool_param_headers(&inside_array).is_err());
}

#[test]
fn annotation_with_invalid_header_token_is_rejected() {
    for bad_name in ["X Bad!", "X@Y", ""] {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string", "x-mcp-header": bad_name}}
        });
        assert!(tool_param_headers(&schema).is_err(), "{bad_name}");
    }
}

#[test]
fn duplicate_annotation_names_are_rejected_case_insensitively() {
    let schema = json!({
        "type": "object",
        "properties": {
            "a": {"type": "string", "x-mcp-header": "X-Foo"},
            "b": {"type": "string", "x-mcp-header": "x-foo"}
        }
    });
    assert!(tool_param_headers(&schema).is_err());
}

#[test]
fn annotation_with_non_scalar_type_is_rejected() {
    for bad_type in ["number", "object", "array"] {
        let schema = json!({
            "type": "object",
            "properties": {
                "value": {"type": bad_type, "x-mcp-header": "X-Value"}
            }
        });
        assert!(tool_param_headers(&schema).is_err(), "{bad_type}");
    }
}

#[test]
fn invalid_annotation_drops_only_that_tool_from_the_catalog() {
    let good = json!({
        "type": "object",
        "properties": {"name": {"type": "string", "x-mcp-header": "X-Name"}}
    });
    let bad = json!({
        "type": "object",
        "properties": {"tags": {"type": "array", "items": {"x-mcp-header": "X-Tag"}}}
    });
    let catalog: Vec<(&str, &Value)> = vec![("good", &good), ("bad", &bad)];
    let kept: Vec<&str> = catalog
        .into_iter()
        .filter(|(_, schema)| tool_param_headers(schema).is_ok())
        .map(|(name, _)| name)
        .collect();
    assert_eq!(kept, vec!["good"]);
}

#[test]
fn modern_headers_are_derived_from_body_meta() {
    let params = json!({
        "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"},
        "name": "greet"
    });
    let headers = modern_request_headers("tools/call", Some(&params));
    assert!(headers.contains(&("MCP-Protocol-Version".into(), "2026-07-28".into())));
    assert!(headers.contains(&("Mcp-Method".into(), "tools/call".into())));
    assert!(headers.contains(&("Mcp-Name".into(), "greet".into())));
}

#[test]
fn modern_headers_are_absent_without_meta() {
    assert!(modern_request_headers("tools/list", Some(&json!({}))).is_empty());
    assert!(modern_request_headers("ping", None).is_empty());
}

#[test]
fn mcp_name_is_picked_by_method() {
    let meta = json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}});

    let mut tasks = json!({"taskId": "t-1"});
    tasks["_meta"] = meta["_meta"].clone();
    let headers = modern_request_headers("tasks/get", Some(&tasks));
    assert!(headers.contains(&("Mcp-Name".into(), "t-1".into())));

    let mut resource = json!({"uri": "file:///x"});
    resource["_meta"] = meta["_meta"].clone();
    let headers = modern_request_headers("resources/read", Some(&resource));
    assert!(headers.contains(&("Mcp-Name".into(), "file:///x".into())));

    let mut prompt = json!({"name": "p"});
    prompt["_meta"] = meta["_meta"].clone();
    let headers = modern_request_headers("prompts/get", Some(&prompt));
    assert!(headers.contains(&("Mcp-Name".into(), "p".into())));
}

#[test]
fn param_headers_read_annotated_arguments() {
    let schema = json!({
        "type": "object",
        "properties": {"name": {"type": "string", "x-mcp-header": "X-Name"}}
    });
    let headers = tool_param_headers(&schema).expect("annotation");

    let out = param_headers_for_args(&headers, &json!({"name": "sam"}));
    assert_eq!(out, vec![("Mcp-Param-X-Name".into(), "sam".into())]);

    let missing = param_headers_for_args(&headers, &json!({}));
    assert!(missing.is_empty());
}

#[test]
fn annotation_error_keeps_the_fixed_typed_code() {
    assert_eq!(
        HeaderAnnotationError.to_string(),
        "invalid x-mcp-header annotation"
    );
}
