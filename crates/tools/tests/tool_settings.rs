//! The `setting` tool at the tools boundary (Go `internal/tools/builtin/setting.go`,
//! docs/contracts/tools.md §1.18): the list/propose/open decision table, the
//! unavailable and invalid-value identities, the host routing, the 40-row
//! list cap, and the per-arguments decision barrier on `propose` only.

use std::sync::{Arc, Mutex};

use reasonix_tools::abi::{
    CallContext, is_decision_barrier_call, read_only_executes_host_mutation,
};
use reasonix_tools::builtin::settings::{
    CODE_SETTING_INVALID_VALUE, CODE_SETTING_UNAVAILABLE, SETTING_DESCRIPTION, SETTING_LIST_MAX,
    SETTING_TOOL_NAME, SettingInfo, SettingReceipt, SettingsHost, SettingsLink, list_settings,
    setting_tool,
};
use reasonix_tools::refusal::Refusal;
use serde_json::{Value, json};

struct FakeHost {
    listed: Mutex<Vec<String>>,
    proposed: Mutex<Vec<(String, Value, String)>>,
    opened: Mutex<Vec<String>>,
    rows: Vec<SettingInfo>,
    fail: Option<String>,
}

impl FakeHost {
    fn new(rows: Vec<SettingInfo>) -> Self {
        Self {
            listed: Mutex::new(Vec::new()),
            proposed: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            rows,
            fail: None,
        }
    }
}

impl SettingsHost for FakeHost {
    fn settings_list(&self, query: &str) -> Vec<SettingInfo> {
        self.listed
            .lock()
            .expect("listed lock")
            .push(query.to_owned());
        self.rows.clone()
    }

    fn propose_setting(
        &self,
        id: &str,
        value: &Value,
        reason: &str,
    ) -> Result<SettingReceipt, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(err) = &self.fail {
            return Err(Box::new(std::io::Error::other(err.clone())));
        }
        self.proposed.lock().expect("proposed lock").push((
            id.to_owned(),
            value.clone(),
            reason.to_owned(),
        ));
        Ok(SettingReceipt {
            id: id.to_owned(),
            from: json!(5),
            to: value.clone(),
            effect: "default".to_owned(),
            via: "click".to_owned(),
        })
    }

    fn open_setting(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(err) = &self.fail {
            return Err(Box::new(std::io::Error::other(err.clone())));
        }
        self.opened.lock().expect("opened lock").push(id.to_owned());
        Ok(())
    }
}

fn bind(fake: Arc<FakeHost>) -> Arc<SettingsLink> {
    let link = Arc::new(SettingsLink::default());
    link.bind(fake);
    link
}

fn as_prefixed<'a>(
    err: &'a (dyn std::error::Error + 'static),
) -> Option<&'a reasonix_tools::builtin::peer::PrefixedError> {
    err.downcast_ref::<reasonix_tools::builtin::peer::PrefixedError>()
}

#[test]
fn schema_description_and_flags_match_the_go_contract() {
    let link = Arc::new(SettingsLink::default());
    let tool = setting_tool(&link);
    assert_eq!(tool.name(), SETTING_TOOL_NAME);
    assert_eq!(tool.description(), SETTING_DESCRIPTION);
    assert!(
        tool.description()
            .contains("Find and change the user's own settings")
    );

    let s = tool.schema();
    assert_eq!(s["required"], json!(["op"]));
    assert_eq!(
        s["properties"]["op"]["enum"],
        json!(["list", "propose", "open"])
    );
    assert!(s["properties"]["query"].is_object());
    assert!(s["properties"]["id"].is_object());
    assert!(s["properties"]["value"].is_object());
    assert!(s["properties"]["reason"].is_object());

    assert!(tool.read_only());
    assert!(read_only_executes_host_mutation(&*tool));
}

#[test]
fn without_a_host_the_tool_is_unavailable_as_a_typed_refusal() {
    let link = Arc::new(SettingsLink::default());
    let err = setting_tool(&link)
        .execute(&CallContext::new(), &json!({"op": "list"}))
        .unwrap_err();
    let refusal = err.downcast_ref::<Refusal>().expect("typed refusal");
    assert_eq!(refusal.code, CODE_SETTING_UNAVAILABLE);
    assert_eq!(
        refusal.message,
        "Settings cannot be changed from this session."
    );
}

#[test]
fn list_routes_the_query_and_renders_the_rows_as_json() {
    let rows = vec![
        SettingInfo {
            id: "peer_dispatch".to_owned(),
            class: "switch".to_owned(),
            effect: "on".to_owned(),
            value: Some(json!(true)),
        },
        SettingInfo {
            id: "editor_font".to_owned(),
            class: "editor".to_owned(),
            effect: "Inter".to_owned(),
            value: None,
        },
    ];
    let fake = Arc::new(FakeHost::new(rows));
    let link = bind(fake.clone());
    let out = setting_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"op": "list", "query": "peers"}),
        )
        .expect("list answers");

    let listed = fake.listed.lock().expect("listed lock");
    assert_eq!(listed[0], "peers");

    let parsed: Value = serde_json::from_str(&out).expect("list renders JSON");
    assert_eq!(parsed["settings"][0]["id"], "peer_dispatch");
    assert_eq!(parsed["settings"][0]["value"], json!(true));
    assert_eq!(parsed["settings"][1]["value"], Value::Null);
    assert!(parsed.get("truncated").is_none(), "every row fits");
}

