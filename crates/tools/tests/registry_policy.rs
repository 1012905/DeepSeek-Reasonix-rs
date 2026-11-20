//! Registry policy oracle (Go `registry_restrict_test.go`,
//! `replace_prefix_test.go`, `withdrawn_test.go`, `disabled_tools_test.go`,
//! `disabled_mcp_test.go`, `held_mcp_test.go`, `contract_lock_test.go`).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use reasonix_tools::abi::{CallContext, CapabilitySet, Tool, ToolResult};
use reasonix_tools::refusal::Refusal;
use reasonix_tools::registry::{
    CODE_MCP_TOOL_DISABLED, CODE_MCP_TOOL_HELD, CODE_TOOL_DISABLED, DisableConflict, HeldMcp,
    HeldMcpClass, McpBinding, Registry, mcp_binding_for,
};
use reasonix_tools::withdrawn::{WithdrawnTool, is_withdrawn};
use serde_json::{Value, json};

#[derive(Default)]
struct StubTool {
    name: String,
    schema: Option<Value>,
    server: String,
    raw: String,
}

impl StubTool {
    fn plain(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_owned(),
            ..Default::default()
        })
    }

    fn mcp(name: &str, server: &str, raw: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_owned(),
            server: server.to_owned(),
            raw: raw.to_owned(),
            ..Default::default()
        })
    }
}

impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "stub desc"
    }
    fn schema(&self) -> Value {
        self.schema
            .clone()
            .unwrap_or_else(|| json!({"type":"object"}))
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(String::new())
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> CapabilitySet {
        let mut caps = CapabilitySet::default();
        if !self.server.is_empty() {
            let meta: Arc<dyn reasonix_tools::registry::McpMetadata> =
                Arc::new(Meta(self.server.clone(), self.raw.clone()));
            caps = caps.with("mcp", meta);
        }
        caps
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct Meta(String, String);

impl reasonix_tools::abi::Capability for Meta {}

impl reasonix_tools::registry::McpMetadata for Meta {
    fn mcp_server_name(&self) -> String {
        self.0.clone()
    }
    fn mcp_raw_tool_name(&self) -> String {
        self.1.clone()
    }
}

/// A tool declaring the host feature that requires it (Go `requiredTool`).
struct RequiredTool {
    inner: Arc<StubTool>,
    by: String,
}

impl Tool for RequiredTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn schema(&self) -> Value {
        self.inner.schema()
    }
    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.inner.execute(ctx, args)
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> CapabilitySet {
        let req: Arc<dyn reasonix_tools::registry::HostRequired> =
            Arc::new(RequiredBy(self.by.clone()));
        CapabilitySet::default().with("host-required", req)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct RequiredBy(String);

impl reasonix_tools::abi::Capability for RequiredBy {}

impl reasonix_tools::registry::HostRequired for RequiredBy {
    fn host_required_by(&self) -> String {
        self.0.clone()
    }
}

/// A tool carrying a declared Reach (Go `reachingTool`).
struct ReachingTool {
    inner: Arc<StubTool>,
    reach: u8,
}

impl Tool for ReachingTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn schema(&self) -> Value {
        self.inner.schema()
    }
    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.inner.execute(ctx, args)
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> CapabilitySet {
        let reach: Arc<dyn reasonix_tools::registry::ReachDeclarer> =
            Arc::new(DeclaredReach(self.reach));
        CapabilitySet::default().with("reach", reach)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct DeclaredReach(u8);

impl reasonix_tools::abi::Capability for DeclaredReach {}

impl reasonix_tools::registry::ReachDeclarer for DeclaredReach {
    fn reach(&self) -> u8 {
        self.0
    }
}

// ---- registry_restrict_test.go ----

fn admit_reads(tool: &dyn Tool) -> bool {
    reasonix_tools::registry::reach_of(tool) == reasonix_tools::registry::REACH_LOCAL_READ
}

#[test]
fn restrict_drops_what_is_held_and_refuses_what_comes_later() {
    let r = Registry::new();
    r.add(Arc::new(ReachingTool {
        inner: StubTool::plain("read"),
        reach: reasonix_tools::registry::REACH_LOCAL_READ,
    }));
    r.add(StubTool::plain("unclassified"));
    r.add(Arc::new(ReachingTool {
        inner: StubTool::plain("control"),
        reach: reasonix_tools::registry::REACH_HOST_CONTROL,
    }));
    let before = r.schema_revision();

    r.restrict(Arc::new(admit_reads));
    assert_eq!(
        r.all_names(),
        vec!["read".to_owned()],
        "after Restrict the registry holds only read"
    );
    assert!(
        r.schema_revision() != before,
        "dropping tools left the schema revision unchanged"
    );

    r.add(StubTool::plain("late"));
    r.add(Arc::new(ReachingTool {
        inner: StubTool::plain("read2"),
        reach: reasonix_tools::registry::REACH_LOCAL_READ,
    }));
    r.add(Arc::new(ReachingTool {
        inner: StubTool::plain("read"),
        reach: reasonix_tools::registry::REACH_HOST_CONTROL,
    }));
    assert_eq!(
        r.all_names(),
        vec!["read".to_owned(), "read2".to_owned()],
        "registry holds read and read2 in order"
    );
    let held = r.get("read").expect("read kept");
    assert_eq!(
        reasonix_tools::registry::reach_of(held.as_ref()),
        reasonix_tools::registry::REACH_LOCAL_READ,
        "a rejected replacement displaced an admitted tool"
    );
    assert!(
        r.get("unclassified").is_none(),
        "a tool the ceiling refuses is still resolvable"
    );
}

#[test]
fn reach_of_is_unstated_until_declared() {
    assert_eq!(
        reasonix_tools::registry::reach_of(StubTool::plain("x").as_ref()),
        reasonix_tools::registry::REACH_UNSTATED
    );
}

// ---- replace_prefix_test.go ----

/// The window a remove-then-add pair leaves open is what ReplacePrefix
/// closes: concurrent readers never see fewer than the namespace's tools.
#[test]
fn replace_prefix_never_shows_an_empty_namespace() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let r = Arc::new(Registry::new());
    let first = vec![
        StubTool::plain("mcp__atlas__a") as Arc<dyn Tool>,
        StubTool::plain("mcp__atlas__b") as Arc<dyn Tool>,
    ];
    let second = vec![
        StubTool::plain("mcp__atlas__b") as Arc<dyn Tool>,
        StubTool::plain("mcp__atlas__c") as Arc<dyn Tool>,
    ];
    r.replace_prefix("mcp__atlas__", &first);

    let lowest = Arc::new(Mutex::new(usize::MAX));
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let r = r.clone();
            let lowest = lowest.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let seen = r
                        .schemas()
                        .iter()
                        .filter(|s| s.name.starts_with("mcp__atlas__"))
                        .count();
                    let mut l = lowest.lock().unwrap();
                    *l = (*l).min(seen);
                }
            })
        })
        .collect();
    for _ in 0..200 {
        r.replace_prefix("mcp__atlas__", &second);
        r.replace_prefix("mcp__atlas__", &first);
    }
    stop.store(true, Ordering::SeqCst);
    for reader in readers {
        reader.join().unwrap();
    }
    let l = *lowest.lock().unwrap();
    assert!(
        l >= 2,
        "a reader saw {l} tools under the prefix mid-swap, want never fewer than 2"
    );
}

#[test]
fn replace_prefix_leaves_a_suspended_prefix_off() {
    let r = Registry::new();
    r.suspend_prefix("mcp__atlas__");
    let tools = vec![StubTool::plain("mcp__atlas__a") as Arc<dyn Tool>];
    assert_eq!(r.replace_prefix("mcp__atlas__", &tools), 0);
    assert!(
        r.get("mcp__atlas__a").is_none(),
        "a swap registered a tool the session turned off"
    );
}

#[test]
fn replace_prefix_honors_the_admit_ceiling() {
    let r = Registry::new();
    r.restrict(Arc::new(|t: &dyn Tool| t.name() != "mcp__atlas__b"));
    let tools = vec![
        StubTool::plain("mcp__atlas__a") as Arc<dyn Tool>,
        StubTool::plain("mcp__atlas__b") as Arc<dyn Tool>,
    ];
    r.replace_prefix("mcp__atlas__", &tools);
    assert!(
        r.get("mcp__atlas__a").is_some(),
        "an admitted tool did not reach the registry"
    );
    assert!(
        r.get("mcp__atlas__b").is_none(),
        "a swap admitted a tool the ceiling rejects"
    );
}

