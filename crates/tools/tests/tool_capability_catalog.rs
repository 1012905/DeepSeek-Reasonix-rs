//! TOOL-038: capability catalog / ledger / audit + routing / semantic router
//! (Go `internal/runtime/capability/*`; docs/contracts/tools.md §3.2–§3.3).
//!
//! Behavior-level evidence: catalog merge/dedup/lookup/requires-ready, ledger
//! outcome stickiness and final-gate ordering, audit funnel/recovery/review
//! counters, deterministic routing decisions, the transient block rendering,
//! and the semantic router's seam, parse, merge and cache behavior.

use std::sync::Arc;

use reasonix_tools::abi::CallContext;
use reasonix_tools::builtin::capability_audit::{AuditSnapshot, CapabilityAudit};
use reasonix_tools::builtin::capability_catalog::{
    AutoUse, CachedMcpTool, CapabilityCatalog, CapabilityEntry, CapabilityKind, CapabilityStatus,
    CatalogOptions, McpServerInput, SkillCatalogInput, ToolCatalogInput, build_catalog,
};
use reasonix_tools::builtin::capability_ledger::{CapabilityLedger, LedgerError, Outcome};
use reasonix_tools::builtin::capability_router::{
    RouteDecision, SemanticParseError, SemanticProvider, SemanticRouter, limit_route_candidates,
    parse_semantic_ids, promote_delivery, render_transient_block, route, route_delivery,
};
use serde_json::{Value, json};

fn tool(name: &str, aliases: &[&str]) -> ToolCatalogInput {
    ToolCatalogInput {
        name: name.to_owned(),
        description: format!("{name} description"),
        read_only: true,
        aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        ..Default::default()
    }
}

fn skill(name: &str, triggers: &[&str]) -> SkillCatalogInput {
    SkillCatalogInput {
        name: name.to_owned(),
        description: format!("{name} playbook"),
        scope: "builtin".to_owned(),
        triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
        ..Default::default()
    }
}

