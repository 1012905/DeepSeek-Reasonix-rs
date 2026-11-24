//! The per-run registry (tools.md §1.2, §1.3, §1.9-§1.11): enabled built-ins
//! plus runtime-added tools, with provider-surface projection, admission
//! ceilings, and typed policy refusals.
//!
//! Lock discipline mirrors the Go source: per-tool methods
//! (`description`/`read_only`/capabilities) run outside the registry lock —
//! a lazy MCP placeholder's read-only may take a spawn mutex held by a swap
//! that needs this registry's write lock, so holding the lock across it is
//! an AB-BA deadlock. Caches are captured under the lock and callbacks are
//! invoked after it is released.

pub mod entries;
pub use entries::*;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::abi::{CapabilitySet, Tool, ToolSchema};
use crate::refusal::Refusal;
use crate::schema::canonicalize_schema;

/// `tool.disabled_by_config` — the user's `[tools].disabled` removed the
/// tool by name.
pub const CODE_TOOL_DISABLED: &str = "tool.disabled_by_config";

/// Feature ids that can require a tool (tools.md §1.9). A client shows its
/// own wording for the id; [`feature_label`] is the English one.
pub const FEATURE_GOAL_MODE: &str = "goal_mode";
pub const FEATURE_DEFERRED_MCP: &str = "deferred_mcp_tools";
pub const FEATURE_PLAN_STEPS: &str = "plan_steps";
pub const FEATURE_TURN_EXIT: &str = "turn_exit";

/// English phrase for a feature id, or the id itself.
pub fn feature_label(id: &str) -> &str {
    match id {
        FEATURE_GOAL_MODE => "goal mode",
        FEATURE_DEFERRED_MCP => "loading deferred MCP tools",
        FEATURE_PLAN_STEPS => "plan step tracking",
        FEATURE_TURN_EXIT => "ending a turn the host left open",
        _ => id,
    }
}

/// One tool the user asked to remove and the host kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableConflict {
    pub tool: String,
    pub by: String,
}

/// The provider-visible contract for a tool schema snapshot (tools.md §1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEntry {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub schema: Value,
    /// Host identity; excluded from serialized contracts.
    pub mcp: Option<McpBinding>,
    /// Extra capability ids this tool answers to; omitted when empty so a
    /// tool without aliases keeps its contract bytes.
    pub capability_aliases: Vec<String>,
}

/// Admission ceiling over what this registry may ever hold.
type AdmitFn = Arc<dyn Fn(&dyn Tool) -> bool + Send + Sync>;
/// Execution authority applied to existing and future MCP adapters.
type BindMcpFn = Arc<dyn Fn(Arc<dyn Tool>) -> Arc<dyn Tool> + Send + Sync>;

#[derive(Default)]
struct RegistryState {
    tools: HashMap<String, Arc<dyn Tool>>,
    order: Vec<String>,
    canon: HashMap<String, Value>,
    suspended: BTreeSet<String>,
    disabled_mcp: BTreeSet<String>,
    disabled_tools: BTreeSet<String>,
    disabled_mcp_by_server: BTreeMap<String, BTreeSet<String>>,
    held_mcp: BTreeMap<String, BTreeMap<String, HeldMcp>>,
    provider_visible: Option<BTreeSet<String>>,
    admit: Option<AdmitFn>,
    bind_mcp: Option<BindMcpFn>,
    schema_rev: u64,
}

