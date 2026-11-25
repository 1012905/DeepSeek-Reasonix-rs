//! TOOL-036: the `wireTools` assembly order (docs/contracts/tools.md §3.0;
//! Go `internal/assembly/boot/build.go:359-455`).
//!
//! The registry is assembled in a fixed behavioral order: later tools see
//! earlier ones, and `use_capability` is added LAST so the capability surface
//! it narrates is complete. This module freezes the sixteen steps as an
//! append-only supplier list. The integration slice binds each step's
//! factories to the live registry; this slice owns the order semantics, the
//! step numbers and the Go-side comments, not the runtime wiring.
//!
//! Extending the list: a new domain's tools land as either an additional op
//! inside their anchor step's [`Supplier::ops`] list, or a new [`Supplier`]
//! row placed beside its anchor step. Neither path rewrites this table; the
//! step guard below keeps the numbering contiguous and the proxy last.

#![forbid(unsafe_code)]

/// The frozen number of wireTools steps (Go `build.go:361-454`).
pub const EXPECTED_STEP_COUNT: usize = 16;

/// One assembly step. `step` keeps the Go 1-based number; `ops` lists the
/// ordered Go operations inside composite steps, left to right as the source
/// calls them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supplier {
    /// Go `wireTools` step number (1-based).
    pub step: u16,
    /// Stable slug the integration slice anchors new entries to.
    pub id: &'static str,
    /// The Go function / operation this step calls, for cross-reference.
    pub go_symbol: &'static str,
    /// The contract sentence that fixes the step's behavior.
    pub note: &'static str,
    /// Ordered sub-operations inside the step; one entry for a plain step.
    pub ops: &'static [&'static str],
}

/// The sixteen built-in suppliers in wireTools order.
pub fn builtin_suppliers() -> Vec<Supplier> {
    Vec::from(BUILTIN_SUPPLIERS)
}

/// The frozen table, one row per Go step.
const BUILTIN_SUPPLIERS: &[Supplier] = &[
    Supplier {
        step: 1,
        id: "registry",
        go_symbol: "newToolRegistry / resolveToolEnvironment",
        note: "newToolRegistry → resolveToolEnvironment. The registry exists before any tool is added to it.",
        ops: &["newToolRegistry", "resolveToolEnvironment"],
    },
    Supplier {
        step: 2,
        id: "builtins",
        go_symbol: "addBuiltins (boot.go:406)",
        note: "addBuiltins: Workspace.Tools(enabled...), each reg.Add; unknown name warns on stderr. Confined replacements are added ONLY IF the plain tool is already present.",
        ops: &["addBuiltins"],
    },
    Supplier {
        step: 3,
        id: "image_present_views_peers_setting_systemone_advisor_websearch_fetchchain",
        go_symbol: "addViewImage → addPresentFiles → bindFileViews → peerTools → addSettingTool → addSystemOne → addAdvisor → addWebSearch → addFetchChain",
        note: "Step 3 chain: view image, present files, file views binding, peer tools, setting, system one, advisor, web search, fetch chain — in this order.",
        ops: &[
            "addViewImage",
            "addPresentFiles",
            "bindFileViews",
            "peerTools",
            "addSettingTool",
            "addSystemOne",
            "addAdvisor",
            "addWebSearch",
            "addFetchChain",
        ],
    },
    Supplier {
        step: 4,
        id: "script",
        go_symbol: "cfg.Agent.CodeMode → reg.Add(script.New())",
        note: "Code mode on: register the script tool. Gated on the agent config.",
        ops: &["registerScriptTool"],
    },
    Supplier {
        step: 5,
        id: "mcp_machine",
        go_symbol: "wireMCP → bindMachineTools",
        note: "wireMCP, then bindMachineTools (browser/computer session tools). Connection-supplied names fill the MCP namespace here.",
        ops: &["wireMCP", "bindMachineTools"],
    },
    Supplier {
        step: 6,
        id: "subagent_store_policy_gate",
        go_symbol: "newSubagentStore / permission.New / control.NewSharedHeadlessGate",
        note: "Subagent store with destroyed-checker; permission policy (dynamic bash fallback, ask-on-origin, session allow); one shared headless gate for the executor and every sub-agent.",
        ops: &[
            "newSubagentStore",
            "buildPermissionPolicy",
            "sharedHeadlessGate",
        ],
    },
    Supplier {
        step: 7,
        id: "hooks_roles_subagent_config",
        go_symbol: "loadHooks / roleWiring / newSubagentConfig",
        note: "Load hooks, wire roles, build the subagent config (task model/effort fallbacks).",
        ops: &["loadHooks", "roleWiring", "newSubagentConfig"],
    },
    Supplier {
        step: 8,
        id: "delegation_runner",
        go_symbol: "roles.delegation",
        note: "runner + skillRun = roles.delegation. The delegation surface exists before session and skill tools register against it.",
        ops: &["delegationRunner"],
    },
    Supplier {
        step: 9,
        id: "session_tools",
        go_symbol: "registerSessionTools",
        note: "Product docs, indexed history (gated !Retrieval), sessiontool list/read, memory recall/remember/forget gates, turn-exit tools.",
        ops: &["registerSessionTools"],
    },
    Supplier {
        step: 10,
        id: "native_surface",
        go_symbol: "agent.NewRenderSurfaceTool",
        note: "Native surfaces on: register the render-surface tool.",
        ops: &["registerRenderSurfaceTool"],
    },
    Supplier {
        step: 11,
        id: "commands_install_source",
        go_symbol: "skillRunners → loadCommands → addInstallSourceTool",
        note: "Skill runners, loaded slash commands, then the install-source tool.",
        ops: &["skillRunners", "loadCommands", "addInstallSourceTool"],
    },
    Supplier {
        step: 12,
        id: "skill_tools",
        go_symbol: "registerSkillTools",
        note: "read_only_skill (gated), run_skill (gated), read_skill, install_skill, builtin subagent tools; the slash-command tool is added LAST so it carries every entry.",
        ops: &[
            "registerReadOnlySkill",
            "registerRunSkill",
            "registerReadSkill",
            "registerInstallSkill",
            "registerBuiltinSubagentTools",
            "registerSlashCommandTool",
        ],
    },
    Supplier {
        step: 13,
        id: "capability_surface",
        go_symbol: "newCapabilitySurface / ConfigureComposition / ConfigureInventorySource / BindMCPRuntime",
        note: "Capability surface: composition configuration, inventory source, MCP runtime binding; resource read tool when the registry holds that name.",
        ops: &[
            "newCapabilitySurface",
            "configureComposition",
            "bindMcpRuntime",
            "resourceReadTool",
        ],
    },
    Supplier {
        step: 14,
        id: "extension_tools",
        go_symbol: "addExtensionTools",
        note: "ext__-prefixed extension tools; refresh re-adds them on reload.",
        ops: &["addExtensionTools"],
    },
    Supplier {
        step: 15,
        id: "capability_proxy",
        go_symbol: "reg.Add(t.caps.proxy)",
        note: "use_capability LAST: the proxy narrates the complete inventory, so every earlier step must already be registered.",
        ops: &["addCapabilityProxy"],
    },
    Supplier {
        step: 16,
        id: "catalog_gate",
        go_symbol: "gateSkillsOnCatalog",
        note: "Gate skills on the catalog after the proxy and every tool are registered.",
        ops: &["gateSkillsOnCatalog"],
    },
];