#[test]
fn replace_prefix_ignores_tools_outside_the_prefix() {
    let r = Registry::new();
    r.add(StubTool::plain("mcp__ledger__post"));
    let tools = vec![StubTool::plain("mcp__ledger__other") as Arc<dyn Tool>];
    r.replace_prefix("mcp__atlas__", &tools);
    assert!(r.get("mcp__ledger__other").is_none());
    assert!(r.get("mcp__ledger__post").is_some());
}

// ---- withdrawn_test.go ----

fn stub_schema(tool: &dyn Tool) -> Value {
    tool.schema()
}

/// Withdrawing has to leave the provider-visible array byte-identical: the
/// point of keeping the slot is that the cached prefix does not move.
#[test]
fn withdraw_keeps_the_schema_and_fails_the_call() {
    let r = Registry::new();
    let live = StubTool::mcp("mcp__atlas__gone", "atlas", "gone");
    r.add(live.clone());
    let before = r.schemas();

    r.add(WithdrawnTool::wrap(
        live,
        "MCP tool \"gone\" not found on server \"atlas\"",
    ));
    let after = r.schemas();
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
    assert_eq!(
        before[0], after[0],
        "the schema moved when the tool was withdrawn"
    );

    let held = r.get("mcp__atlas__gone").expect("slot kept");
    assert!(
        is_withdrawn(&held),
        "the registry holds a live tool where a withdrawn slot was registered"
    );
    let err = held
        .execute(&CallContext::new(), &Value::Null)
        .expect_err("calling a withdrawn tool returned ok");
    assert!(
        err.to_string().contains("not found on server"),
        "calling a withdrawn tool returned {err}, want its reason"
    );
    assert!(
        held.read_only(),
        "a withdrawn tool reported a side effect it cannot have"
    );
    let _ = stub_schema;
}

/// A slot is not a capability: anything that routes by what a tool can still
/// do has to stop seeing this one, or a catalog keeps offering a dead call.
#[test]
fn withdraw_drops_the_mcp_binding() {
    let r = Registry::new();
    let live = StubTool::mcp("mcp__atlas__gone", "atlas", "gone");
    r.add(WithdrawnTool::wrap(live.clone(), "gone"));
    assert_eq!(
        r.mcp_bindings().len(),
        0,
        "a withdrawn tool still publishes MCP bindings, want none"
    );
    assert!(
        !is_withdrawn(&(live.clone() as Arc<dyn Tool>)),
        "a live tool reported itself withdrawn"
    );
}

#[test]
fn withdraw_restates_rather_than_nests() {
    let once = WithdrawnTool::wrap(StubTool::plain("mcp__atlas__gone"), "first");
    let twice = WithdrawnTool::wrap(once, "second");
    let err = twice
        .execute(&CallContext::new(), &Value::Null)
        .expect_err("reason");
    assert_eq!(
        err.to_string(),
        "second",
        "reason = {err}, want the restated one"
    );
    assert_eq!(
        twice.name(),
        "mcp__atlas__gone",
        "want the original tool's name"
    );
}

// ---- disabled_tools_test.go ----

#[test]
fn disable_tools_removes_from_every_surface_and_blocks_later_adds() {
    let r = Registry::new();
    r.add(StubTool::plain("web_fetch"));
    r.add(StubTool::plain("read_file"));

    let conflicts = r.disable_tools(&["web_fetch", "ghost"]);
    assert!(conflicts.is_empty(), "conflicts = {conflicts:?}, want none");
    assert!(
        r.get("web_fetch").is_none(),
        "a disabled tool is still registered"
    );
    let names = r
        .provider_schemas(&CallContext::new())
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["read_file".to_owned()]);
    if let reasonix_tools::ResolveCall::Exact(_) = r.resolve_call("web_fetch") {
        panic!("a call resolved to the disabled tool");
    }
    r.add(StubTool::plain("web_fetch"));
    assert!(
        r.get("web_fetch").is_none(),
        "a later registration path brought the disabled tool back"
    );
    let refusal = r.disabled_refusal("web_fetch").expect("refusal");
    assert_eq!(refusal.code, CODE_TOOL_DISABLED);
    assert!(refusal.remedy.is_some(), "want the setting that lifts it");
    assert!(
        r.disabled_refusal("read_file").is_none(),
        "an enabled tool answered as disabled"
    );
}

