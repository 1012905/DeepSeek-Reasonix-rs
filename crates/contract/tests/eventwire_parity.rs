//! Parity between the Rust wire types and `eventwire_manifest.json`, the Go
//! side's reflection-time view of the wire. Two directions per type: a
//! default instance must serialize to exactly the non-omitempty fields, and a
//! probe carrying every manifest field must deserialize — which is what
//! `deny_unknown_fields` on the wire types is there to make visible.

use std::collections::HashSet;

use reasonix_contract::agentgraph;
use reasonix_contract::delivery;
use reasonix_contract::eventwire;
use reasonix_contract::pricing;
use reasonix_contract::surfacespec;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn manifest() -> &'static serde_json::Value {
    use std::sync::OnceLock;
    static M: OnceLock<serde_json::Value> = OnceLock::new();
    M.get_or_init(|| serde_json::from_str(eventwire::MANIFEST).expect("manifest parses as JSON"))
}

fn entry(m: &serde_json::Value, key: &str) -> (Vec<String>, HashSet<String>) {
    let t = &m["types"][key];
    assert!(t.is_object(), "manifest is missing {key}");
    let fields: Vec<String> = t["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let omitted: HashSet<String> = t["omitted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    (fields, omitted)
}

fn check_serialize<T: Serialize + Default>(
    key: &str,
    fields: &[String],
    omitted: &HashSet<String>,
) {
    let required: HashSet<&String> = fields.iter().filter(|f| !omitted.contains(*f)).collect();
    let v = serde_json::to_value(T::default())
        .unwrap_or_else(|e| panic!("{key}: default instance failed to serialize: {e}"));
    let obj = v
        .as_object()
        .unwrap_or_else(|| panic!("{key}: default instance did not serialize to an object"));
    let actual: HashSet<&String> = obj.keys().collect();
    assert_eq!(
        actual, required,
        "{key}: a default instance must serialize to exactly the non-omitempty fields"
    );
}

fn check_deserialize<T: DeserializeOwned>(key: &str, fields: &[String]) {
    let mut probe = serde_json::Map::new();
    for f in fields {
        probe.insert(f.clone(), serde_json::Value::Null);
    }
    match serde_json::from_value::<T>(serde_json::Value::Object(probe)) {
        Ok(_) => {}
        Err(e) => {
            // serde has no structured classification for an unknown field, so
            // inside this one probe the message is the only signal it emits.
            let msg = e.to_string();
            assert!(
                !msg.contains("unknown field"),
                "{key}: manifest fields not accepted by the Rust side: {msg}"
            );
        }
    }
}

macro_rules! check {
    ($key:literal => $ty:path) => {{
        let (fields, omitted) = entry(manifest(), $key);
        check_serialize::<$ty>($key, &fields, &omitted);
        check_deserialize::<$ty>($key, &fields);
    }};
}

#[test]
fn kind_names_match_manifest() {
    let m = manifest();
    let manifest_kinds: HashSet<&str> = m["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let rust_kinds: HashSet<&str> = eventwire::KIND_NAMES.iter().copied().collect();
    assert_eq!(manifest_kinds, rust_kinds, "kind sets must match");
    assert_eq!(
        rust_kinds.len(),
        eventwire::KIND_NAMES.len(),
        "KIND_NAMES must not repeat a kind"
    );
}

#[test]
fn wire_types_match_manifest_both_ways() {
    check!("agentgraph.Delta" => agentgraph::Delta);
    check!("agentgraph.Edge" => agentgraph::Edge);
    check!("agentgraph.Graph" => agentgraph::Graph);
    check!("agentgraph.Node" => agentgraph::Node);
    check!("delivery.File" => delivery::File);
    check!("event.ContextComposition" => eventwire::event::ContextComposition);
    check!("event.ContextItem" => eventwire::event::ContextItem);
    check!("event.ContextPart" => eventwire::event::ContextPart);
    check!("eventwire.Approval" => eventwire::approval::Approval);
    check!("eventwire.Ask" => eventwire::ask::Ask);
    check!("eventwire.AskOption" => eventwire::ask::AskOption);
    check!("eventwire.AskOrigin" => eventwire::ask::AskOrigin);
    check!("eventwire.AskQuestion" => eventwire::ask::AskQuestion);
    check!("eventwire.Attachment" => eventwire::lease::Attachment);
    check!("eventwire.Bound" => eventwire::tool::Bound);
    check!("eventwire.CacheDiagnostics" => eventwire::usage::CacheDiagnostics);
    check!("eventwire.Compaction" => eventwire::event::Compaction);
    check!("eventwire.CompletionReceipt" => eventwire::receipt::CompletionReceipt);
    check!("eventwire.CompletionSummary" => eventwire::event::CompletionSummary);
    check!("eventwire.ContextMaintenance" => eventwire::event::ContextMaintenance);
    check!("eventwire.DecisionReceipt" => eventwire::event::DecisionReceipt);
    check!("eventwire.Event" => eventwire::event::Event);
    check!("eventwire.ExtensionActionRef" => eventwire::extension::ExtensionActionRef);
    check!("eventwire.ExtensionCard" => eventwire::extension::ExtensionCard);
    check!("eventwire.ExtensionForm" => eventwire::extension::ExtensionForm);
    check!("eventwire.ExtensionFormField" => eventwire::extension::ExtensionFormField);
    check!("eventwire.ExtensionKeyValue" => eventwire::extension::ExtensionKeyValue);
    check!("eventwire.ExtensionNotification" => eventwire::extension::ExtensionNotification);
    check!("eventwire.ExtensionPanel" => eventwire::extension::ExtensionPanel);
    check!("eventwire.ExtensionStatus" => eventwire::extension::ExtensionStatus);
    check!("eventwire.ExtensionSurface" => eventwire::extension::ExtensionSurface);
    check!("eventwire.ExtensionView" => eventwire::extension::ExtensionView);
    check!("eventwire.ExtensionViewNode" => eventwire::extension::ExtensionViewNode);
    check!("eventwire.Guardian" => eventwire::guardian::Guardian);
    check!("eventwire.MemoryCitation" => eventwire::event::MemoryCitation);
    check!("eventwire.Profile" => eventwire::tool::Profile);
    check!("eventwire.ProgressWatch" => eventwire::event::ProgressWatch);
    check!("eventwire.ReceiptChange" => eventwire::receipt::ReceiptChange);
    check!("eventwire.ReceiptGap" => eventwire::receipt::ReceiptGap);
    check!("eventwire.ReceiptVerification" => eventwire::receipt::ReceiptVerification);
    check!("eventwire.ShellExecution" => eventwire::tool::ShellExecution);
    check!("eventwire.StreamAttempt" => eventwire::event::StreamAttempt);
    check!("eventwire.Tool" => eventwire::tool::Tool);
    check!("eventwire.TodoProgress" => eventwire::event::TodoProgress);
    check!("eventwire.Usage" => eventwire::usage::Usage);
    check!("eventwire.Via" => eventwire::event::Via);
    check!("eventwire.VideoShown" => eventwire::tool::VideoShown);
    check!("eventwire.WorkspaceChanged" => eventwire::event::WorkspaceChanged);
    check!("eventwire.WorkspaceLease" => eventwire::lease::WorkspaceLease);
    check!("eventwire.WorkspacePathChange" => eventwire::event::WorkspacePathChange);
    check!("eventwire.WorkspaceRevision" => eventwire::event::WorkspaceRevision);
    check!("pricing.CostQuote" => pricing::CostQuote);
    check!("pricing.Money" => pricing::Money);
    check!("pricing.RateSnapshot" => pricing::RateSnapshot);
    check!("pricing.Valuation" => pricing::Valuation);
    check!("provider.ArgumentNormalization" => eventwire::tool::ArgumentNormalization);
    check!("surfacespec.Dataset" => surfacespec::Dataset);
    check!("surfacespec.Diff" => surfacespec::Diff);
    check!("surfacespec.Fact" => surfacespec::Fact);
    check!("surfacespec.Field" => surfacespec::Field);
    check!("surfacespec.Image" => surfacespec::Image);
    check!("surfacespec.Location" => surfacespec::Location);
    check!("surfacespec.Node" => surfacespec::Node);
    check!("surfacespec.Option" => surfacespec::SpecOption);
    check!("surfacespec.Series" => surfacespec::Series);
    check!("surfacespec.Spec" => surfacespec::Spec);
}
