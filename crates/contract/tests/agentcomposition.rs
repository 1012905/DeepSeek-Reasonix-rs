//! Ported from the reference agentcomposition Go tests:
//! composition_test.go, live_test.go, authority_test.go and
//! builtin_revisions_test.go (ledger PMSG-008..013), plus Rust-side
//! strictness probes for decode, digests and parameter discipline.

use std::collections::BTreeMap;

use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

use reasonix_contract::agentcomposition::{
    Binding, CompositionError, Definition, Module, Parameters, Resource, Selection, Version,
    builtins, decode, digest, parse_parameters, revision,
};
use reasonix_contract::gojson::marshal;

fn raw(parameters: &str) -> Option<Box<RawValue>> {
    Some(RawValue::from_string(parameters.to_owned()).unwrap())
}

fn module(id: &str, parameters: Option<&str>) -> Module {
    Module {
        id: id.to_owned(),
        parameters: parameters.and_then(raw),
        optional: false,
    }
}

fn select(members: Vec<Module>) -> Selection {
    Selection {
        mode: "select".to_owned(),
        members: Some(members),
    }
}

fn selection_ref<'a>(d: &'a Definition, group: &str) -> &'a Selection {
    match group {
        "tools" => &d.tools,
        "skills" => &d.skills,
        "mcp" => &d.mcp,
        "extensions" => &d.extensions,
        "hooks" => &d.hooks,
        "commands" => &d.commands,
        other => panic!("unknown group {other}"),
    }
}

fn selection_mut<'a>(d: &'a mut Definition, group: &str) -> &'a mut Selection {
    match group {
        "tools" => &mut d.tools,
        "skills" => &mut d.skills,
        "mcp" => &mut d.mcp,
        "extensions" => &mut d.extensions,
        "hooks" => &mut d.hooks,
        "commands" => &mut d.commands,
        other => panic!("unknown group {other}"),
    }
}

#[test]
fn definitions_validate_identity_and_selections() {
    let mut d = builtins()[0].clone();
    d.validate().unwrap();
    d.id = "user:../escape".to_owned();
    assert!(d.validate().unwrap_err().is_invalid());

    let mut d = builtins()[1].clone();
    d.skills = select(vec![module("one", None), module("one", None)]);
    let err = d.validate().unwrap_err();
    assert!(
        err.to_string()
            .contains("duplicate or empty module \"one\""),
        "{err}"
    );
}