#[test]
fn disable_tools_keeps_what_the_host_needs_and_says_who() {
    let r = Registry::new();
    r.add(Arc::new(RequiredTool {
        inner: StubTool::plain("todo_write"),
        by: "task plans".to_owned(),
    }));
    let conflicts = r.disable_tools(&["todo_write"]);
    assert!(
        r.get("todo_write").is_some(),
        "a host-required tool was removed"
    );
    assert_eq!(
        conflicts,
        vec![DisableConflict {
            tool: "todo_write".to_owned(),
            by: "task plans".to_owned()
        }]
    );
    assert!(
        r.disabled_refusal("todo_write").is_none(),
        "a kept tool answers as disabled"
    );
}

#[test]
fn disable_tools_bumps_the_schema_revision_only_when_something_changed() {
    let r = Registry::new();
    r.add(StubTool::plain("web_fetch"));
    let before = r.schema_revision();
    r.disable_tools(&["ghost"]);
    assert_eq!(
        r.schema_revision(),
        before,
        "naming a tool nobody registered changed the schema revision"
    );
    r.disable_tools(&["web_fetch"]);
    assert!(
        r.schema_revision() != before,
        "removing a tool did not change the schema revision"
    );
}

// ---- disabled_mcp_test.go / held_mcp_test.go ----

fn disabled_test_binding() -> McpBinding {
    McpBinding {
        package: "demo".to_owned(),
        server: "mock".to_owned(),
        raw_name: "write".to_owned(),
        visible_name: "write".to_owned(),
        callable_name: "mcp__mock__write".to_owned(),
        capability_id: "mcp-tool:mock/write".to_owned(),
    }
}

#[test]
fn replace_disabled_mcp_drops_stale_aliases() {
    let reg = Registry::new();
    reg.replace_disabled_mcp("mock", &[disabled_test_binding()]);
    assert!(
        reg.disabled_mcp("mcp__mock__write") && reg.disabled_mcp("mcp-tool:mock/write"),
        "initial disabled policy was not recorded"
    );

    reg.replace_disabled_mcp("mock", &[]);
    assert!(
        !reg.disabled_mcp("mcp__mock__write") && !reg.disabled_mcp("mcp-tool:mock/write"),
        "replacing a server policy with an empty set left stale aliases"
    );
}

#[test]
fn disabled_mcp_does_not_claim_bare_aliases() {
    let reg = Registry::new();
    reg.mark_disabled_mcp(&disabled_test_binding());
    assert!(
        !reg.disabled_mcp("write"),
        "bare raw name must not claim an ambiguous disabled-tool match"
    );
    assert!(
        reg.disabled_mcp_refusal("write").is_none(),
        "bare raw name must not return a disabled-tool refusal"
    );
}

#[test]
fn copy_disabled_mcp_from_derived_registry() {
    let parent = Registry::new();
    parent.replace_disabled_mcp("mock", &[disabled_test_binding()]);
    let child = Registry::new();
    child.copy_disabled_mcp_from(&parent);

    assert!(
        child.disabled_mcp("mcp__mock__write") && child.disabled_mcp("mcp-tool:mock/write"),
        "derived registry did not inherit the disabled-tool policy"
    );
    parent.clear_disabled_mcp("mock");
    assert!(
        child.disabled_mcp("mcp__mock__write"),
        "derived registry must remain independent after copying"
    );
}

fn held_fixture() -> HeldMcp {
    HeldMcp {
        class: HeldMcpClass::Changed,
        binding: McpBinding {
            server: "srv".to_owned(),
            raw_name: "write".to_owned(),
            visible_name: "write".to_owned(),
            callable_name: "mcp__srv__write".to_owned(),
            ..Default::default()
        },
    }
}