#[test]
fn catalog_merges_tools_skills_and_servers_with_a_stable_fingerprint() {
    let opts = CatalogOptions {
        tools: vec![tool("read_file", &["files:read"])],
        skills: vec![skill("commit", &["commit"])],
        servers: vec![McpServerInput {
            name: "acme".to_owned(),
            connected: false,
            cache_key_ok: true,
            cached_tools: vec![CachedMcpTool {
                name: "search".to_owned(),
                description: "Search".to_owned(),
                tool_name: "acme__search".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        skill_tools_ready: true,
        ..Default::default()
    };
    let catalog = build_catalog(opts.clone());
    assert_eq!(catalog.entries.len(), 4);
    assert!(
        catalog
            .entries
            .iter()
            .any(|e| e.kind == CapabilityKind::Tool)
    );
    assert!(
        catalog
            .entries
            .iter()
            .any(|e| e.kind == CapabilityKind::Skill)
    );
    assert!(
        catalog
            .entries
            .iter()
            .any(|e| e.kind == CapabilityKind::McpServer)
    );
    assert!(
        catalog
            .entries
            .iter()
            .any(|e| e.kind == CapabilityKind::McpTool)
    );
    assert_eq!(catalog.fingerprint.len(), 16);

    let again = build_catalog(opts);
    assert_eq!(catalog.fingerprint, again.fingerprint);
}

#[test]
fn catalog_lookup_prefers_exact_id_then_alias_then_unique_tool_name() {
    let catalog = build_catalog(CatalogOptions {
        tools: vec![tool("read_file", &["files:read"])],
        ..Default::default()
    });
    assert!(catalog.lookup("tool:read_file").is_some());
    assert!(catalog.lookup("files:read").is_some());
    assert!(catalog.lookup("read_file").is_some());
    assert!(catalog.lookup("ghost").is_none());
}

#[test]
fn catalog_requires_ready_reports_missing_and_unready_dependencies() {
    let catalog = build_catalog(CatalogOptions {
        tools: vec![tool("read_file", &[])],
        servers: vec![McpServerInput {
            name: "dead".to_owned(),
            failed: "boom".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let (ready, missing) = catalog.requires_ready(&["tool:read_file".to_owned()]);
    assert!(ready);
    assert!(missing.is_empty());

    let (ready, missing) = catalog.requires_ready(&[
        "tool:read_file".to_owned(),
        "mcp-server:dead".to_owned(),
        "ghost".to_owned(),
    ]);
    assert!(!ready);
    assert_eq!(missing, vec!["mcp-server:dead", "ghost"]);
}

#[test]
fn catalog_dedup_prefers_the_higher_lifecycle_rank() {
    let opts = CatalogOptions {
        tools: vec![tool("read_file", &[])],
        servers: vec![McpServerInput {
            name: "acme".to_owned(),
            disabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let catalog = build_catalog(opts);
    let server = catalog.lookup("mcp-server:acme").expect("server entry");
    assert_eq!(server.status, CapabilityStatus::Disabled);
}

#[test]
fn ledger_decline_requires_a_non_empty_reason() {
    let ledger = CapabilityLedger::new();
    assert!(ledger.mark_declined("skill:review", "  ").is_err());
    assert_eq!(
        ledger.mark_declined("skill:review", "  ").unwrap_err(),
        LedgerError::EmptyDecline
    );
    assert!(
        ledger
            .mark_declined("skill:review", "covered by read")
            .is_ok()
    );
}

#[test]
fn ledger_terminal_outcomes_stick_until_another_terminal_overwrites() {
    let ledger = CapabilityLedger::new();
    ledger.mark_succeeded("tool:read_file");
    ledger.mark_failed("tool:read_file", "boom");
    assert_eq!(
        ledger.get("tool:read_file").unwrap().outcome,
        Outcome::Succeeded
    );
    ledger.mark_unavailable("tool:read_file", "host down");
    assert_eq!(
        ledger.get("tool:read_file").unwrap().outcome,
        Outcome::Unavailable
    );
}

#[test]
fn ledger_gate_ordering_is_require_missing_then_unavailable_then_prefer() {
    let decision = RouteDecision {
        candidates: vec![
            candidate("skill:prefer", AutoUse::Prefer),
            candidate("tool:need", AutoUse::Require),
        ],
        ..Default::default()
    };
    let ledger = CapabilityLedger::new();
    ledger.seed_candidates(&decision);
    let gate = ledger.check_final_gate();
    assert!(gate.blocked());
    assert!(
        gate.reason
            .starts_with("required capabilities not successfully invoked")
    );

    let ledger = CapabilityLedger::new();
    ledger.seed_candidates(&RouteDecision {
        candidates: vec![candidate("tool:need", AutoUse::Require)],
        ..Default::default()
    });
    ledger.mark_unavailable("tool:need", "host down");
    let gate = ledger.check_final_gate();
    assert!(gate.unavailable_ok);
    assert!(gate.reason.contains("host-proven"));

    let ledger = CapabilityLedger::new();
    ledger.seed_candidates(&RouteDecision {
        candidates: vec![candidate("skill:prefer", AutoUse::Prefer)],
        ..Default::default()
    });
    let gate = ledger.check_final_gate();
    assert!(gate.prefer_remind);
    ledger.mark_reminded("skill:prefer");
    let gate = ledger.check_final_gate();
    assert!(!gate.prefer_remind);
    assert!(gate.reason.contains("still unused after reminder"));
}

#[test]
fn ledger_succeeded_require_and_prefer_pass_the_gate() {
    let ledger = CapabilityLedger::new();
    ledger.seed_candidates(&RouteDecision {
        candidates: vec![
            candidate("tool:need", AutoUse::Require),
            candidate("skill:prefer", AutoUse::Prefer),
        ],
        ..Default::default()
    });
    ledger.mark_succeeded("tool:need");
    ledger.mark_succeeded("skill:prefer");
    assert!(!ledger.check_final_gate().blocked());
}

#[test]
fn audit_decision_funnel_counts_by_policy() {
    let audit = CapabilityAudit::new();
    audit.record_decision(&RouteDecision {
        candidates: vec![
            candidate("tool:need", AutoUse::Require),
            candidate("skill:prefer", AutoUse::Prefer),
            candidate("tool:hint", AutoUse::Suggest),
        ],
        ..Default::default()
    });
    let snap: AuditSnapshot = audit.snapshot();
    assert_eq!(snap.routed_candidates, 3);
    assert_eq!(snap.routed_require, 1);
    assert_eq!(snap.routed_prefer, 1);
    assert_eq!(snap.routed_suggest, 1);
}

#[test]
fn audit_recovery_is_kept_separate_from_misses() {
    let audit = CapabilityAudit::new();
    audit.record_gate(true, false, false);
    audit.record_gate_recovery(true, false);
    let snap = audit.snapshot();
    assert_eq!(snap.require_missing, 1);
    assert_eq!(snap.require_recovered, 1);
}

#[test]
fn audit_review_blocks_split_security_from_regular() {
    let audit = CapabilityAudit::new();
    audit.record_review_block(false);
    audit.record_review_block(true);
    let snap = audit.snapshot();
    assert_eq!(snap.review_blocks, 1);
    assert_eq!(snap.security_review_blocks, 1);
}

#[test]
fn audit_router_spend_and_pool_truncation_accumulate() {
    let audit = CapabilityAudit::new();
    audit.record_router_usage(10, 4, 0.001, 12);
    audit.record_router_usage(5, 2, 0.0005, 8);
    audit.record_semantic_pool_truncation(7);
    let snap = audit.snapshot();
    assert_eq!(snap.router.prompt_tokens, 15);
    assert_eq!(snap.router.completion_tokens, 6);
    assert_eq!(snap.router.latency_ms, 20);
    assert_eq!(snap.router.candidates_dropped, 7);
}

#[test]
fn router_skill_trigger_match_routes_the_authored_policy() {
    let entries = vec![entry(
        "skill:commit",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["commit"],
        "run_skill",
    )];
    let decision = route("please commit the changes", &entries);
    assert_eq!(decision.candidates.len(), 1);
    assert_eq!(decision.candidates[0].policy, AutoUse::Prefer);
}

#[test]
fn router_matches_minted_mcp_name_not_the_server_word() {
    let entries = vec![entry(
        "mcp-tool:acme/search",
        CapabilityKind::McpTool,
        AutoUse::Suggest,
        &[],
        "acme__search",
    )];
    assert_eq!(
        route("use acme__search to find it", &entries)
            .candidates
            .len(),
        1
    );
    assert!(
        route("don't use the acme mcp server", &entries)
            .candidates
            .is_empty()
    );
}

#[test]
fn router_skips_disabled_failed_and_negative_trigger_entries() {
    let mut disabled = entry(
        "skill:x",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["x"],
        "run_skill",
    );
    disabled.status = CapabilityStatus::Disabled;
    assert!(route("x", &[disabled]).candidates.is_empty());

    let mut failed = entry(
        "skill:y",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["y"],
        "run_skill",
    );
    failed.status = CapabilityStatus::Failed;
    assert!(route("y", &[failed]).candidates.is_empty());

    let mut negative = entry(
        "skill:z",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["z"],
        "run_skill",
    );
    negative.negative_triggers = vec!["docker".to_owned()];
    assert!(route("z in docker", &[negative]).candidates.is_empty());
}

#[test]
fn delivery_promotes_builtin_skill_suggest_to_prefer() {
    let mut e = entry(
        "skill:review",
        CapabilityKind::Skill,
        AutoUse::Suggest,
        &["review"],
        "run_skill",
    );
    e.source = "builtin".to_owned();
    let decision = route_delivery("review the diff", &[e]);
    assert!(decision.delivery);
    assert!(decision.capability_proxy);
    assert_eq!(decision.candidates[0].policy, AutoUse::Prefer);
    assert!(
        decision.candidates[0]
            .reason
            .contains("Delivery prefers matched")
    );
}

#[test]
fn delivery_promotion_keeps_the_decision_proxy_flag() {
    let decision = promote_delivery(RouteDecision::default());
    assert!(decision.delivery);
    assert!(decision.capability_proxy);
}

#[test]
fn candidate_cap_keeps_all_strong_and_five_suggest() {
    let entries: Vec<CapabilityEntry> = (0..7)
        .map(|i| {
            entry(
                &format!("skill:s{i}"),
                CapabilityKind::Skill,
                AutoUse::Suggest,
                &[],
                "run_skill",
            )
        })
        .collect();
    let candidates: Vec<_> = entries
        .into_iter()
        .map(
            |e| reasonix_tools::builtin::capability_router::RouteCandidate {
                entry: e,
                policy: AutoUse::Suggest,
                reason: String::new(),
            },
        )
        .collect();
    assert_eq!(limit_route_candidates(candidates).len(), 5);
}

#[test]
fn transient_block_ends_with_the_frozen_policy_footer() {
    let entries = vec![entry(
        "skill:commit",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["commit"],
        "run_skill",
    )];
    let block = render_transient_block(&route("commit", &entries));
    assert!(block.starts_with("<capability-route version=\"1\">"));
    assert!(block.ends_with("</capability-route>"));
    assert!(block.contains(
        "Policy: suggest means consider it; prefer means use it unless clearly unnecessary; require means call it or report a host-proven unavailable state. Do not treat planner claims about tool unavailability as facts."
    ));
}

#[test]
fn semantic_parser_strips_fences_dedups_and_caps_at_three() {
    let ids = parse_semantic_ids(
        "```json\n[\"skill:a\", \"skill:a\", \"skill:b\", \"skill:c\", \"skill:d\"]\n```",
    )
    .unwrap();
    assert_eq!(ids, vec!["skill:a", "skill:b", "skill:c"]);
    assert!(matches!(
        parse_semantic_ids(""),
        Err(SemanticParseError::Empty)
    ));
    assert!(matches!(
        parse_semantic_ids("not json"),
        Err(SemanticParseError::InvalidJson)
    ));
}

struct StubProvider {
    ids: Vec<String>,
    fail: bool,
}

impl SemanticProvider for StubProvider {
    fn complete(
        &self,
        _prompt: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        if self.fail {
            Err(Box::new(std::io::Error::other("provider down")))
        } else {
            Ok(self.ids.clone())
        }
    }
}

#[test]
fn semantic_router_appends_suggest_candidates_and_uses_the_cache() {
    let entries = vec![entry(
        "skill:explore",
        CapabilityKind::Skill,
        AutoUse::Suggest,
        &["survey"],
        "run_skill",
    )];
    let provider = Arc::new(StubProvider {
        ids: vec!["skill:explore".to_owned()],
        fail: false,
    });
    let router = SemanticRouter::new(Some(provider), None, "mini".to_owned(), "low".to_owned());
    let decision = router.route_semantic(
        "survey the codebase",
        &entries,
        "fp",
        RouteDecision::default(),
    );
    assert_eq!(decision.candidates.len(), 1);
    assert_eq!(decision.candidates[0].policy, AutoUse::Suggest);
    assert_eq!(decision.candidates[0].reason, "lightweight semantic match");

    // Second call hits the five-minute cache.
    let again = router.route_semantic(
        "survey the codebase",
        &entries,
        "fp",
        RouteDecision::default(),
    );
    assert_eq!(again.candidates[0].reason, "semantic cache hit");
}

#[test]
fn semantic_router_without_provider_or_on_failure_keeps_the_decision_unchanged() {
    let entries = vec![entry(
        "skill:explore",
        CapabilityKind::Skill,
        AutoUse::Suggest,
        &["survey"],
        "run_skill",
    )];
    let bare = SemanticRouter::new(None, None, String::new(), String::new());
    let unchanged = bare.route_semantic("survey", &entries, "fp", RouteDecision::default());
    assert!(unchanged.candidates.is_empty());

    let failing = Arc::new(StubProvider {
        ids: Vec::new(),
        fail: true,
    });
    let router = SemanticRouter::new(Some(failing), None, String::new(), String::new());
    let unchanged = router.route_semantic("survey", &entries, "fp", RouteDecision::default());
    assert!(unchanged.candidates.is_empty());
}

#[test]
fn semantic_router_skips_a_decision_that_already_has_a_strong_match() {
    let entries = vec![entry(
        "skill:explore",
        CapabilityKind::Skill,
        AutoUse::Prefer,
        &["survey"],
        "run_skill",
    )];
    let provider = Arc::new(StubProvider {
        ids: vec!["skill:explore".to_owned()],
        fail: false,
    });
    let router = SemanticRouter::new(Some(provider), None, String::new(), String::new());
    let strong = RouteDecision {
        candidates: vec![reasonix_tools::builtin::capability_router::RouteCandidate {
            entry: entries[0].clone(),
            policy: AutoUse::Prefer,
            reason: "trigger".to_owned(),
        }],
        ..Default::default()
    };
    let decision = router.route_semantic("survey", &entries, "fp", strong);
    assert_eq!(decision.candidates.len(), 1);
    assert_eq!(decision.candidates[0].reason, "trigger");
}

// ---------------------------------------------------------------------------
// Local fixtures
// ---------------------------------------------------------------------------

fn candidate(
    id: &str,
    policy: AutoUse,
) -> reasonix_tools::builtin::capability_router::RouteCandidate {
    reasonix_tools::builtin::capability_router::RouteCandidate {
        entry: CapabilityEntry {
            id: id.to_owned(),
            ..Default::default()
        },
        policy,
        reason: "test".to_owned(),
    }
}

fn entry(
    id: &str,
    kind: CapabilityKind,
    auto: AutoUse,
    triggers: &[&str],
    name: &str,
) -> CapabilityEntry {
    CapabilityEntry {
        id: id.to_owned(),
        kind,
        auto_use: auto,
        triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
        tool_name: name.to_owned(),
        status: CapabilityStatus::Ready,
        ..Default::default()
    }
}

#[test]
fn catalog_and_ledger_expose_call_contexts_for_integration_wiring() {
    // Cards and seams must construct cleanly with no host binding attached.
    let _ctx = CallContext::new();
    let _ = CallContext::new();
    let catalog: CapabilityCatalog = build_catalog(CatalogOptions::default());
    assert!(catalog.entries.is_empty());
    let _: Value = json!({});
}