/// The stable id of the step that registers `use_capability`.
pub const PROXY_SUPPLIER_ID: &str = "capability_proxy";
/// The stable id of the final catalog-gate step.
pub const GATE_SUPPLIER_ID: &str = "catalog_gate";

/// Where a later domain's tools plug into the frozen order. Integration
/// slices read this table instead of re-deriving the anchor step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertionPoint {
    /// Domain slug matching the parallel slice's group (web/browser/computer/media/plan).
    pub domain: &'static str,
    /// Anchor step the new tools register beside.
    pub anchor_step: u16,
    /// Anchor supplier id.
    pub anchor_id: &'static str,
    /// The ordered slot inside the anchor step's [`Supplier::ops`] list.
    pub slot: &'static str,
    /// The shape an integration entry takes.
    pub shape: &'static str,
}

/// Insertion points for the wave39-a domain groups, anchored to their Go
/// counterparts. New rows are grouped under the anchor step; they never
/// reorder the frozen steps.
pub fn insertion_points() -> &'static [InsertionPoint] {
    &[
        InsertionPoint {
            domain: "web",
            anchor_step: 2,
            anchor_id: "builtins",
            slot: "addBuiltins",
            shape: "web_fetch joins the builtin batch in step 2; web_search joins the step 3 addWebSearch slot. Each tool is an Arc<dyn Tool> factory handed to the anchor step's registration closure.",
        },
        InsertionPoint {
            domain: "browser",
            anchor_step: 5,
            anchor_id: "mcp_machine",
            slot: "bindMachineTools",
            shape: "browser_open/read/act register beside bindMachineTools in step 5, after wireMCP. Session tools bind through the same machine-tools seam; no earlier step may reference their names.",
        },
        InsertionPoint {
            domain: "computer",
            anchor_step: 5,
            anchor_id: "mcp_machine",
            slot: "bindMachineTools",
            shape: "computer_read/act share the step 5 machine-tools group with browser; registered after the browser entries so the two session families keep their Go relative order.",
        },
        InsertionPoint {
            domain: "media",
            anchor_step: 3,
            anchor_id: "image_present_views_peers_setting_systemone_advisor_websearch_fetchchain",
            slot: "addViewImage",
            shape: "viewimage/readvideo join the step 3 view-image slot, ahead of present_files. File-presentation tools stay after the image readers, as in the Go chain.",
        },
        InsertionPoint {
            domain: "plan",
            anchor_step: 9,
            anchor_id: "session_tools",
            slot: "registerSessionTools",
            shape: "plan/goal/context tools register in the step 9 session-tools batch; skill-facing plan wrappers instead join the step 12 registerSkillTools group, ahead of the trailing slash-command tool.",
        },
    ]
}