#[test]
fn the_list_is_capped_at_forty_and_marks_truncated() {
    let rows = (0..SETTING_LIST_MAX + 1)
        .map(|i| SettingInfo {
            id: format!("setting_{i}"),
            class: "switch".to_owned(),
            effect: "off".to_owned(),
            value: None,
        })
        .collect::<Vec<_>>();
    let out = list_settings(rows).expect("list renders");
    let parsed: Value = serde_json::from_str(&out).expect("list renders JSON");
    assert_eq!(
        parsed["settings"].as_array().unwrap().len(),
        SETTING_LIST_MAX
    );
    assert_eq!(parsed["truncated"], json!(true));
}

#[test]
fn propose_requires_an_id_and_a_value() {
    let link = bind(Arc::new(FakeHost::new(Vec::new())));
    for args in [
        json!({"op": "propose", "id": "", "value": json!(true)}),
        json!({"op": "propose", "id": "peer_dispatch"}),
    ] {
        let err = setting_tool(&link)
            .execute(&CallContext::new(), &args)
            .unwrap_err();
        let prefixed = as_prefixed(&*err).expect("typed invalid-value identity");
        assert_eq!(prefixed.code, CODE_SETTING_INVALID_VALUE);
        assert_eq!(prefixed.message, "op=propose needs an id and a value");
    }
}

#[test]
fn propose_routes_the_change_and_renders_the_receipt() {
    let fake = Arc::new(FakeHost::new(Vec::new()));
    let link = bind(fake.clone());
    let out = setting_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"op": "propose", "id": "peer_dispatch", "value": json!(false), "reason": "the other session asked"}),
        )
        .expect("propose accepted");

    let proposed = fake.proposed.lock().expect("proposed lock");
    assert_eq!(
        proposed[0],
        (
            "peer_dispatch".to_owned(),
            json!(false),
            "the other session asked".to_owned()
        )
    );

    let receipt: SettingReceipt = serde_json::from_str(&out).expect("receipt JSON");
    assert_eq!(receipt.id, "peer_dispatch");
    assert_eq!(receipt.from, json!(5));
    assert_eq!(receipt.to, json!(false));
    assert_eq!(receipt.effect, "default");
    assert_eq!(receipt.via, "click");
}

#[test]
fn propose_host_errors_pass_through_unchanged() {
    let fake = Arc::new(FakeHost {
        listed: Mutex::new(Vec::new()),
        proposed: Mutex::new(Vec::new()),
        opened: Mutex::new(Vec::new()),
        rows: Vec::new(),
        fail: Some("open_projects.declined: folder refused".to_owned()),
    });
    let link = bind(fake);
    let err = setting_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"op": "propose", "id": "peer_dispatch", "value": json!(true)}),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("open_projects.declined: folder refused")
    );
}

#[test]
fn open_points_the_user_at_the_setting() {
    let fake = Arc::new(FakeHost::new(Vec::new()));
    let link = bind(fake.clone());
    let out = setting_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"op": "open", "id": "editor_font"}),
        )
        .expect("open answered");

    let opened = fake.opened.lock().expect("opened lock");
    assert_eq!(opened[0], "editor_font");
    assert_eq!(
        out,
        "Pointed the user at editor_font. Only they can change it; carry on without it and say what it blocks."
    );
}

#[test]
fn an_unknown_op_is_rejected() {
    let link = bind(Arc::new(FakeHost::new(Vec::new())));
    let err = setting_tool(&link)
        .execute(&CallContext::new(), &json!({"op": "tweak"}))
        .unwrap_err();
    assert_eq!(err.to_string(), "setting needs op list, propose or open");
}

#[test]
fn only_propose_is_a_per_call_decision_barrier() {
    let link = Arc::new(SettingsLink::default());
    let tool = setting_tool(&link);
    assert!(is_decision_barrier_call(
        &*tool,
        &json!({"op": "propose", "id": "x", "value": json!(true)})
    ));
    for args in [
        json!({"op": "list", "query": "x"}),
        json!({"op": "open", "id": "x"}),
        json!({"op": "tweak"}),
        json!({}),
    ] {
        assert!(!is_decision_barrier_call(&*tool, &args), "args {args}");
    }
}

#[test]
fn an_argument_type_mismatch_is_rejected_with_the_envelope_error() {
    let link = Arc::new(SettingsLink::default());
    let err = setting_tool(&link)
        .execute(&CallContext::new(), &json!({"op": ["list"]}))
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("setting needs {\"op\": list|propose|open, ...}:")
    );
}

#[test]
fn the_registered_setting_tool_is_read_only_in_the_contract() {
    let registry = reasonix_tools::registry::Registry::new();
    let link = Arc::new(SettingsLink::default());
    registry.add(setting_tool(&link));
    let entries = registry.contract_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "setting");
    assert!(entries[0].read_only);
}