/// A per-run set of tools: enabled built-ins plus plugin tools.
#[derive(Default)]
pub struct Registry {
    state: Mutex<RegistryState>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryState> {
        // A panic while holding the lock leaves the registry intact; the
        // gate must still answer (the safety gate recovers the same way).
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Restricts the provider-visible schema surface to the given names while
    /// keeping all registered tools executable. `None` clears the
    /// restriction; names are trimmed and empties dropped.
    pub fn set_provider_visible_tools(&self, names: Option<&[&str]>) {
        let mut st = self.lock();
        match names {
            None => {
                if st.provider_visible.take().is_some() {
                    st.schema_rev += 1;
                }
            }
            Some(names) => {
                let visible: BTreeSet<String> = names
                    .iter()
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect();
                let changed = st.provider_visible.as_ref() != Some(&visible);
                st.provider_visible = Some(visible);
                if changed {
                    st.schema_rev += 1;
                }
            }
        }
    }

    /// Whether `name` is currently provider-visible.
    pub fn provider_visible(&self, name: &str) -> bool {
        let st = self.lock();
        match &st.provider_visible {
            None => true,
            Some(set) => set.contains(name.trim()),
        }
    }

    /// Whether the name belongs to an explicit provider-visible allowlist.
    /// False when no allowlist is set: a registry that shows whatever it
    /// holds promises nothing about which names a provider sees.
    pub fn provider_surface_pinned(&self, name: &str) -> bool {
        let st = self.lock();
        st.provider_visible
            .as_ref()
            .is_some_and(|set| set.contains(name.trim()))
    }

    /// Adds (or replaces) a tool, preserving first-seen order. The schema is
    /// canonicalized once here; it never changes after registration, so
    /// every-turn schema exports reuse the cached bytes.
    pub fn add(&self, tool: Arc<dyn Tool>) {
        let admit = {
            let st = self.lock();
            st.admit.clone()
        };
        if admit.is_some_and(|admit| !admit(tool.as_ref())) {
            return;
        }
        let tool = {
            let st = self.lock();
            match &st.bind_mcp {
                Some(bind) if mcp_binding_for(tool.as_ref()).is_some() => bind(tool),
                _ => tool,
            }
        };
        let name = tool.name().to_owned();
        let canon = canonicalize_schema(&tool.schema());
        let mut st = self.lock();
        if self.suspended(&st, &name) || self.refuses(&st, tool.as_ref()) {
            return;
        }
        if !st.tools.contains_key(&name) {
            st.order.push(name.clone());
        }
        st.tools.insert(name.clone(), tool);
        st.canon.insert(name, canon);
        st.schema_rev += 1;
    }

    fn suspended(&self, st: &RegistryState, name: &str) -> bool {
        st.suspended.iter().any(|p| name.starts_with(p.as_str()))
    }

    /// Whether the user's removal list turns `tool` away. A tool the host
    /// requires is never turned away.
    fn refuses(&self, st: &RegistryState, tool: &dyn Tool) -> bool {
        if !st.disabled_tools.contains(tool.name()) {
            return false;
        }
        tool.capabilities().get::<dyn HostRequired>().is_none()
    }

    /// Unregisters every tool whose name starts with `prefix` (an MCP
    /// server's namespace on disconnect) and returns the count removed.
    pub fn remove_prefix(&self, prefix: &str) -> usize {
        let mut st = self.lock();
        remove_prefix_locked(&mut st, prefix)
    }

    /// Unregisters matching tools and refuses future adds for that prefix
    /// until [`Registry::resume_prefix`] — the per-session off switch that
    /// stops an in-flight handshake swapping tools back in.
    pub fn suspend_prefix(&self, prefix: &str) -> usize {
        let mut st = self.lock();
        st.suspended.insert(prefix.to_owned());
        remove_prefix_locked(&mut st, prefix)
    }

    /// Allows future adds for a previously suspended prefix.
    pub fn resume_prefix(&self, prefix: &str) {
        self.lock().suspended.remove(prefix);
    }

    /// Looks up a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.lock().tools.get(name).cloned()
    }

    /// Live MCP capability bindings in canonical-name order.
    pub fn mcp_bindings(&self) -> Vec<McpBinding> {
        let st = self.lock();
        let mut out: Vec<McpBinding> = st
            .order
            .iter()
            .filter_map(|name| st.tools.get(name).and_then(|t| mcp_binding_for(t.as_ref())))
            .collect();
        out.sort_by(|a, b| a.callable_name.cmp(&b.callable_name));
        out
    }