/// Assembly-order validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssemblyError {
    /// The table no longer carries the frozen sixteen steps.
    #[error("assembly has {count} suppliers, want {expected}")]
    StepCountMismatch { count: usize, expected: usize },
    /// A step number is missing, duplicated, or out of the 1..=16 range.
    #[error("step sequence is not contiguous 1..={expected}: {detail}")]
    NonContiguousSteps { expected: usize, detail: String },
    /// Two suppliers share a stable id.
    #[error("duplicate supplier id {0:?}")]
    DuplicateId(&'static str),
    /// The capability proxy is not the tool-adding step immediately before the
    /// catalog gate.
    #[error("proxy step {proxy:?} must precede gate step {gate:?}")]
    ProxyNotLast { proxy: u16, gate: u16 },
    /// A conditional surface anchors to an unknown supplier id.
    #[error("conditional surface {slug:?} anchors to unknown supplier {anchor_id:?}")]
    ConditionalAnchorUnknown {
        slug: &'static str,
        anchor_id: &'static str,
    },
    /// The anchor step disagrees with the frozen supplier table.
    #[error(
        "conditional surface {slug:?} anchors step {got}, supplier {anchor_id:?} is step {want}"
    )]
    ConditionalStepMismatch {
        slug: &'static str,
        anchor_id: &'static str,
        got: u16,
        want: u16,
    },
}

/// Validates the frozen table: sixteen contiguous steps, unique ids, the proxy
/// registered immediately ahead of the final gate.
pub fn validate() -> Result<(), AssemblyError> {
    let suppliers = builtin_suppliers();
    if suppliers.len() != EXPECTED_STEP_COUNT {
        return Err(AssemblyError::StepCountMismatch {
            count: suppliers.len(),
            expected: EXPECTED_STEP_COUNT,
        });
    }
    for (i, supplier) in suppliers.iter().enumerate() {
        if supplier.step as usize != i + 1 {
            return Err(AssemblyError::NonContiguousSteps {
                expected: EXPECTED_STEP_COUNT,
                detail: format!(
                    "position {i} carries step {}, want {}",
                    supplier.step,
                    i + 1
                ),
            });
        }
    }
    for (i, supplier) in suppliers.iter().enumerate() {
        if suppliers[..i].iter().any(|other| other.id == supplier.id) {
            return Err(AssemblyError::DuplicateId(supplier.id));
        }
    }
    let proxy = suppliers
        .iter()
        .find(|s| s.id == PROXY_SUPPLIER_ID)
        .expect("proxy supplier is in the frozen table");
    let gate = suppliers
        .iter()
        .find(|s| s.id == GATE_SUPPLIER_ID)
        .expect("gate supplier is in the frozen table");
    if proxy.step + 1 != gate.step {
        return Err(AssemblyError::ProxyNotLast {
            proxy: proxy.step,
            gate: gate.step,
        });
    }
    Ok(())
}

/// Looks up a supplier by its 1-based Go step number.
pub fn supplier_at(step: u16) -> Option<&'static Supplier> {
    BUILTIN_SUPPLIERS.iter().find(|s| s.step == step)
}

/// Looks up a supplier by stable id.
pub fn supplier_by_id(id: &str) -> Option<&'static Supplier> {
    BUILTIN_SUPPLIERS.iter().find(|s| s.id == id)
}

/// One conditional surface resolved against its frozen anchor supplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionalAnchor<'a> {
    /// The conditional face record.
    pub surface: &'a crate::builtin::conditional::ConditionalSurface,
    /// The frozen supplier it registers beside.
    pub supplier: &'a Supplier,
}

/// Resolves each conditional surface to its anchor supplier. The guard checks
/// the anchor id exists and its step agrees with the frozen table; the slot op
/// itself may be a new op the integration slice appends to the anchor step's
/// [`Supplier::ops`] list. This never rewrites the frozen order.
pub fn resolve_conditional_anchors(
    surfaces: &[crate::builtin::conditional::ConditionalSurface],
) -> Result<Vec<ConditionalAnchor<'_>>, AssemblyError> {
    surfaces
        .iter()
        .map(|surface| {
            let supplier = supplier_by_id(surface.anchor_id).ok_or(
                AssemblyError::ConditionalAnchorUnknown {
                    slug: surface.slug,
                    anchor_id: surface.anchor_id,
                },
            )?;
            if supplier.step != surface.anchor_step as u16 {
                return Err(AssemblyError::ConditionalStepMismatch {
                    slug: surface.slug,
                    anchor_id: surface.anchor_id,
                    got: surface.anchor_step as u16,
                    want: supplier.step,
                });
            }
            Ok(ConditionalAnchor { surface, supplier })
        })
        .collect()
}