#[test]
fn held_mcp_refusal_is_typed_and_server_qualified() {
    let reg = Registry::new();
    reg.replace_held_mcp("srv", &[held_fixture()]);
    let refusal = reg.mcp_refusal("mcp__srv__write").expect("held refusal");
    assert_eq!(refusal.code, CODE_MCP_TOOL_HELD);
    assert!(
        refusal.message.contains("\"srv\""),
        "message = {}",
        refusal.message
    );
    assert!(
        reg.mcp_refusal("write").is_none(),
        "a bare name is ambiguous across servers and must not be claimed"
    );
    reg.replace_held_mcp("srv", &[]);
    assert!(
        reg.mcp_refusal("mcp__srv__write").is_none(),
        "a released tool kept refusing"
    );
}

#[test]
fn disabled_policy_wins_over_held_and_children_inherit_both() {
    let parent = Registry::new();
    let b = held_fixture().binding;
    parent.replace_disabled_mcp("srv", std::slice::from_ref(&b));
    parent.replace_held_mcp("srv", &[held_fixture()]);
    let child = Registry::new();
    child.copy_disabled_mcp_from(&parent);
    assert_eq!(
        child.mcp_refusal("mcp__srv__write").unwrap().code,
        CODE_MCP_TOOL_DISABLED,
        "want the configuration refusal first"
    );
    parent.replace_disabled_mcp("srv", &[]);
    child.copy_disabled_mcp_from(&parent);
    let refusal = child
        .mcp_refusal("mcp__srv__write")
        .expect("child lost the held set");
    assert_eq!(refusal.code, CODE_MCP_TOOL_HELD);
}

// ---- contract_lock_test.go (AB-BA regression) ----

struct BlockingReadOnlyTool {
    name: String,
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Tool for BlockingReadOnlyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "blocking test tool"
    }
    fn schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok("ok".to_owned())
    }
    fn read_only(&self) -> bool {
        let _ = self.entered.send(());
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// ContractEntries must not hold any registry lock across a tool's own
/// callbacks: with a reader parked inside read_only, a registry writer (Add)
/// still completes.
#[test]
fn contract_entries_does_not_hold_registry_lock_across_tool_callbacks() {
    let reg = Arc::new(Registry::new());
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    reg.add(Arc::new(BlockingReadOnlyTool {
        name: "blocking_tool".to_owned(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    }));

    let reader = {
        let reg = reg.clone();
        thread::spawn(move || reg.contract_entries())
    };
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("ContractEntries never reached the read_only callback");

    let adder = {
        let reg = reg.clone();
        thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            reg.add(Arc::new(BlockingReadOnlyTool {
                name: "writer_tool".to_owned(),
                entered: tx,
                release: Mutex::new(rx),
            }));
        })
    };
    adder.join().expect(
        "deadlock: registry writer blocked while ContractEntries was parked inside read_only",
    );

    let _ = release_tx.send(());
    let entries = reader.join().expect("reader thread");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "blocking_tool");
    assert!(entries[0].read_only);
}

// ---- binding identity (Go mcp_binding.go) ----

#[test]
fn mcp_binding_for_reads_declared_identity() {
    let t = StubTool::mcp("mcp__srv__write", "srv", "write");
    let b = mcp_binding_for(t.as_ref()).expect("binding");
    assert_eq!(b.callable_name, "mcp__srv__write");
    assert_eq!(b.capability_id, "mcp-tool:srv/write");
    assert!(mcp_binding_for(StubTool::plain("bash").as_ref()).is_none());
    // A binding without a server or raw name is no binding at all.
    let broken = StubTool::mcp("mcp__x__y", "", "");
    assert!(mcp_binding_for(broken.as_ref()).is_none());
}

#[test]
fn refusal_helper_matches_disabled_policy() {
    let reg = Registry::new();
    reg.disable_tools(&["web_fetch"]);
    let refusal: Refusal = reg.call_refusal("web_fetch").expect("refusal");
    assert_eq!(refusal.code, CODE_TOOL_DISABLED);
    assert!(reg.call_refusal("read_file").is_none());
}