    /// Resolves an exact provider-visible name or a unique portable MCP
    /// reference. Exact names always win. Ambiguous aliases return their
    /// canonical candidates and are never executed.
    pub fn resolve_call(&self, name: &str) -> ResolveCall {
        let st = self.lock();
        if let Some(t) = st.tools.get(name) {
            return ResolveCall::Exact(t.clone());
        }
        let mut matches: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        for (canonical, t) in &st.tools {
            if let Some(b) = mcp_binding_for(t.as_ref())
                && mcp_binding_aliases(&b).iter().any(|a| a == name)
            {
                matches.insert(canonical.clone(), t.clone());
            }
        }
        match matches.len() {
            0 => ResolveCall::Unknown,
            1 => ResolveCall::Exact(matches.into_values().next().expect("one match")),
            _ => ResolveCall::Ambiguous(matches.into_keys().collect()),
        }
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.lock().order.len()
    }

    /// Whether the registry holds no tools.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The revision that changes whenever the provider-visible tool set
    /// changes.
    pub fn schema_revision(&self) -> u64 {
        self.lock().schema_rev
    }

    /// Registered tool names in insertion order.
    pub fn names(&self) -> Vec<String> {
        self.lock().order.clone()
    }

    /// Every registered tool name, including tools hidden from the
    /// provider-visible schema (capability catalogs, diagnostics).
    pub fn all_names(&self) -> Vec<String> {
        self.names()
    }