#[test]
fn revision_canonicalizes_parameter_objects() {
    let mut d = builtins()[0].clone();
    d.mcp = select(vec![module("example", Some(r#"{"b":2,"a":1}"#))]);
    let first = revision(&d);
    d.mcp.members.as_mut().unwrap()[0].parameters = raw(r#"{"a":1,"b":2}"#);
    assert_eq!(revision(&d), first);
    d.name = "Changed".to_owned();
    assert_ne!(revision(&d), first);
}

#[test]
fn minimal_template_has_no_automation_or_delegation() {
    let bs = builtins();
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[0].name, "日常搭档");
    assert_eq!(bs[1].name, "轻装搭档");
    for d in &bs[1..] {
        for group in ["skills", "mcp", "extensions", "hooks", "commands"] {
            let sel = selection_ref(d, group);
            assert_eq!(sel.mode, "select", "{group}");
            assert!(sel.members.as_deref().unwrap_or(&[]).is_empty(), "{group}");
        }
        let tools = &d.tools;
        assert_eq!(tools.mode, "select");
        let members = tools.members.as_deref().unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].id, "bash");
        assert!(!members[0].optional);
        assert_eq!(members[1].id, "view_image");
        assert!(members[1].optional);
    }
}

#[test]
fn optional_modules_decode_and_validate() {
    let d = builtins()[0].clone();
    let data = marshal(&d).unwrap();
    for group in ["tools", "skills", "mcp", "extensions", "hooks", "commands"] {
        let mut doc: Map<String, Value> = serde_json::from_str(&data).unwrap();
        doc.insert(
            group.to_owned(),
            json!({"mode": "select", "members": [{"id": "example", "optional": true}]}),
        );
        let data2 = marshal(&doc).unwrap();
        let decoded = decode(data2.as_bytes());
        if group == "tools" {
            let d2 = decoded.unwrap();
            let sel = selection_ref(&d2, group);
            assert_eq!(sel.mode, "select");
            let members = sel.members.as_deref().unwrap();
            assert_eq!(members[0].id, "example");
            assert!(members[0].optional);
        } else {
            let err = decoded.unwrap_err();
            assert!(err.is_invalid(), "{group}: {err}");
            let tolerant: Definition = serde_json::from_str(&data2).unwrap();
            assert!(tolerant.validate().is_err(), "{group}");
        }
    }
}

#[test]
fn optional_tools_binding_round_trip() {
    let d = builtins()[1].clone();
    let mut b = Binding {
        resolved: d.clone(),
        version: Version {
            definition: d.clone(),
            revision: revision(&d),
        },
        ..Default::default()
    };
    b.seal();
    b.validate().unwrap();

    let mut b2 = b.clone();
    for def in [&b2.resolved, &b2.version.definition] {
        let members = def.tools.members.as_deref().unwrap();
        assert_eq!(members[0].id, "bash");
        assert!(!members[0].optional);
        assert_eq!(members[1].id, "view_image");
        assert!(members[1].optional);
    }
    b2.resolved.tools.members.as_deref_mut().unwrap()[1].optional = false;
    let err = b2.validate().unwrap_err();
    assert!(err.to_string().contains("binding digest"), "{err}");
}

#[test]
fn absent_optional_preserves_revision_and_binding_digest() {
    let mut d = builtins()[0].clone();
    d.tools = select(vec![module("bash", None)]);
    let data = marshal(&d).unwrap();

    let mut legacy: Map<String, Value> = serde_json::from_str(&data).unwrap();
    legacy.insert(
        "tools".to_owned(),
        json!({"mode": "select", "members": [{"id": "bash"}]}),
    );
    assert_eq!(revision(&d), digest(&legacy));

    let decoded = decode(data.as_bytes()).unwrap();
    assert!(!decoded.tools.members.as_deref().unwrap()[0].optional);

    let mut binding = Binding {
        resolved: decoded.clone(),
        version: Version {
            definition: decoded.clone(),
            revision: revision(&decoded),
        },
        ..Default::default()
    };
    binding.seal();

    let legacy_binding_bytes = marshal(&binding).unwrap();
    let mut legacy_binding: Map<String, Value> =
        serde_json::from_str(&legacy_binding_bytes).unwrap();
    legacy_binding.insert("resolved".to_owned(), Value::Object(legacy.clone()));
    legacy_binding.insert(
        "version".to_owned(),
        json!({"definition": legacy, "revision": revision(&decoded)}),
    );
    legacy_binding.insert("digest".to_owned(), json!(""));
    assert_eq!(digest(&legacy_binding), binding.digest);
}

#[test]
fn binding_resource_formats() {
    let d = builtins()[0].clone();
    let cases: &[(&str, bool)] = &[
        ("", true),
        (r#","formatVersion":2,"resourceMode":"live""#, true),
        (r#","formatVersion":99,"resourceMode":"live""#, false),
        (r#","formatVersion":2"#, false),
        (r#","resourceMode":"live""#, false),
        (r#","formatVersion":2,"resourceMode":"other""#, false),
    ];
    for &(extra, valid) in cases {
        let b = Binding {
            version: Version {
                definition: d.clone(),
                revision: revision(&d),
            },
            resolved: d.clone(),
            resources: Some(Vec::new()),
            ..Default::default()
        };
        let mut text = marshal(&b).unwrap();
        text.pop();
        text.push_str(extra);
        text.push('}');
        let mut parsed: Binding = serde_json::from_str(&text).unwrap();
        parsed.seal();
        assert_eq!(parsed.validate().is_ok(), valid, "extra {extra}");
    }
}

#[test]
fn definition_authority_by_source_and_module() {
    let cases: &[(&str, &str, bool)] = &[
        ("mcp", "{}", true),
        (
            "mcp",
            r#"{"callTimeoutSeconds":5,"startupTimeoutSeconds":10}"#,
            true,
        ),
        ("mcp", r#"{"args":["other-package"]}"#, false),
        ("mcp", r#"{"args":[]}"#, false),
        ("mcp", r#"{"url":"https://other.invalid/mcp"}"#, false),
        ("mcp", r#"{"env":{"PATH":"${OTHER_PATH}"}}"#, false),
        ("mcp", r#"{"env":{}}"#, false),
        (
            "mcp",
            r#"{"headers":{"Authorization":"${OTHER_TOKEN}"}}"#,
            false,
        ),
        ("mcp", r#"{"headers":{}}"#, false),
        ("extensions", r#"{"timeoutMillis":5000}"#, true),
        ("extensions", r#"{"args":["other-script"]}"#, false),
        (
            "extensions",
            r#"{"env":{"NODE_OPTIONS":"${OTHER_OPTIONS}"}}"#,
            false,
        ),
    ];
    for scope in ["builtin", "user", "project"] {
        for &(group, parameters, ok) in cases {
            let mut d = builtins()[0].clone();
            d.id = format!("{scope}:test");
            *selection_mut(&mut d, group) = select(vec![module("authorized", Some(parameters))]);
            let validated = d.validate_for_execution();
            if ok || scope != "project" {
                validated.unwrap();
            } else {
                let err = validated.unwrap_err();
                assert!(err.is_invalid(), "{scope} {group}: {err}");
            }
            let data = marshal(&d).unwrap();
            let decoded = decode(data.as_bytes());
            if !ok && scope == "project" {
                let err = decoded.unwrap_err();
                assert!(err.is_invalid(), "{scope} {group}: {err}");
                assert!(
                    err.to_string()
                        .contains("project module authorized may override only timeouts"),
                    "{err}"
                );
            } else {
                decoded.unwrap();
            }
        }
    }
}

#[test]
fn builtin_revisions_match_kernel() {
    let revisions: BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../src/agentcomposition/builtin-revisions.json"
    ))
    .unwrap();
    assert_eq!(
        revisions["builtin:general"],
        "90d6a645648f1767371fedc2c01a487428dc847417c77bddd82a365520a65dfb"
    );
    assert_eq!(
        revisions["builtin:minimal"],
        "b25962a98543abdb021332dcb2216d66a59ad90a3395c3d2f7d9cc30186a7a5b"
    );
    for d in builtins() {
        assert_eq!(revisions[&d.id], revision(&d), "revision drift {}", d.id);
    }
}

#[test]
fn decode_rejects_unknown_fields_and_arrays() {
    let d = builtins()[0].clone();
    let data = marshal(&d).unwrap();
    let mut doc: Map<String, Value> = serde_json::from_str(&data).unwrap();
    doc.insert("unexpected".to_owned(), json!(1));
    let bad = marshal(&doc).unwrap();
    let err = decode(bad.as_bytes()).unwrap_err();
    assert!(err.is_invalid());
    assert!(err.to_string().contains("unknown field"), "{err}");

    let err = decode(b"[1,2]").unwrap_err();
    assert!(err.is_invalid());
}

#[test]
fn decode_rejects_nested_unknown_fields() {
    let d = builtins()[0].clone();
    let data = marshal(&d).unwrap();
    let mut doc: Map<String, Value> = serde_json::from_str(&data).unwrap();
    doc.get_mut("persona")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("extra".to_owned(), json!(1));
    let bad = marshal(&doc).unwrap();
    let err = decode(bad.as_bytes()).unwrap_err();
    assert!(err.is_invalid());
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn decode_rejects_trailing_data() {
    let err = decode(b"{} {}").unwrap_err();
    assert_eq!(err.to_string(), "invalid agent composition: trailing data");
}

#[test]
fn decode_round_trips_builtins() {
    for d in builtins() {
        let text = marshal(&d).unwrap();
        let round = marshal(&decode(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(text, round, "round trip changed {}", d.id);
    }
}

#[test]
fn retained_resource_limit_is_enforced() {
    let d = builtins()[0].clone();
    let mut resources = Vec::new();
    for i in 0..20001 {
        resources.push(Resource {
            id: format!("r{i}"),
            kind: "kind".to_owned(),
            data: raw("1"),
        });
    }
    let mut b = Binding {
        version: Version {
            definition: d.clone(),
            revision: revision(&d),
        },
        resolved: d,
        resources: Some(resources),
        ..Default::default()
    };
    b.seal();
    let err = b.validate().unwrap_err();
    assert!(err.to_string().contains("retained resource limit"), "{err}");
}

#[test]
fn resource_without_data_is_invalid() {
    let d = builtins()[0].clone();
    let mut b = Binding {
        version: Version {
            definition: d.clone(),
            revision: revision(&d),
        },
        resolved: d,
        resources: Some(vec![Resource {
            id: "r".to_owned(),
            kind: "k".to_owned(),
            data: None,
        }]),
        ..Default::default()
    };
    b.seal();
    assert_eq!(b.validate().unwrap_err(), CompositionError::invalid());
}

#[test]
fn selection_allows() {
    let inherit = Selection {
        mode: "inherit".to_owned(),
        members: None,
    };
    assert!(inherit.allows("anything"));
    let sel = select(vec![module("bash", None)]);
    assert!(sel.allows("bash"));
    assert!(!sel.allows("view_image"));
    assert!(!Selection::default().allows("bash"));
}

#[test]
fn seal_excludes_started() {
    let d = builtins()[0].clone();
    let version = Version {
        definition: d.clone(),
        revision: revision(&d),
    };
    let mut a = Binding {
        resolved: d.clone(),
        version: version.clone(),
        started: true,
        ..Default::default()
    };
    let mut b = Binding {
        resolved: d,
        version,
        started: false,
        ..Default::default()
    };
    a.seal();
    b.seal();
    assert_eq!(a.digest, b.digest);
}

#[test]
fn parse_parameters_enforces_reference_discipline() {
    assert_eq!(parse_parameters(None).unwrap(), Parameters::default());
    assert_eq!(
        parse_parameters(raw("{}").as_deref()).unwrap(),
        Parameters::default()
    );
    assert_eq!(
        parse_parameters(raw("null").as_deref()).unwrap(),
        Parameters::default()
    );

    let err = parse_parameters(raw(r#"{"env":{"PATH":"/usr/bin"}}"#).as_deref()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid agent composition: environment and header values must be ${NAME} references"
    );
    let err = parse_parameters(raw(r#"{"headers":{"Authorization":"Bearer abc"}}"#).as_deref())
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid agent composition: environment and header values must be ${NAME} references"
    );

    parse_parameters(
        raw(r#"{"env":{"PATH":"${OTHER_PATH}"},"headers":{"X-Trace":"${TOKEN}"}}"#).as_deref(),
    )
    .unwrap();
    parse_parameters(raw(r#"{"args":["KEY=${TOKEN}"],"url":"${ENDPOINT}"}"#).as_deref()).unwrap();

    let err = parse_parameters(raw(r#"{"unknown":1}"#).as_deref()).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("invalid agent composition: module parameters:"),
        "{err}"
    );
    let err = parse_parameters(raw(r#"{"timeoutMillis":-1}"#).as_deref()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid agent composition: negative timeout"
    );

    let err = parse_parameters(raw(r#"{"args":["-H","Authorization: Bearer xyz"]}"#).as_deref())
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid agent composition: credentials require references"
    );
    let err = parse_parameters(raw(r#"{"url":"https://u:p@h/m"}"#).as_deref()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid agent composition: endpoint credentials require references"
    );
}

#[test]
fn module_parameters_group_rules() {
    let mut d = builtins()[0].clone();
    d.tools = select(vec![module("bash", Some(r#"{"a":1}"#))]);
    assert_eq!(
        d.validate().unwrap_err().to_string(),
        "invalid agent composition: parameters unsupported for module bash"
    );

    let mut d = builtins()[0].clone();
    d.tools = select(vec![module("bash", Some("{}"))]);
    d.validate().unwrap();

    let mut d = builtins()[0].clone();
    d.tools = select(vec![module("bash", Some("null"))]);
    assert_eq!(
        d.validate().unwrap_err().to_string(),
        "invalid agent composition: module parameters"
    );

    let mut d = builtins()[0].clone();
    d.mcp = select(vec![module("example", Some("null"))]);
    assert_eq!(
        d.validate().unwrap_err().to_string(),
        "invalid agent composition: module parameters"
    );

    let mut d = builtins()[0].clone();
    d.mcp = select(vec![module("example", None)]);
    d.validate().unwrap();
}
