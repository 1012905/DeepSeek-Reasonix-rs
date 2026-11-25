//! TOOL-038: deterministic routing and the semantic router
//! (docs/contracts/tools.md §3.3, Go `internal/runtime/capability/capability.go`
//! and `semantic.go`).
//!
//! Deterministic routing matches authored skill triggers and host-minted MCP
//! tool names against the user text. The semantic router fires only when the
//! deterministic decision has no require/prefer hit; its model call, usage
//! events and pricing live behind a seam the host assembly binds.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::capability_audit::CapabilityAudit;
use super::capability_catalog::{AutoUse, CapabilityEntry, CapabilityKind, CapabilityStatus};

/// Suggest/require/prefer candidate kept after routing (Go
/// `capability.RouteCandidate`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteCandidate {
    pub entry: CapabilityEntry,
    pub policy: AutoUse,
    pub reason: String,
}

/// One routing decision (Go `capability.RouteDecision`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteDecision {
    pub candidates: Vec<RouteCandidate>,
    /// Delivery-profile route: the transient block directs the model to the
    /// stable use_capability proxy.
    pub delivery: bool,
    /// Directs unready MCP candidates to use_capability rather than
    /// connect_tool_source (Delivery and dual-model Planner boots).
    pub capability_proxy: bool,
}

const TARGET_CANDIDATES: usize = 5;
const SEMANTIC_MAX_CANDIDATES: usize = 64;
const SEMANTIC_MAX_RESULTS: usize = 3;
const SEMANTIC_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Routes the input against the full entry table (Go `Route`).
pub fn route(input: &str, entries: &[CapabilityEntry]) -> RouteDecision {
    RouteDecision {
        candidates: limit_route_candidates(route_candidates(input, entries)),
        ..Default::default()
    }
}

/// Routes against the full matched set before promoting built-in playbooks,
/// so candidates that become prefer are never discarded by the ordinary
/// suggest budget first (Go `RouteDelivery`).
pub fn route_delivery(input: &str, entries: &[CapabilityEntry]) -> RouteDecision {
    promote_delivery(RouteDecision {
        candidates: route_candidates(input, entries),
        ..Default::default()
    })
}