    /// Tool definitions in stable name order for the provider. When a
    /// provider-visible allowlist is set, only those tools appear.
    pub fn schemas(&self) -> Vec<ToolSchema> {
        let st = self.lock();
        let mut names: Vec<&String> = st
            .order
            .iter()
            .filter(|name| is_provider_visible(&st, name))
            .collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| {
                let t = st.tools.get(name)?;
                Some(ToolSchema {
                    name: t.name().to_owned(),
                    description: t.description().to_owned(),
                    parameters: st.canon.get(name).cloned().unwrap_or(Value::Null),
                })
            })
            .collect()
    }

    /// The surface one turn carries: stable tools first, contextual after,
    /// each group sorted — an absence is a byte prefix of a presence, so the
    /// provider cache keeps every token before the boundary. Per-tool
    /// `provider_visible` calls run outside the lock (see module docs).
    pub fn provider_schemas(&self, ctx: &crate::abi::CallContext) -> Vec<ToolSchema> {
        let snapshot = self.visible_snapshot();
        let mut stable = Vec::new();
        let mut dynamic = Vec::new();
        for (_, t) in snapshot {
            let entry = (t.name().to_owned(), t.description().to_owned());
            let caps = t.capabilities();
            match caps.get::<dyn crate::abi::ContextualTool>() {
                Some(ct) if !ct.provider_visible(ctx) => {}
                Some(_) => dynamic.push(entry),
                None => stable.push(entry),
            }
        }
        stable.sort();
        dynamic.sort();
        stable
            .into_iter()
            .chain(dynamic)
            .map(|(name, description)| ToolSchema {
                name,
                description,
                parameters: Value::Null,
            })
            .collect()
    }

    /// The contextual projection for host metadata and diagnostics. Provider
    /// requests use [`Registry::schemas`] so phase changes do not churn the
    /// cache-stable tool contract.
    pub fn schemas_for_context(&self, ctx: &crate::abi::CallContext) -> Vec<ToolSchema> {
        let snapshot = self.visible_snapshot();
        let mut out = Vec::new();
        for (_, t) in snapshot {
            if let Some(ct) = t.capabilities().get::<dyn crate::abi::ContextualTool>()
                && !ct.provider_visible(ctx)
            {
                continue;
            }
            out.push(ToolSchema {
                name: t.name().to_owned(),
                description: t.description().to_owned(),
                parameters: Value::Null,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    fn visible_snapshot(&self) -> Vec<(String, Arc<dyn Tool>)> {
        let st = self.lock();
        let mut names: Vec<&String> = st
            .order
            .iter()
            .filter(|name| is_provider_visible(&st, name))
            .collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| st.tools.get(name).map(|t| (name.clone(), t.clone())))
            .collect()
    }

    /// The registry's provider-visible contract snapshot. Per-tool calls run
    /// outside the lock (see module docs).
    pub fn contract_entries(&self) -> Vec<ContractEntry> {
        self.contract_entries_visible(true)
    }

    /// Every registered tool's contract, including tools hidden from the
    /// provider schema — capability catalogs use this so use_capability can
    /// list and call tool:<name> targets.
    pub fn all_contract_entries(&self) -> Vec<ContractEntry> {
        self.contract_entries_visible(false)
    }

    fn contract_entries_visible(&self, provider_visible_only: bool) -> Vec<ContractEntry> {
        let snapshot: Vec<Arc<dyn Tool>> = {
            let st = self.lock();
            st.order
                .iter()
                .filter(|name| !provider_visible_only || is_provider_visible(&st, name))
                .filter_map(|name| st.tools.get(name).cloned())
                .collect()
        };
        let mut entries: Vec<ContractEntry> = snapshot
            .iter()
            .map(|t| {
                let caps: CapabilitySet = t.capabilities();
                let aliases = caps
                    .get::<dyn CapabilityAliased>()
                    .map(|a| a.capability_aliases())
                    .unwrap_or_default();
                ContractEntry {
                    name: t.name().to_owned(),
                    description: t.description().trim().to_owned(),
                    read_only: t.read_only(),
                    schema: canonicalize_schema(&t.schema()),
                    mcp: mcp_binding_for(t.as_ref()),
                    capability_aliases: aliases,
                }
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Fixes the ceiling on what the registry may hold. Tools already
    /// registered that `admit` rejects are removed, and every later add is
    /// checked against it. `admit` runs outside the registry lock: a lazy
    /// tool's own methods may take a lock a registry swap holds.
    pub fn restrict(&self, admit: AdmitFn) {
        {
            self.lock().admit = Some(admit.clone());
        }
        let held: Vec<(String, Arc<dyn Tool>)> = {
            let st = self.lock();
            st.order
                .iter()
                .filter_map(|name| st.tools.get(name).map(|t| (name.clone(), t.clone())))
                .collect()
        };
        let rejected: BTreeSet<String> = held
            .iter()
            .filter(|(_, t)| !admit(t.as_ref()))
            .map(|(name, _)| name.clone())
            .collect();
        let mut st = self.lock();
        drop_under_prefix(&mut st, |name| rejected.contains(name));
        st.schema_rev += 1;
    }

    /// Applies execution authority to existing and future MCP adapters,
    /// re-adding the MCP tools already registered.
    pub fn bind_mcp_runtime(&self, bind: BindMcpFn) {
        {
            self.lock().bind_mcp = Some(bind);
        }
        let held: Vec<Arc<dyn Tool>> = {
            let st = self.lock();
            st.order
                .iter()
                .filter_map(|name| st.tools.get(name).cloned())
                .filter(|t| mcp_binding_for(t.as_ref()).is_some())
                .collect()
        };
        for t in held {
            self.add(t);
        }
    }

    /// Adds or removes names from an existing provider-visible restriction;
    /// with no restriction every registered tool is already shown.
    pub fn show_to_provider(&self, shown: bool, names: &[&str]) {
        let mut st = self.lock();
        let Some(visible) = st.provider_visible.as_mut() else {
            return;
        };
        let mut changed = false;
        for name in names {
            if shown {
                changed |= visible.insert((*name).to_owned());
            } else {
                changed |= visible.remove(*name);
            }
        }
        if changed {
            st.schema_rev += 1;
        }
    }

    /// Removes the named tools from every surface and refuses any later
    /// registration, so no assembly path brings one back. A tool a host
    /// feature requires is kept and reported. Names nothing has registered
    /// are remembered for tools that arrive later.
    pub fn disable_tools(&self, names: &[&str]) -> Vec<DisableConflict> {
        let mut st = self.lock();
        let mut conflicts = Vec::new();
        let mut drop = BTreeSet::new();
        for name in names {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let required_by = st
                .tools
                .get(name)
                .and_then(|t| t.capabilities().get::<dyn HostRequired>())
                .map(|r| r.host_required_by());
            if let Some(by) = required_by {
                conflicts.push(DisableConflict {
                    tool: name.to_owned(),
                    by,
                });
                continue;
            }
            st.disabled_tools.insert(name.to_owned());
            if st.tools.contains_key(name) {
                drop.insert(name.to_owned());
            }
        }
        if !drop.is_empty() {
            drop_under_prefix(&mut st, |name| drop.contains(name));
            st.schema_rev += 1;
        }
        conflicts
    }

    /// Removed names no tool is registered under, sorted.
    pub fn disabled_names(&self) -> Vec<String> {
        let st = self.lock();
        st.disabled_tools
            .iter()
            .filter(|name| !st.tools.contains_key(*name))
            .cloned()
            .collect()
    }

    /// The typed refusal for a tool the user's configuration removed.
    /// `None` for any other name.
    pub fn disabled_refusal(&self, name: &str) -> Option<Refusal> {
        let name = name.trim();
        let st = self.lock();
        let disabled = st.disabled_tools.contains(name) && !st.tools.contains_key(name);
        if !disabled {
            return None;
        }
        Some(Refusal::code(
            CODE_TOOL_DISABLED,
            format!(
                "blocked: tool {name:?} is turned off in this configuration ([tools] disabled); it was not run"
            ),
        )
        .with_remedy(crate::refusal::RemedyOp::Open, "tools.disabled"))
    }

    /// Answers for a tool name the registry does not serve: the user's
    /// removal first, then an MCP tool the configuration disabled or a
    /// definition held for re-approval.
    pub fn call_refusal(&self, name: &str) -> Option<Refusal> {
        self.disabled_refusal(name)
            .or_else(|| self.mcp_refusal(name))
    }

    /// Replaces one server's disabled-tool policy so a removed, renamed, or
    /// reconnected server cannot leave stale aliases that blame
    /// configuration for a tool that no longer exists.
    pub fn replace_disabled_mcp(&self, server: &str, bindings: &[McpBinding]) {
        let server = server.trim();
        if server.is_empty() {
            return;
        }
        let mut st = self.lock();
        if bindings.is_empty() {
            st.disabled_mcp_by_server.remove(server);
        } else {
            let mut names = BTreeSet::new();
            for b in bindings {
                names.extend(entries::disabled_mcp_names(b));
            }
            st.disabled_mcp_by_server.insert(server.to_owned(), names);
        }
        rebuild_disabled_mcp(&mut st);
    }

    /// Adds one binding to its server's disabled-tool policy. Prefer
    /// [`Registry::replace_disabled_mcp`] for a complete snapshot.
    pub fn mark_disabled_mcp(&self, binding: &McpBinding) {
        let server = if binding.server.trim().is_empty() {
            binding.callable_name.trim()
        } else {
            binding.server.trim()
        };
        if server.is_empty() {
            return;
        }
        let mut st = self.lock();
        let added = entries::disabled_mcp_names(binding);
        for name in &added {
            st.disabled_mcp_by_server
                .entry(server.to_owned())
                .or_default()
                .insert(name.clone());
            st.disabled_mcp.insert(name.clone());
        }
    }

    /// Removes every disabled-tool alias recorded for one server.
    pub fn clear_disabled_mcp(&self, server: &str) {
        let server = server.trim();
        if server.is_empty() {
            return;
        }
        let mut st = self.lock();
        if st.disabled_mcp_by_server.remove(server).is_some() {
            rebuild_disabled_mcp(&mut st);
        }
    }

    /// Copies the disabled-tool and held-tool policy into a derived registry.
    /// Child agents may not copy the tools themselves, but they still need
    /// the same attribution when a stale call names a hidden tool.
    pub fn copy_disabled_mcp_from(&self, parent: &Registry) {
        if std::ptr::eq(self, parent) {
            return;
        }
        let by_server = {
            let p = parent.lock();
            p.disabled_mcp_by_server.clone()
        };
        {
            let mut st = self.lock();
            st.disabled_mcp_by_server = by_server;
            rebuild_disabled_mcp(&mut st);
        }
        self.copy_held_mcp_from(parent);
    }

    /// Whether the name identifies a tool disabled by configuration.
    pub fn disabled_mcp(&self, name: &str) -> bool {
        self.disabled_mcp_refusal(name).is_some()
    }

    /// The typed refusal for a configuration-disabled MCP tool. `None` for
    /// unknown names.
    pub fn disabled_mcp_refusal(&self, name: &str) -> Option<Refusal> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let st = self.lock();
        if st.disabled_mcp.contains(name) {
            Some(Refusal::code(
                CODE_MCP_TOOL_DISABLED,
                format!("blocked: tool {name:?} is disabled by configuration"),
            ))
        } else {
            None
        }
    }

    /// Replaces one server's held set, so a reconnect that no longer holds a
    /// tool stops refusing it.
    pub fn replace_held_mcp(&self, server: &str, held: &[HeldMcp]) {
        let server = server.trim();
        if server.is_empty() {
            return;
        }
        let mut st = self.lock();
        if held.is_empty() {
            st.held_mcp.remove(server);
            return;
        }
        let mut names = BTreeMap::new();
        for h in held {
            for name in entries::disabled_mcp_names(&h.binding) {
                names.insert(name, h.clone());
            }
        }
        st.held_mcp.insert(server.to_owned(), names);
    }

    /// Copies the held set into a derived registry.
    pub fn copy_held_mcp_from(&self, parent: &Registry) {
        if std::ptr::eq(self, parent) {
            return;
        }
        let held = parent.lock().held_mcp.clone();
        self.lock().held_mcp = held;
    }

    /// The typed refusal for a tool held pending re-approval. `None` for any
    /// other name.
    pub fn held_mcp_refusal(&self, name: &str) -> Option<Refusal> {
        let name = name.trim();
        let st = self.lock();
        st.held_mcp
            .values()
            .find_map(|names| names.get(name))
            .map(held_mcp_refusal_for)
    }

    /// Answers for an MCP tool name the registry does not serve: the
    /// configuration policy first, then a definition held for re-approval.
    pub fn mcp_refusal(&self, name: &str) -> Option<Refusal> {
        self.disabled_mcp_refusal(name)
            .or_else(|| self.held_mcp_refusal(name))
    }

    /// Swaps one name-prefixed namespace for the tools given, in a single
    /// lock hold: a request composed between a remove and its add would be
    /// built from a server that appears to have no tools. The admit ceiling
    /// and suspended prefixes bind as they do in add; a tool outside the
    /// prefix is ignored. Returns how many tools the namespace holds after.
    pub fn replace_prefix(&self, prefix: &str, tools: &[Arc<dyn Tool>]) -> usize {
        if prefix.is_empty() {
            return 0;
        }
        // admit may call back into a tool that takes its own lock, which a
        // registry swap must not be holding this one while it waits for.
        let admit = self.lock().admit.clone();
        let mut admitted: Vec<(String, Arc<dyn Tool>, Value)> = Vec::new();
        for t in tools {
            let name = t.name().to_owned();
            if !name.starts_with(prefix) {
                continue;
            }
            if admit.as_ref().is_some_and(|admit| !admit(t.as_ref())) {
                continue;
            }
            let canon = canonicalize_schema(&t.schema());
            admitted.push((name, t.clone(), canon));
        }

        let mut st = self.lock();
        drop_under_prefix(&mut st, |name| name.starts_with(prefix));
        let mut registered = 0;
        for (name, tool, canon) in admitted {
            if self.suspended(&st, &name) {
                continue;
            }
            if !st.tools.contains_key(&name) {
                st.order.push(name.clone());
            }
            st.tools.insert(name.clone(), tool);
            st.canon.insert(name, canon);
            registered += 1;
        }
        if registered > 0 {
            st.schema_rev += 1;
        }
        registered
    }
}

/// One registry lookup outcome.
#[derive(Clone)]
pub enum ResolveCall {
    Exact(Arc<dyn Tool>),
    Ambiguous(Vec<String>),
    Unknown,
}

impl std::fmt::Debug for ResolveCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveCall::Exact(t) => f.debug_tuple("Exact").field(&t.name()).finish(),
            ResolveCall::Ambiguous(c) => f.debug_tuple("Ambiguous").field(c).finish(),
            ResolveCall::Unknown => f.write_str("Unknown"),
        }
    }
}

/// A tool another host feature cannot work without. The owner of that
/// feature says so, so which tools stay is declared where the dependence
/// lives, not in a name list kept beside the setting.
pub trait HostRequired: crate::abi::Capability {
    fn host_required_by(&self) -> String;
}

/// A tool that answers to capability ids besides tool:<name>. Declaring them
/// on the tool is what keeps the catalog that lists an id and the proxy that
/// resolves one reading the same table.
pub trait CapabilityAliased: crate::abi::Capability {
    fn capability_aliases(&self) -> Vec<String>;
}

/// How far one call of a tool can act beyond the conversation (tools.md
/// §1.25). A tool states it about itself; the host never reads it off a
/// name. The zero value says nothing, which is why a tool nobody classified
/// is never admitted where reach matters.
pub const REACH_UNSTATED: u8 = 0;
/// Only reads files through the host-confined readers: starts no program of
/// the model's choosing, touches no network and leaves nothing behind.
pub const REACH_LOCAL_READ: u8 = 1;
/// Only hands the turn back to the host: records a conclusion or a question
/// and changes nothing outside the session.
pub const REACH_HOST_CONTROL: u8 = 2;

/// A tool that states its reach.
pub trait ReachDeclarer: crate::abi::Capability {
    fn reach(&self) -> u8;
}

/// What the tool declared, or [`REACH_UNSTATED`].
pub fn reach_of(tool: &dyn Tool) -> u8 {
    tool.capabilities()
        .get::<dyn ReachDeclarer>()
        .map_or(REACH_UNSTATED, |d| d.reach())
}

/// `open_projects`: approving adds the folders the card lists to the user's
/// project list and marks them trusted (Go `ApprovalScopeOpenProjects`).
pub const APPROVAL_SCOPE_OPEN_PROJECTS: &str = "open_projects";

/// A tool whose approval authorizes more than the call itself. The scope is
/// a code a frontend renders in the reader's language; the person approving
/// has to be able to see what it covers (Go `ApprovalScoper`).
pub trait ApprovalScoper: crate::abi::Capability {
    fn approval_scope(&self) -> String;
}

/// The tool's declared approval scope, or "" when approving it authorizes
/// the call alone (Go `ApprovalScopeOf`).
pub fn approval_scope_of(tool: &dyn Tool) -> String {
    tool.capabilities()
        .get::<dyn ApprovalScoper>()
        .map_or(String::new(), |s| s.approval_scope())
}

fn is_provider_visible(st: &RegistryState, name: &str) -> bool {
    match &st.provider_visible {
        None => true,
        Some(set) => set.contains(name),
    }
}

fn remove_prefix_locked(st: &mut RegistryState, prefix: &str) -> usize {
    let before = st.order.len();
    drop_under_prefix(st, |name| name.starts_with(prefix));
    let removed = before - st.order.len();
    if removed > 0 {
        st.schema_rev += 1;
    }
    removed
}

fn drop_under_prefix(st: &mut RegistryState, drop: impl Fn(&str) -> bool) {
    st.order.retain(|name| {
        if drop(name) {
            st.tools.remove(name);
            st.canon.remove(name);
            false
        } else {
            true
        }
    });
}

fn rebuild_disabled_mcp(st: &mut RegistryState) {
    st.disabled_mcp = st
        .disabled_mcp_by_server
        .values()
        .flatten()
        .cloned()
        .collect();
}
