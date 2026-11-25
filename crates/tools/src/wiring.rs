//! Wave 40 integration: the live registry is built by walking the frozen
//! sixteen-step assembly order instead of being appended by hand.
//!
//! Every domain lands at its [`crate::assembly::insertion_points`] anchor, so a
//! later parity check can assert position, not only presence. Steps whose
//! tools are host-owned (process loading, network providers, MCP handshakes)
//! stay empty here on purpose: `step_tools` returns an empty vector for them
//! and their GO operation names remain visible in the frozen table.

#![forbid(unsafe_code)]

use std::sync::Arc;

use crate::abi::Tool;
use crate::assembly;
use crate::builtin;
use crate::registry::Registry;

/// The construction inputs the assembly needs beyond the frozen table.
#[derive(Debug, Clone, Default)]
pub struct WireEnv {
    /// Workspace root the confined writers map onto. Empty keeps writers out.
    pub workspace_root: String,
}

impl WireEnv {
    pub fn new(workspace_root: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

/// The tools this crate registers for one assembly step.
///
/// An empty vector is a host-side step: the Go operation exists in the frozen
/// table, but the tool it adds needs a process, a provider or a config source
/// that the composition root owns. Keeping the step in the walk preserves its
/// position without inventing a placeholder tool.
pub fn step_tools(id: &str, env: &WireEnv) -> Vec<Arc<dyn Tool>> {
    match id {
        // Step 2 addBuiltins: the confined read/write families, the web
        // readers, and the two host-free context cards the builtin batch
        // carries (fold-on-demand compress and the budget reporter).
        "builtins" => {
            let mut tools = builtin::confine_readers(&[]);
            tools.extend(builtin::web_tools());
            tools.push(Arc::new(builtin::ctxfold::CompressTool));
            tools.push(Arc::new(builtin::contextbudget::ContextBudgetTool));
            if !env.workspace_root.is_empty() {
                tools.extend(builtin::confine_writers(
                    std::slice::from_ref(&env.workspace_root),
                    None,
                    None,
                ));
            }
            tools
        }
        // Step 3 chain: addViewImage → addPresentFiles → … → addWebSearch.
        // media lands in the image slot, ahead of present_files, as Go orders
        // the image readers before file presentation.
        "image_present_views_peers_setting_systemone_advisor_websearch_fetchchain" => {
            let mut tools = builtin::media_tools();
            tools.push(Arc::new(builtin::agentstate::ViewImageTool::new(None)));
            tools.push(builtin::present_files_tool());
            tools.push(Arc::new(builtin::conditional::WebSearchTool::new(None)));
            tools
        }
        // Step 5 bindMachineTools: browser then computer, the Go relative order.
        "mcp_machine" => {
            let mut tools = builtin::browser_tools();
            tools.extend(builtin::computer_tools());
            tools
        }
        // Step 8 roles.delegation: the runner's two tools register against it.
        "delegation_runner" => vec![
            Arc::new(builtin::delegate::SpawnTool::new(None)),
            Arc::new(builtin::delegate::SubagentResultTool::new(
                None,
                env.workspace_root.clone(),
            )),
        ],
        // Step 9 registerSessionTools: planning, product docs, the turn-exit
        // decision, and the user-facing ask that every turn carries.
        "session_tools" => {
            let mut tools = builtin::plan_tools();
            tools.push(builtin::docs_tool(None));
            tools.push(Arc::new(builtin::agentstate::AwaitUserTool::new(None)));
            tools.extend(builtin::turn_exit_tools());
            tools
        }
        // Step 11 addInstallSourceTool: the model-facing card, so an
        // unticketed apply answers with a plan instead of installing.
        "commands_install_source" => vec![Arc::new(
            builtin::conditional_slash::InstallSourceTool::new(None, None, true),
        )],
        // Step 12 registerSkillTools: the four skill tools, then the
        // slash-command tool LAST so it carries every entry.
        "skill_tools" => vec![
            Arc::new(builtin::skill_tools::RunSkillTool::new(None, None, None)),
            Arc::new(builtin::skill_tools::ReadOnlySkillTool::new(
                None, None, None,
            )),
            Arc::new(builtin::skill_tools::ReadSkillTool::new(None)),
            Arc::new(builtin::skill_tools::InstallSkillTool::new(None, None)),
            Arc::new(builtin::conditional_slash::SlashCommandTool::new(
                builtin::conditional_slash::SlashCatalog::build(Vec::new()),
                None,
            )),
        ],
        // Host-side steps: extension loading (14), the capability surface and
        // the use_capability proxy (13/15), and the catalog gate (16).
        // TODO(wave41-capability-proxy): add the proxy once the catalog and
        // ledger drive a live inventory.
        _ => Vec::new(),
    }
}

/// Walks the frozen table in order and collects every tool this crate owns.
pub fn assembled_tools(env: &WireEnv) -> Vec<Arc<dyn Tool>> {
    let mut tools = Vec::new();
    for supplier in assembly::builtin_suppliers() {
        tools.extend(step_tools(supplier.id, env));
    }
    tools
}

/// Builds the registry a runtime drives: the frozen order, one pass.
pub fn assemble_registry(workspace_root: &str) -> Registry {
    let registry = Registry::new();
    for tool in assembled_tools(&WireEnv::new(workspace_root)) {
        registry.add(tool);
    }
    registry
}

/// The step a tool name lands in, read off the assembled walk. Used by the
/// parity test to pin position rather than mere presence.
pub fn step_of(name: &str, env: &WireEnv) -> Option<u16> {
    for supplier in assembly::builtin_suppliers() {
        if step_tools(supplier.id, env)
            .iter()
            .any(|tool| tool.name() == name)
        {
            return Some(supplier.step);
        }
    }
    None
}