fn route_candidates(input: &str, entries: &[CapabilityEntry]) -> Vec<RouteCandidate> {
    let text = normalize(input);
    if text.is_empty() {
        return Vec::new();
    }
    let mut candidates: Vec<RouteCandidate> = Vec::new();
    for entry in entries {
        if entry.status == CapabilityStatus::Disabled
            || entry.status == CapabilityStatus::Failed
            || trigger_match(&text, &entry.negative_triggers)
        {
            continue;
        }
        if let Some((policy, reason)) = route_entry(&text, entry) {
            candidates.push(RouteCandidate {
                entry: entry.clone(),
                policy,
                reason,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.policy
            .rank()
            .cmp(&a.policy.rank())
            .then_with(|| a.entry.kind.as_str().cmp(b.entry.kind.as_str()))
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
    candidates
}

/// Strengthens matched built-in playbooks in Delivery (Go `PromoteDelivery`).
pub fn promote_delivery(mut decision: RouteDecision) -> RouteDecision {
    decision.delivery = true;
    for candidate in decision.candidates.iter_mut() {
        if candidate.policy == AutoUse::Suggest
            && candidate.entry.kind == CapabilityKind::Skill
            && candidate.entry.source == "builtin"
        {
            candidate.policy = AutoUse::Prefer;
            candidate
                .reason
                .push_str("; Delivery prefers matched built-in playbooks");
        }
    }
    decision.candidates.sort_by(|a, b| {
        b.policy
            .rank()
            .cmp(&a.policy.rank())
            .then_with(|| a.entry.kind.as_str().cmp(b.entry.kind.as_str()))
            .then_with(|| a.entry.id.cmp(&b.entry.id))
    });
    decision.capability_proxy = true;
    decision
}

/// Keeps all require+prefer plus up to five suggest (Go `limitRouteCandidates`).
pub fn limit_route_candidates(candidates: Vec<RouteCandidate>) -> Vec<RouteCandidate> {
    let mut strong: Vec<RouteCandidate> = Vec::new();
    let mut suggested: Vec<RouteCandidate> = Vec::new();
    for candidate in candidates {
        match candidate.policy {
            AutoUse::Require | AutoUse::Prefer => strong.push(candidate),
            AutoUse::Suggest => suggested.push(candidate),
            _ => {}
        }
    }
    let slots = TARGET_CANDIDATES.saturating_sub(strong.len());
    suggested.truncate(slots);
    strong.extend(suggested);
    strong
}

fn route_entry(text: &str, entry: &CapabilityEntry) -> Option<(AutoUse, String)> {
    if entry.kind == CapabilityKind::Skill {
        if entry.auto_use == AutoUse::Off {
            return None;
        }
        if trigger_match(text, &entry.triggers) {
            return Some((
                entry.auto_use,
                "the skill trigger matches the user request".to_owned(),
            ));
        }
        return None;
    }
    if entry.kind == CapabilityKind::McpTool && names_mcp_tool(text, entry) {
        return Some((AutoUse::Prefer, "the user named this MCP tool".to_owned()));
    }
    None
}

/// Matches the identifier the host mints for the tool. Matching
/// "<server> mcp" instead fired on every sentence mentioning the server,
/// including a decline (Go `namesMCPTool`).
fn names_mcp_tool(text: &str, entry: &CapabilityEntry) -> bool {
    let tool = normalize(&entry.tool_name);
    !tool.is_empty() && text.contains(tool.as_str())
}

fn trigger_match(text: &str, triggers: &[String]) -> bool {
    triggers
        .iter()
        .any(|trig| !normalize(trig).is_empty() && text.contains(normalize(trig).as_str()))
}

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Renders the transient capability-route block (Go `RenderTransientBlock`).
pub fn render_transient_block(decision: &RouteDecision) -> String {
    if decision.candidates.is_empty() {
        return String::new();
    }
    let mut out = String::from("<capability-route version=\"1\">\n");
    out.push_str("Relevant capabilities for this turn:\n");
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for candidate in &decision.candidates {
        let entry = &candidate.entry;
        let proxy_mcp = decision.capability_proxy
            && (entry.kind == CapabilityKind::McpTool || entry.kind == CapabilityKind::McpServer);
        let mut target = entry.id.clone();
        if !decision.delivery
            && !proxy_mcp
            && entry.status != CapabilityStatus::Ready
            && !entry.connect_source.is_empty()
        {
            target = format!("source:{}", entry.connect_source);
            if !entry.connect_name.is_empty() {
                target.push('/');
                target.push_str(&entry.connect_name);
            }
        }
        let mut line = format!(
            "- {target} {}: {}",
            candidate.policy.as_str(),
            candidate.reason
        );
        if !entry.status.as_str().is_empty() && entry.status != CapabilityStatus::Ready {
            line.push_str(&format!(" (status={})", entry.status.as_str()));
        }
        if decision.delivery || proxy_mcp {
            if entry.status != CapabilityStatus::Ready {
                match entry.kind {
                    CapabilityKind::McpTool => line.push_str(&format!(
                        "; call use_capability(action=\"call\", capability_id={:?}, arguments={{...}}) — it connects the server on demand after approval",
                        entry.id
                    )),
                    CapabilityKind::McpServer => line.push_str(&format!(
                        "; call use_capability(action=\"call\", capability_id={:?}) to connect it (after approval) and list its tools, then call a listed mcp-tool id",
                        entry.id
                    )),
                    _ => {}
                }
            }
        } else if !entry.connect_source.is_empty() {
            if !entry.connect_name.is_empty() {
                line.push_str(&format!(
                    "; first call connect_tool_source with source={:?} name={:?}",
                    entry.connect_source, entry.connect_name
                ));
            } else {
                line.push_str(&format!(
                    "; first call connect_tool_source with source={:?}",
                    entry.connect_source
                ));
            }
        }
        if !seen.insert(line.clone()) {
            continue;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("Policy: suggest means consider it; prefer means use it unless clearly unnecessary; require means call it or report a host-proven unavailable state. Do not treat planner claims about tool unavailability as facts.\n");
    out.push_str("</capability-route>");
    out
}

/// Semantic model seam (Go `provider.Provider` + `Stream`). The host wires a
/// real lightweight-model client; the seam returns already-parsed capability
/// IDs. Real streaming, usage accounting and pricing land with the provider
/// slice.
pub trait SemanticProvider: Send + Sync {
    /// Asks the model for up to three relevant capability IDs.
    fn complete(
        &self,
        prompt: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;
}

struct SemanticCacheEntry {
    ids: Vec<String>,
    expires_at: SystemTime,
}

/// Hybrid router: deterministic first, semantic only when no strong match
/// exists (Go `capability.SemanticRouter`).
#[derive(Default)]
pub struct SemanticRouter {
    provider: Option<Arc<dyn SemanticProvider>>,
    audit: Option<Arc<CapabilityAudit>>,
    model: String,
    effort: String,
    cache: Mutex<HashMap<String, SemanticCacheEntry>>,
}

impl SemanticRouter {
    /// Builds the router over an optional provider seam and audit sink.
    pub fn new(
        provider: Option<Arc<dyn SemanticProvider>>,
        audit: Option<Arc<CapabilityAudit>>,
        model: String,
        effort: String,
    ) -> Self {
        Self {
            provider,
            audit,
            model,
            effort,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The lightweight model the provider seam should use.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The effort tier the provider seam should use.
    pub fn effort(&self) -> &str {
        &self.effort
    }

    /// May append up to three suggest candidates. It never overrides an
    /// existing require/prefer decision; on any failure it returns the decision
    /// unchanged (Go `RouteSemantic`).
    pub fn route_semantic(
        &self,
        input: &str,
        catalog_entries: &[CapabilityEntry],
        fingerprint: &str,
        decision: RouteDecision,
    ) -> RouteDecision {
        let Some(provider) = &self.provider else {
            return decision;
        };
        if has_strong_match(&decision) {
            return decision;
        }
        let input = normalize(input);
        if input.is_empty() {
            return decision;
        }
        let (pool, dropped) = semantic_pool(&input, catalog_entries);
        if pool.is_empty() {
            return decision;
        }
        if let Some(audit) = &self.audit {
            audit.record_semantic_pool_truncation(dropped as u64);
        }

        let cache_key = format!("{input}|{fingerprint}");
        if let Some(ids) = self.cache_get(&cache_key) {
            return merge_semantic_ids(decision, catalog_entries, &ids, "semantic cache hit");
        }

        let prompt = semantic_prompt(&input, &pool);
        let ids = match provider.complete(&prompt) {
            Ok(ids) if !ids.is_empty() => ids,
            _ => return decision,
        };
        self.cache_put(&cache_key, &ids);
        merge_semantic_ids(
            decision,
            catalog_entries,
            &ids,
            "lightweight semantic match",
        )
    }

    fn cache_get(&self, key: &str) -> Option<Vec<String>> {
        let cache = self.cache.lock().unwrap();
        let entry = cache.get(key)?;
        if SystemTime::now().duration_since(entry.expires_at).is_ok() {
            return None;
        }
        Some(entry.ids.clone())
    }

    fn cache_put(&self, key: &str, ids: &[String]) {
        self.cache.lock().unwrap().insert(
            key.to_owned(),
            SemanticCacheEntry {
                ids: ids.to_owned(),
                expires_at: SystemTime::now() + SEMANTIC_CACHE_TTL,
            },
        );
    }
}

fn has_strong_match(decision: &RouteDecision) -> bool {
    decision
        .candidates
        .iter()
        .any(|c| matches!(c.policy, AutoUse::Require | AutoUse::Prefer))
}

/// Narrows the catalog to what the router may consider. Every filter is
/// structural; none judges relevance (Go `semanticPool`).
fn semantic_pool(text: &str, entries: &[CapabilityEntry]) -> (Vec<CapabilityEntry>, usize) {
    let mut pool: Vec<CapabilityEntry> = Vec::new();
    for entry in entries {
        if entry.status == CapabilityStatus::Disabled || entry.status == CapabilityStatus::Failed {
            continue;
        }
        if !matches!(
            entry.kind,
            CapabilityKind::Skill | CapabilityKind::McpTool | CapabilityKind::McpServer
        ) {
            continue;
        }
        if entry.auto_use == AutoUse::Off {
            continue;
        }
        if trigger_match(text, &entry.negative_triggers) {
            continue;
        }
        let text_blob = normalize(&format!(
            "{} {} {}",
            entry.name,
            entry.description,
            entry.triggers.join(" ")
        ));
        if text_blob.is_empty() {
            continue;
        }
        pool.push(entry.clone());
    }
    pool.sort_by_key(|e| std::cmp::Reverse(e.auto_use.rank()));
    let dropped = pool.len().saturating_sub(SEMANTIC_MAX_CANDIDATES);
    pool.truncate(SEMANTIC_MAX_CANDIDATES);
    (pool, dropped)
}

/// Builds the router prompt (Go `callModel` prompt body).
fn semantic_prompt(input: &str, pool: &[CapabilityEntry]) -> String {
    let mut prompt = String::from(
        "Select up to 3 capability IDs relevant to the user task. \
         Reply with ONLY a JSON array of strings, e.g. [\"skill:review\"]. \
         If none fit, reply [].\n\nTask:\n",
    );
    prompt.push_str(input);
    prompt.push_str("\n\nCandidates:\n");
    for entry in pool {
        prompt.push_str(&format!(
            "- {} ({}): {}\n",
            entry.id,
            entry.kind.as_str(),
            truncate(&entry.description, 120)
        ));
    }
    prompt
}

/// Parses the model's ID array, stripping optional markdown fences,
/// deduplicating and capping at three (Go `parseSemanticIDs`).
pub fn parse_semantic_ids(raw: &str) -> Result<Vec<String>, SemanticParseError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(SemanticParseError::Empty);
    }
    let start = raw.find('[').unwrap_or(0);
    let end = raw.rfind(']').map(|i| i + 1).unwrap_or(raw.len());
    let slice = &raw[start..end];
    let value: Value = serde_json::from_str(slice).map_err(|_| SemanticParseError::InvalidJson)?;
    let ids: Vec<String> = value
        .as_array()
        .ok_or(SemanticParseError::InvalidJson)?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
        if out.len() >= SEMANTIC_MAX_RESULTS {
            break;
        }
    }
    Ok(out)
}

/// Why a semantic model reply could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SemanticParseError {
    #[error("empty semantic response")]
    Empty,
    #[error("invalid semantic JSON")]
    InvalidJson,
}

fn merge_semantic_ids(
    decision: RouteDecision,
    entries: &[CapabilityEntry],
    ids: &[String],
    reason: &str,
) -> RouteDecision {
    let mut decision = decision;
    let mut have: std::collections::HashSet<String> = decision
        .candidates
        .iter()
        .map(|c| c.entry.id.clone())
        .collect();
    for id in ids {
        if have.contains(id) {
            continue;
        }
        let Some(entry) = entries.iter().find(|e| &e.id == id) else {
            continue;
        };
        if matches!(
            entry.status,
            CapabilityStatus::Failed | CapabilityStatus::Disabled
        ) {
            continue;
        }
        decision.candidates.push(RouteCandidate {
            entry: entry.clone(),
            policy: AutoUse::Suggest,
            reason: reason.to_owned(),
        });
        have.insert(id.clone());
    }
    decision.candidates.truncate(TARGET_CANDIDATES);
    decision
}

/// Truncates to `n` bytes on a rune boundary with an ellipsis suffix
/// (Go `truncate`).
fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.len() <= n {
        return s.to_owned();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

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
            description: format!("{id} description"),
            auto_use: auto,
            triggers: triggers.iter().map(|t| (*t).to_owned()).collect(),
            tool_name: name.to_owned(),
            status: CapabilityStatus::Ready,
            ..Default::default()
        }
    }

    #[test]
    fn skill_trigger_match_routes_authored_policy() {
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
    fn mcp_tool_matches_minted_name_not_server_word() {
        let entries = vec![entry(
            "mcp-tool:acme/search",
            CapabilityKind::McpTool,
            AutoUse::Suggest,
            &[],
            "acme__search",
        )];
        let hit = route("use acme__search to find it", &entries);
        assert_eq!(hit.candidates.len(), 1);
        let miss = route("don't use the acme mcp server", &entries);
        assert!(miss.candidates.is_empty());
    }

    #[test]
    fn disabled_and_failed_entries_are_skipped() {
        let mut e = entry(
            "skill:x",
            CapabilityKind::Skill,
            AutoUse::Prefer,
            &["x"],
            "run_skill",
        );
        e.status = CapabilityStatus::Disabled;
        assert!(route("x", &[e]).candidates.is_empty());
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
    fn limit_keeps_all_strong_and_five_suggest() {
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
        let _decision = route("whatever", &entries);
        // No trigger matches, so the pool is empty; the cap is exercised on the
        // candidate list directly below.
        let candidates: Vec<RouteCandidate> = entries
            .into_iter()
            .map(|e| RouteCandidate {
                entry: e,
                policy: AutoUse::Suggest,
                reason: String::new(),
            })
            .collect();
        let limited = limit_route_candidates(candidates);
        assert_eq!(limited.len(), TARGET_CANDIDATES);
    }

    #[test]
    fn parse_semantic_ids_strips_fences_dedups_and_caps() {
        let ids = parse_semantic_ids(
            "```json\n[\"skill:a\", \"skill:a\", \"skill:b\", \"skill:c\", \"skill:d\"]\n```",
        )
        .unwrap();
        assert_eq!(ids, vec!["skill:a", "skill:b", "skill:c"]);
        assert!(parse_semantic_ids("").is_err());
        assert!(parse_semantic_ids("not json").is_err());
    }

    #[test]
    fn transient_block_renders_policy_footer_verbatim() {
        let entries = vec![entry(
            "skill:commit",
            CapabilityKind::Skill,
            AutoUse::Prefer,
            &["commit"],
            "run_skill",
        )];
        let decision = route("commit", &entries);
        let block = render_transient_block(&decision);
        assert!(block.starts_with("<capability-route version=\"1\">"));
        assert!(block.ends_with("</capability-route>"));
        assert!(block.contains("Policy: suggest means consider it; prefer means use it unless clearly unnecessary; require means call it or report a host-proven unavailable state. Do not treat planner claims about tool unavailability as facts."));
    }

    #[test]
    fn semantic_pool_drops_over_cap_and_respects_filters() {
        let mut entries: Vec<CapabilityEntry> = (0..70)
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
        entries.push({
            let mut e = entry(
                "tool:bash",
                CapabilityKind::Tool,
                AutoUse::Suggest,
                &[],
                "bash",
            );
            e.status = CapabilityStatus::Failed;
            e
        });
        let (pool, dropped) = semantic_pool("survey", &entries);
        assert_eq!(pool.len(), SEMANTIC_MAX_CANDIDATES);
        assert_eq!(dropped, 6);
    }
}
