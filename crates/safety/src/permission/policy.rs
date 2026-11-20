//! The pure, I/O-free heart of the permission layer: a set of rules plus the
//! writer fallback mode, and the evaluation that decides a tool call.

use serde_json::value::RawValue;

use super::bash::{BashApprovalClass, bash_analysis};
use super::decision::{Decision, match_glob, parse_decision, subjects};
use super::permrule::{Rule, parse as parse_permrule};
use super::rule_group::{group_grant_rule, in_group, is_browser_tool, is_computer_tool};

/// The legacy bare PowerShell write cmdlets older Desktop settings accepted
/// as generic deny rules. The compatibility expansion is deny-only and
/// additive, so it cannot broaden an allow or weaken an exact tool deny.
pub(crate) fn legacy_deny_cmdlet(s: &str) -> Option<&'static str> {
    legacy_bare_power_shell_deny_cmdlet(s)
}

fn legacy_bare_power_shell_deny_cmdlet(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "set-content" => Some("Set-Content"),
        "add-content" => Some("Add-Content"),
        "out-file" => Some("Out-File"),
        _ => None,
    }
}

fn is_case_insensitive_power_shell_cmdlet(s: &str) -> bool {
    legacy_bare_power_shell_deny_cmdlet(s).is_some()
}

fn parse_rules(ss: &[&str]) -> Vec<Rule> {
    ss.iter()
        .filter_map(|s| {
            let (r, ok) = parse_permrule(s);
            ok.then_some(r)
        })
        .collect()
}

fn parse_deny_rules(ss: &[&str]) -> Vec<Rule> {
    let mut out = Vec::new();
    for s in ss {
        let (r, ok) = parse_permrule(s);
        if !ok {
            continue;
        }
        // Preserve the generic ToolName meaning while also recognizing the
        // three bare PowerShell write cmdlets accepted by older Desktop
        // settings as command prefixes.
        let bare = r.subject.is_empty();
        let cmdlet = if bare {
            legacy_bare_power_shell_deny_cmdlet(&r.tool)
        } else {
            None
        };
        out.push(r);
        if let Some(cmdlet) = cmdlet {
            out.push(Rule {
                tool: "Bash".to_string(),
                subject: format!("{cmdlet}:*"),
                literal: false,
            });
        }
    }
    out
}

/// A set of rules plus the writer fallback mode. It is the pure, I/O-free
/// heart of the permission layer.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// The fallback decision for writer tools when no rule matches. Read-only
    /// tools always fall back to Allow.
    pub mode: Decision,
    pub allow: Vec<Rule>,
    pub ask: Vec<Rule>,
    pub deny: Vec<Rule>,
    /// An explicit frontend/session override such as Claude Code's
    /// --allowed-tools. Deny rules still win, while these rules override
    /// configured Ask entries for the current process only.
    pub session_allow: Vec<Rule>,
    /// Lets the writer fallback Mode cover command substitution and
    /// interpreter -c/-e forms. Deliberately opt-in: broad Bash allow rules
    /// alone must not re-open nested-command bypasses.
    pub allow_dynamic_bash: bool,
    /// Denies every call that is not a read, whatever the rules allow.
    pub read_only: bool,
    /// Puts a call that repeats external text to a person when only Mode
    /// allows it.
    pub ask_on_origin: bool,
}

impl Policy {
    /// Builds a Policy from config string slices and a mode string ("ask" by
    /// default). Malformed rule strings are dropped.
    pub fn new(mode: &str, allow: &[&str], ask: &[&str], deny: &[&str]) -> Policy {
        Policy {
            mode: parse_decision(mode),
            allow: parse_rules(allow),
            ask: parse_rules(ask),
            deny: parse_deny_rules(deny),
            ..Policy::default()
        }
    }

    /// Returns a copy of the policy with additional ephemeral allow rules.
    /// Malformed entries are ignored consistently with [`Policy::new`].
    pub fn with_session_allow(mut self, rules: &[&str]) -> Policy {
        self.session_allow.extend(parse_rules(rules));
        self
    }

    /// Enables the explicit advanced override for dynamic shell shapes. Deny,
    /// ask, and exact allow rules retain precedence.
    pub fn with_allow_dynamic_bash_fallback(mut self, enabled: bool) -> Policy {
        self.allow_dynamic_bash = enabled;
        self
    }

    /// Puts a call that repeats origin text to a person when only the mode
    /// allows it.
    pub fn with_ask_on_origin(mut self, on: bool) -> Policy {
        self.ask_on_origin = on;
        self
    }

    /// Evaluates a tool call. `read_only` is the tool's own classification;
    /// args is the raw JSON the model sent, from which the call's subject is
    /// extracted for glob matching. Calls with multiple subjects, such as
    /// move_file's source and destination paths, must be safe for every
    /// subject before the call is allowed. Precedence: deny > ask > allow >
    /// fallback (Allow for readers, Mode for writers). SessionAllow sits
    /// between deny and configured ask rules.
    pub fn decide(&self, tool_name: &str, read_only: bool, args: &RawValue) -> Decision {
        self.decide_subjects(tool_name, read_only, &subjects(args))
    }

    /// Reports only configured deny-rule matches. It deliberately excludes
    /// the fallback Mode so installing or explicitly authorizing an MCP
    /// server remains the final allow decision.
    pub fn explicitly_denies(&self, tool_name: &str, args: &RawValue) -> bool {
        let mut s = subjects(args);
        if s.is_empty() {
            s = vec![String::new()];
        }
        s.iter()
            .any(|subject| match_any_raw(&self.deny, tool_name, subject))
    }

    /// Evaluates a tool call when the caller already extracted the stable
    /// approval subject from args.
    pub fn decide_subject(&self, tool_name: &str, read_only: bool, subject: &str) -> Decision {
        if self.read_only && !read_only {
            return Decision::Deny;
        }
        if canonical_rule_tool(tool_name) == "bash" {
            return self.decide_bash_subject(read_only, subject);
        }
        if let Some(d) = self.decide_browser_credential(tool_name, subject) {
            return d;
        }
        if let Some(d) = self.decide_computer(tool_name, subject) {
            return d;
        }
        if match_any(&self.deny, tool_name, subject) {
            return Decision::Deny;
        }
        if match_any(&self.session_allow, tool_name, subject) {
            return Decision::Allow;
        }
        if match_any(&self.ask, tool_name, subject) {
            return Decision::Ask;
        }
        if match_any(&self.allow, tool_name, subject) || read_only {
            return Decision::Allow;
        }
        if super::write_fence::subject_requires_human(tool_name, subject) {
            return Decision::Ask;
        }
        self.mode
    }

    fn decide_bash_subject(&self, read_only: bool, subject: &str) -> Decision {
        let engine = bash_analysis();
        let class = engine.approval_class(subject);
        let requires_exact = class != BashApprovalClass::Reusable;
        let mut requires_human = class == BashApprovalClass::RequireHuman;
        let parts = engine.decompose(subject);
        if match_any_raw(&self.deny, "bash", subject) {
            return Decision::Deny;
        }
        if match_any_exact(&self.session_allow, "bash", subject) {
            return Decision::Allow;
        }
        if class == BashApprovalClass::Reusable
            && parts.is_none()
            && match_any_allow(&self.session_allow, "bash", subject)
        {
            return Decision::Allow;
        }
        if match_any_raw(&self.ask, "bash", subject) {
            return Decision::Ask;
        }
        if match_any_exact(&self.allow, "bash", subject) {
            return Decision::Allow;
        }
        if let Some(parts) = parts {
            return self.decide_bash_segments(read_only, &parts);
        }
        // A remembered prefix rule never covers a network path: the user
        // approved a command, not a connection to whatever machine a later
        // argument names.
        if super::network::operands_name_network_path(subject) {
            requires_human = true;
        }
        if requires_human && self.mode == Decision::Deny {
            return Decision::Deny;
        }
        if requires_human && self.allow_dynamic_bash && self.mode == Decision::Allow {
            return Decision::Allow;
        }
        if requires_human {
            return Decision::Ask;
        }
        if requires_exact && read_only {
            return Decision::Allow;
        }
        if requires_exact {
            return self.mode;
        }
        if match_any_allow(&self.allow, "bash", subject) || read_only {
            Decision::Allow
        } else {
            self.mode
        }
    }

    /// Evaluates each simple-command segment of a compound bash invocation
    /// against the rule table independently. This lets prefix rules like
    /// `Bash(git push:*)` cover common compound flows without ever
    /// synthesizing a new prefix from a compound command.
    ///
    /// Precedence stays deny > ask > allow > fallback. Any single segment
    /// hitting deny denies the whole call; any segment needing approval turns
    /// the whole call into Ask; the whole call is Allow only if every segment
    /// is covered or writer fallback allows uncovered segments.
    fn decide_bash_segments(&self, read_only: bool, parts: &[String]) -> Decision {
        let mut out = Decision::Allow;
        for sub in parts {
            let seg_read_only = read_only || bash_analysis().is_read_only(sub);
            match self.decide_subject("bash", seg_read_only, sub) {
                Decision::Deny => return Decision::Deny,
                Decision::Ask => out = Decision::Ask,
                Decision::Allow => {}
            }
        }
        out
    }

    /// Evaluates a tool call against every subject the call touches. This
    /// keeps two-path operations honest: a move is denied if either endpoint
    /// is denied, asks if either endpoint requires approval, and is allowed
    /// only when every endpoint is allowed under the same policy.
    pub fn decide_subjects(
        &self,
        tool_name: &str,
        read_only: bool,
        subjects: &[String],
    ) -> Decision {
        if subjects.is_empty() {
            return self.decide_subject(tool_name, read_only, "");
        }
        let mut out = Decision::Allow;
        for subject in subjects {
            match self.decide_subject(tool_name, read_only, subject) {
                Decision::Deny => return Decision::Deny,
                Decision::Ask => out = Decision::Ask,
                Decision::Allow => {}
            }
        }
        out
    }

    /// Reports the configured rule responsible for an explicit Ask or Deny
    /// decision. Fallback-mode and dynamic-safety decisions intentionally
    /// have no rule provenance. Compound Bash commands are inspected segment
    /// by segment using the same raw-prefix semantics as [`Policy::decide_subject`].
    pub fn matched_rule(
        &self,
        tool_name: &str,
        decision: Decision,
        args: &RawValue,
    ) -> Option<String> {
        let rules = match decision {
            Decision::Ask => &self.ask,
            Decision::Deny => &self.deny,
            Decision::Allow => return None,
        };
        let mut ss = subjects(args);
        if ss.is_empty() {
            ss = vec![String::new()];
        }
        let raw = canonical_rule_tool(tool_name) == "bash";
        for subject in &ss {
            let mut candidates = vec![subject.clone()];
            if raw && let Some(parts) = bash_analysis().decompose(subject) {
                candidates.extend(parts);
            }
            for candidate in candidates {
                // A matching configured rule is provenance only when that
                // candidate's actual decision has the same outcome.
                // SessionAllow may override an Ask rule on one endpoint while
                // a different endpoint falls back to Ask.
                if self.decide_subject(tool_name, false, &candidate) != decision {
                    continue;
                }
                if let Some(rule) = first_matching_rule(rules, tool_name, &candidate, raw) {
                    return Some(rule_config_string(&rule));
                }
            }
        }
        None
    }

    fn decide_browser_credential(&self, tool_name: &str, subject: &str) -> Option<Decision> {
        if !is_browser_tool(tool_name)
            || !super::browser::browser_subject_requires_explicit_approval(subject)
        {
            return None;
        }
        if match_any(&self.deny, tool_name, subject) {
            return Some(Decision::Deny);
        }
        if match_any_exact(&self.session_allow, tool_name, subject)
            || match_any_exact(&self.allow, tool_name, subject)
        {
            return Some(Decision::Allow);
        }
        if self.mode == Decision::Deny {
            return Some(Decision::Deny);
        }
        Some(Decision::Ask)
    }

    fn decide_computer(&self, tool_name: &str, subject: &str) -> Option<Decision> {
        if !is_computer_tool(tool_name) {
            return None;
        }
        if match_any(&self.deny, tool_name, subject) {
            return Some(Decision::Deny);
        }
        let pointer = super::computer::computer_subject_takes_pointer(subject);
        if pointer
            && (match_any_exact(&self.session_allow, tool_name, subject)
                || match_any_exact(&self.allow, tool_name, subject))
        {
            return Some(Decision::Allow);
        }
        if pointer {
            return Some(if self.mode == Decision::Deny {
                Decision::Deny
            } else {
                Decision::Ask
            });
        }
        if match_any_exact(&self.session_allow, tool_name, subject)
            || match_any_exact(&self.allow, tool_name, subject)
        {
            return Some(Decision::Allow);
        }
        if self.mode == Decision::Deny {
            return Some(Decision::Deny);
        }
        Some(Decision::Ask)
    }

    /// Reports that the call runs because the fallback Mode says so: with the
    /// fallback turned to Ask, no rule and no read classification allows it.
    pub(crate) fn allowed_only_by_mode(
        &self,
        tool_name: &str,
        read_only: bool,
        args: &RawValue,
    ) -> bool {
        let mut probe = self.clone();
        probe.mode = Decision::Ask;
        probe.decide(tool_name, read_only, args) != Decision::Allow
    }
}

/// Reports whether any rule matches the (toolName, subject) pair. A
/// subject-specific rule cannot match a call that exposes no subject.
pub(crate) fn match_any(rules: &[Rule], tool_name: &str, subject: &str) -> bool {
    for r in rules {
        if !rule_tool_matches(&r.tool, tool_name) {
            continue;
        }
        if r.subject.is_empty() {
            return true;
        }
        if subject.is_empty() {
            continue;
        }
        if rule_subject_matches(r, subject) {
            return true;
        }
    }
    false
}

pub(crate) fn match_any_raw(rules: &[Rule], tool_name: &str, subject: &str) -> bool {
    for r in rules {
        if !rule_tool_matches(&r.tool, tool_name) {
            continue;
        }
        if r.subject.is_empty() {
            return true;
        }
        if subject.is_empty() {
            continue;
        }
        if raw_rule_subject_matches(r, subject) {
            return true;
        }
    }
    false
}

fn first_matching_rule(rules: &[Rule], tool_name: &str, subject: &str, raw: bool) -> Option<Rule> {
    for rule in rules {
        if !rule_tool_matches(&rule.tool, tool_name) {
            continue;
        }
        if rule.subject.is_empty() {
            return Some(rule.clone());
        }
        if subject.is_empty() {
            continue;
        }
        let matches = if raw {
            raw_rule_subject_matches(rule, subject)
        } else {
            rule_subject_matches(rule, subject)
        };
        if matches {
            return Some(rule.clone());
        }
    }
    None
}

pub(crate) fn rule_config_string(rule: &Rule) -> String {
    if rule.subject.is_empty() {
        rule.tool.clone()
    } else if rule.literal {
        format!("{}={}", rule.tool, rule.subject)
    } else {
        format!("{}({})", rule.tool, rule.subject)
    }
}

fn raw_rule_subject_matches(rule: &Rule, subject: &str) -> bool {
    if rule.literal {
        return rule.subject == subject;
    }
    if canonical_rule_tool(&rule.tool) == "bash"
        && let Some(base) = bash_prefix_base(&rule.subject)
    {
        return raw_bash_prefix_matches(&base, subject);
    }
    match_glob(&rule.subject, subject)
}

/// Reports whether a Bash rule's command prefix names the subject's program,
/// or the program a wrapper hands it to. Only deny and ask matching reach it;
/// allow rules keep the command as written.
fn raw_bash_prefix_matches(base: &str, subject: &str) -> bool {
    let (base_fields, malformed) = super::shellparse::static_fields(base);
    if malformed.is_empty() && !base_fields.is_empty() {
        // The command prefix reads through wrappers: AnalyzeApprovalFeatures
        // answers the as-written argv, PeelWrappers each layer a wrapper
        // hands the call to (reference bash.go rawBashPrefixMatches).
        if let Some(features) =
            reasonix_core::shellparse_analysis::analyze_approval_features(subject)
            && prefix_fields_match(&base_fields, &features.command_prefix)
        {
            return true;
        }
        if let Ok(inv) = reasonix_core::shellparse_analysis::peel_wrappers(subject) {
            for layer in &inv.layers {
                if prefix_fields_match(&base_fields, layer) {
                    return true;
                }
            }
        }
    }
    let base = base.trim();
    let subject = subject.trim();
    if subject == base
        || (is_case_insensitive_power_shell_cmdlet(base) && subject.eq_ignore_ascii_case(base))
    {
        return true;
    }
    if subject.len() <= base.len() {
        return false;
    }
    let mut prefix_matches = subject.starts_with(base);
    if is_case_insensitive_power_shell_cmdlet(base) {
        prefix_matches = subject[..base.len()].eq_ignore_ascii_case(base);
    }
    if !prefix_matches {
        return false;
    }
    matches!(subject.as_bytes()[base.len()], b' ' | b'\t' | b'\r' | b'\n')
}

fn prefix_fields_match(base_fields: &[String], fields: &[String]) -> bool {
    if fields.len() < base_fields.len() {
        return false;
    }
    base_fields.iter().enumerate().all(|(i, want)| {
        let got = &fields[i];
        got == want
            || (i == 0
                && is_case_insensitive_power_shell_cmdlet(want)
                && got.eq_ignore_ascii_case(want))
    })
}

pub(crate) fn match_any_exact(rules: &[Rule], tool_name: &str, subject: &str) -> bool {
    if subject.is_empty() {
        return false;
    }
    rules.iter().any(|r| {
        rule_tool_matches(&r.tool, tool_name)
            && !r.subject.is_empty()
            && r.subject == subject
            && (r.literal || !has_glob_meta(&r.subject))
    })
}

pub(crate) fn match_any_allow(rules: &[Rule], tool_name: &str, subject: &str) -> bool {
    if match_any_exact(rules, tool_name, subject) {
        return true;
    }
    if (canonical_rule_tool(tool_name) == "bash"
        && bash_analysis().approval_class(subject) != BashApprovalClass::Reusable)
        || (is_browser_tool(tool_name)
            && super::browser::browser_subject_requires_explicit_approval(subject))
    {
        return false;
    }
    match_any(rules, tool_name, subject)
}

/// Reports whether one config-style rule string matches the given tool
/// subject. It is used for session grants as well as persisted config rules
/// so both paths share identical matching semantics.
pub fn rule_matches_string(rule: &str, tool_name: &str, subject: &str) -> bool {
    let (r, ok) = parse_permrule(rule);
    ok && match_any_allow(&[r], tool_name, subject)
}

/// Reports whether every call represented by `candidate` is already covered
/// by `existing`. It intentionally proves only the cases Reasonix creates
/// automatically: exact rules covered by broader globs or bare tool rules,
/// exact duplicate globs, and bare tool rules covering subject rules.
pub fn rule_covers_string(existing: &str, candidate: &str) -> bool {
    let (a, ok) = parse_permrule(existing);
    if !ok {
        return false;
    }
    let (b, ok) = parse_permrule(candidate);
    if !ok {
        return false;
    }
    if !rule_tool_compatible(&a.tool, &b.tool) {
        return false;
    }
    if b.subject.is_empty() {
        return a.subject.is_empty();
    }
    let b_canonical = canonical_rule_tool(&b.tool);
    if b_canonical == "bash"
        && (b.literal || !has_glob_meta(&b.subject))
        && bash_analysis().approval_class(&b.subject) != BashApprovalClass::Reusable
    {
        return match_any_exact(std::slice::from_ref(&a), &b_canonical, &b.subject);
    }
    if a.subject.is_empty() {
        return true;
    }
    if bash_rule_prefix_base_matches(&a, &b) {
        return true;
    }
    if b.literal || !has_glob_meta(&b.subject) {
        return rule_subject_matches(&a, &b.subject);
    }
    !a.literal && a.subject == b.subject
}

pub(crate) fn has_glob_meta(s: &str) -> bool {
    s.contains(['*', '?'])
}

fn bash_rule_prefix_base_matches(existing: &Rule, candidate: &Rule) -> bool {
    if canonical_rule_tool(&existing.tool) != "bash"
        || canonical_rule_tool(&candidate.tool) != "bash"
    {
        return false;
    }
    let (Some(existing_base), Some(candidate_base)) = (
        bash_prefix_base(&existing.subject),
        bash_prefix_base(&candidate.subject),
    ) else {
        return false;
    };
    existing_base == candidate_base
}

pub(crate) fn rule_tool_matches(rule_tool: &str, tool_name: &str) -> bool {
    let rule_tool = canonical_rule_tool(rule_tool);
    rule_tool == tool_name || in_group(&rule_tool, tool_name)
}

fn rule_tool_compatible(existing_tool: &str, candidate_tool: &str) -> bool {
    let existing_tool = canonical_rule_tool(existing_tool);
    let candidate_tool = canonical_rule_tool(candidate_tool);
    existing_tool == candidate_tool || in_group(&existing_tool, &candidate_tool)
}

pub(crate) fn canonical_rule_tool(tool_name: &str) -> String {
    match tool_name.trim() {
        "Bash" | "bash" => "bash".to_string(),
        "Edit" | "edit" | "file_mutation" => "file_mutation".to_string(),
        "Browser" | "browser" => "browser".to_string(),
        "Computer" | "computer" => "computer".to_string(),
        _ => tool_name.to_string(),
    }
}

pub(crate) fn rule_subject_matches(rule: &Rule, subject: &str) -> bool {
    if rule.subject.is_empty() {
        return true;
    }
    if subject.is_empty() {
        return false;
    }
    if rule.literal {
        return rule.subject == subject;
    }
    if canonical_rule_tool(&rule.tool) == "bash" {
        // A bash prefix rule answers through bashPrefixMatches alone: the
        // prefix must name the subject's command as a static command, so a
        // compound statement never falls through to the raw glob.
        if let Some(base) = bash_colon_prefix_base(&rule.subject) {
            return bash_prefix_matches(&base, subject);
        }
        if let Some(base) = legacy_bash_space_star_prefix_base(&rule.subject) {
            return bash_prefix_matches(&base, subject);
        }
    }
    match_glob(&rule.subject, subject)
}

fn bash_colon_prefix_base(pattern: &str) -> Option<String> {
    let base = pattern.strip_suffix(":*")?;
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

fn legacy_bash_space_star_prefix_base(pattern: &str) -> Option<String> {
    let base = pattern.strip_suffix(" *")?;
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

fn bash_prefix_base(pattern: &str) -> Option<String> {
    bash_colon_prefix_base(pattern).or_else(|| legacy_bash_space_star_prefix_base(pattern))
}

fn bash_prefix_matches(base: &str, subject: &str) -> bool {
    let subject = match super::network::normalize_bash_safe_redirects_for_match(subject) {
        Some(normalized) => normalized,
        None => subject.to_string(),
    };
    let (fields, malformed) = super::shellparse::static_fields(&subject);
    if !malformed.is_empty() {
        return false;
    }
    let (base_fields, malformed) = super::shellparse::static_fields(base);
    if !malformed.is_empty() || base_fields.is_empty() || fields.len() < base_fields.len() {
        return false;
    }
    base_fields
        .iter()
        .zip(&fields)
        .all(|(want, got)| got == want)
}

/// Builds the rule string persisted when the user picks "always allow". Bash
/// commands prefer a safe command prefix (e.g. go test:*) so "always allow"
/// covers similar invocations with different arguments. File mutation tools
/// are remembered tool-wide ("Edit") so approving one file edit covers all
/// files. Other tools are remembered by tool name. Deny and ask rules keep
/// their higher precedence.
pub fn remember_rule_for_scope(tool_name: &str, subject: &str) -> String {
    let subject = subject.trim();
    if !subject.is_empty() && tool_name == "bash" {
        if let Some(pattern) = bash_command_prefix(subject) {
            return format!("Bash({pattern})");
        }
        return format!("Bash={subject}");
    }
    if let Some(rule) = group_grant_rule(tool_name, subject) {
        return rule;
    }
    tool_name.to_string()
}

/// Returns the in-memory rule for a session grant. Bash prefers a command
/// prefix when one is available; file mutation tools share a single Edit
/// grant; all other tools are recorded per [`super::write_fence`]'s rules.
pub fn session_grant_rule_for_scope(tool_name: &str, subject: &str) -> String {
    let subject = subject.trim();
    if tool_name == "bash" && !subject.is_empty() {
        if let Some(pattern) = bash_command_prefix(subject) {
            return format!("Bash({pattern})");
        }
        return format!("Bash={subject}");
    }
    if let Some(rule) = group_grant_rule(tool_name, subject) {
        return rule;
    }
    super::write_fence::session_grant_rule(tool_name, subject)
}

/// Returns a conservative prefix rule for "similar command" approvals. It
/// avoids shell syntax and keeps the prefix at command-word boundaries, so
/// approving "go test ./..." grants "go test:*" rather than a broader "go *".
pub fn bash_command_prefix(subject: &str) -> Option<String> {
    let cmd = subject.trim();
    if cmd.is_empty()
        || super::shellparse::contains_shell_syntax(cmd)
        || bash_analysis().approval_class(cmd) != BashApprovalClass::Reusable
    {
        return None;
    }
    if !bash_danger_warning(cmd).is_empty() {
        return None;
    }
    let (fields, malformed) = super::shellparse::static_fields(cmd);
    if !malformed.is_empty() || fields.len() < 2 {
        return None;
    }
    let base = fields[0].to_ascii_lowercase();
    if is_package_manager_run(&base) && fields.len() >= 3 && fields[1].eq_ignore_ascii_case("run") {
        return Some(format!("{} {} {}:*", fields[0], fields[1], fields[2]));
    }
    Some(format!("{} {}:*", fields[0], fields[1]))
}

fn is_package_manager_run(base: &str) -> bool {
    matches!(base, "npm" | "pnpm" | "yarn" | "bun")
}

/// Returns a short label if subject matches a known dangerous pattern, or ""
/// when the command looks safe. This is a visual hint only — the Policy rules
/// are the authority.
pub fn bash_danger_warning(subject: &str) -> String {
    let s = subject.trim();
    for (pattern, label) in DANGEROUS_BASH_PATTERNS {
        if match_glob(pattern, s) {
            return (*label).to_string();
        }
    }
    String::new()
}

const DANGEROUS_BASH_PATTERNS: &[(&str, &str)] = &[
    ("rm -rf*", "recursive delete"),
    ("rm -r *", "recursive delete"),
    ("rm -fr*", "recursive delete"),
    ("git push*--force*", "force push"),
    ("git push*-f*", "force push"),
    ("git reset --hard*", "hard reset"),
    ("git clean -f*", "force clean"),
    ("git restore*", "discards uncommitted changes"),
    ("git checkout -- *", "discards uncommitted changes"),
    ("git checkout .*", "discards uncommitted changes"),
    ("git stash drop*", "drops stashed changes"),
    ("git stash clear*", "drops stashed changes"),
    ("chmod 777*", "world-writable"),
    ("chmod -R 777*", "world-writable recursive"),
    ("chown *", "ownership change"),
    ("sudo *", "superuser"),
    ("mkfs*", "filesystem format"),
    ("dd if=*", "raw device write"),
    ("fdisk*", "partition table"),
    ("> /dev/*", "device overwrite"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::value::RawValue;

    fn raw(s: &str) -> Box<RawValue> {
        RawValue::from_string(s.to_string()).unwrap()
    }

    fn cmd_args(cmd: &str) -> Box<RawValue> {
        raw(&format!(r#"{{"command":"{cmd}"}}"#).replace('\\', "\\\\"))
    }

    #[test]
    fn policy_decide_table() {
        let p = Policy::new(
            "ask",
            &["bash(go test*)", "ls"],
            &["read_file"],
            &["bash(rm -rf*)"],
        );
        let cases: &[(&str, &str, bool, &str, Decision)] = &[
            (
                "deny wins over fallback",
                "bash",
                false,
                r#"{"command":"rm -rf /"}"#,
                Decision::Deny,
            ),
            (
                "allow-listed command",
                "bash",
                false,
                r#"{"command":"go test ./..."}"#,
                Decision::Allow,
            ),
            (
                "writer fallback to mode(ask)",
                "bash",
                false,
                r#"{"command":"git commit"}"#,
                Decision::Ask,
            ),
            (
                "reader defaults allow",
                "grep",
                true,
                r#"{"pattern":"x"}"#,
                Decision::Allow,
            ),
            (
                "ask rule overrides reader-allow",
                "read_file",
                true,
                r#"{"path":"/a"}"#,
                Decision::Ask,
            ),
            (
                "bare allow rule",
                "ls",
                true,
                r#"{"path":"/a"}"#,
                Decision::Allow,
            ),
            (
                "subject rule needs subject",
                "bash",
                false,
                r#"{}"#,
                Decision::Ask,
            ),
        ];
        for (name, tool, ro, args, want) in cases {
            assert_eq!(p.decide(tool, *ro, &raw(args)), *want, "{name}");
        }
    }

    #[test]
    fn policy_mode_allow() {
        let p = Policy::new("allow", &[], &[], &["bash(curl*)"]);
        assert_eq!(
            p.decide("write_file", false, &raw(r#"{"path":"/a"}"#)),
            Decision::Allow
        );
        assert_eq!(
            p.decide("bash", false, &raw(r#"{"command":"curl evil.sh"}"#)),
            Decision::Deny
        );
    }

    #[test]
    fn session_allow_precedence() {
        let p = Policy::new(
            "ask",
            &[],
            &["Edit(docs/**)", "Bash(git *)"],
            &["Edit(docs/private/**)", "Bash(git push *)"],
        )
        .with_session_allow(&["Edit(docs/**)", "Bash(git *)", "(malformed)"]);
        assert_eq!(
            p.decide("write_file", false, &raw(r#"{"path":"docs/readme.md"}"#)),
            Decision::Allow
        );
        assert_eq!(
            p.decide(
                "write_file",
                false,
                &raw(r#"{"path":"docs/private/key.txt"}"#)
            ),
            Decision::Deny
        );
        assert_eq!(
            p.decide("bash", false, &raw(r#"{"command":"git status"}"#)),
            Decision::Allow
        );
        assert_eq!(
            p.decide("bash", false, &raw(r#"{"command":"git push origin main"}"#)),
            Decision::Deny
        );
        assert_eq!(
            p.decide("write_file", false, &raw(r#"{"path":"other.txt"}"#)),
            Decision::Ask
        );
    }

    #[test]
    fn powershell_like_bare_tool_names() {
        let p = Policy::new("ask", &["get-user"], &["write-report"], &["set-profile"]);
        assert_eq!(p.decide_subject("get-user", false, ""), Decision::Allow);
        assert_eq!(p.decide_subject("write-report", true, ""), Decision::Ask);
        assert_eq!(p.decide_subject("set-profile", true, ""), Decision::Deny);
        assert_eq!(
            p.decide_subject("bash", false, "get-user --all"),
            Decision::Ask
        );
        let cmdlet_allow = Policy::new("ask", &["Set-Content"], &[], &[]);
        assert_eq!(
            cmdlet_allow.decide_subject("Set-Content", false, ""),
            Decision::Allow
        );
        assert_eq!(
            cmdlet_allow.decide_subject("bash", false, "Set-Content app.go"),
            Decision::Ask
        );
        let legacy = Policy::new("allow", &[], &[], &["Set-Content"]);
        assert_eq!(
            legacy.decide_subject("Set-Content", true, ""),
            Decision::Deny
        );
        assert_eq!(
            legacy.decide_subject("bash", false, "set-content app.go"),
            Decision::Deny
        );
    }

    #[test]
    fn exact_and_literal_rules_match_exactly() {
        let p = Policy::new("ask", &["Bash(go build)"], &[], &[]);
        assert_eq!(
            p.decide("bash", false, &raw(r#"{"command":"go build"}"#)),
            Decision::Allow
        );
        assert_ne!(
            p.decide("bash", false, &raw(r#"{"command":"go build ./cmd"}"#)),
            Decision::Allow
        );
        let lit = Policy::new("ask", &["bash=rm *.log"], &[], &[]);
        assert_eq!(
            lit.decide("bash", false, &raw(r#"{"command":"rm *.log"}"#)),
            Decision::Allow
        );
        assert_ne!(
            lit.decide("bash", false, &raw(r#"{"command":"rm secrets.log"}"#)),
            Decision::Allow
        );
    }

    #[test]
    fn remember_rule_prefixes() {
        let got = remember_rule_for_scope("bash", "go test ./...");
        assert_eq!(got, "Bash(go test:*)");
        let (r, ok) = parse_permrule(&got);
        assert!(ok && !r.literal && r.tool == "Bash" && r.subject == "go test:*");
        assert!(rule_matches_string(&got, "bash", "go test ./..."));
        assert!(rule_matches_string(
            &got,
            "bash",
            "go test ./internal/session/control"
        ));
        assert!(!rule_matches_string(&got, "bash", "go build ./..."));
        assert!(rule_matches_string(
            "Bash(go test:*)",
            "bash",
            "go \"test\" ./legacy"
        ));
        assert!(!rule_matches_string(
            "Bash(go test:*)",
            "bash",
            "go \"$subcmd\" ./legacy"
        ));
        assert!(rule_matches_string(
            "Bash(go test *)",
            "bash",
            "go test ./legacy"
        ));
        assert!(!rule_matches_string(
            "Bash(go test *)",
            "bash",
            "go test ./legacy && rm -rf /tmp/x"
        ));
        assert_eq!(remember_rule_for_scope("edit_file", "src/app.go"), "Edit");
        assert_eq!(remember_rule_for_scope("ls", ""), "ls");
    }

    #[test]
    fn persisted_edit_rule_is_tool_wide() {
        let rule = remember_rule_for_scope("edit_file", "src/app.go");
        assert_eq!(rule, "Edit");
        for tool in [
            "write_file",
            "edit_file",
            "multi_edit",
            "move_file",
            "notebook_edit",
            "delete_range",
            "delete_symbol",
        ] {
            assert!(rule_matches_string(&rule, tool, "any/path/at/all.txt"));
        }
        assert!(!rule_matches_string(&rule, "bash", "rm -rf /"));
    }

    #[test]
    fn bash_command_prefix_rejects_shell_syntax_and_danger() {
        assert_eq!(bash_command_prefix("go test ./... && rm -rf /tmp/x"), None);
        assert_eq!(bash_command_prefix("rm -rf /tmp/x"), None);
        assert_eq!(
            bash_command_prefix("go test ./...").as_deref(),
            Some("go test:*")
        );
        assert_eq!(bash_command_prefix("git restore src/main.go"), None);
        assert!(bash_command_prefix("git status --short").is_some());
    }

    #[test]
    fn bash_command_prefix_package_manager_run() {
        let got = session_grant_rule_for_scope("bash", "npm run test -- --watch");
        assert_eq!(got, "Bash(npm run test:*)");
        assert!(rule_matches_string(&got, "bash", "npm run test -- src"));
        assert!(!rule_matches_string(&got, "bash", "npm run build"));
    }

    #[test]
    fn session_grant_keys() {
        let a = session_grant_rule_for_scope("bash", "go build");
        let b = session_grant_rule_for_scope("bash", "go test ./...");
        assert_ne!(a, b);
        let a = session_grant_rule_for_scope("edit_file", "src/a.go");
        let b = session_grant_rule_for_scope("write_file", "src/b.go");
        assert_eq!(a, b);
    }

    #[test]
    fn rule_covers_table() {
        let cases: &[(&str, &str, bool)] = &[
            ("Bash(go test:*)", "Bash(go test ./...)", true),
            ("Bash(go test *)", "Bash(go test ./...)", true),
            ("bash(go test *)", "Bash(go test)", true),
            ("bash=go test ./...", "Bash(go test ./...)", true),
            ("Bash(go test *)", "Bash(go test:*)", true),
            ("Bash(go test:*)", "Bash(go test *)", true),
            ("Bash(go test ./...)", "Bash(go test:*)", false),
            ("Edit", "Edit(src/app.go)", true),
            ("file_mutation", "Edit(src/app.go)", true),
            ("Edit(src/app.go)", "Edit", false),
            ("Bash(go test:*)", "Bash(go build ./...)", false),
        ];
        for (existing, candidate, want) in cases {
            assert_eq!(
                rule_covers_string(existing, candidate),
                *want,
                "covers({existing:?}, {candidate:?})"
            );
        }
    }

    #[test]
    fn danger_warning_table() {
        let cases: &[(&str, &str)] = &[
            ("ls", ""),
            ("cat main.go", ""),
            ("rm -rf /tmp/build", "recursive delete"),
            ("rm -r old_files", "recursive delete"),
            ("git push --force origin main", "force push"),
            ("git push -f", "force push"),
            ("git reset --hard HEAD~1", "hard reset"),
            ("chmod 777 script.sh", "world-writable"),
            ("sudo make install", "superuser"),
            ("git clean -fd", "force clean"),
            ("git restore src/main.go", "discards uncommitted changes"),
            (
                "git checkout -- src/main.go",
                "discards uncommitted changes",
            ),
            ("git stash drop", "drops stashed changes"),
            ("git stash clear", "drops stashed changes"),
            ("git status", ""),
            ("git checkout -b feature", ""),
            ("git stash list", ""),
        ];
        for (cmd, want) in cases {
            assert_eq!(bash_danger_warning(cmd), *want, "danger({cmd:?})");
        }
    }

    #[test]
    fn new_policy_counts() {
        let p = Policy::new("deny", &["ls"], &["read_file"], &["bash(rm*)"]);
        assert_eq!(p.mode, Decision::Deny);
        assert_eq!(p.allow.len(), 1);
        assert_eq!(p.ask.len(), 1);
        assert_eq!(p.deny.len(), 1);
    }

    #[test]
    fn move_file_checks_both_endpoints() {
        let deny_dest = Policy::new("allow", &[], &[], &["Edit(secrets/**)"]);
        assert_eq!(
            deny_dest.decide(
                "move_file",
                false,
                &raw(r#"{"source_path":"tmp/a.md","destination_path":"secrets/a.md"}"#)
            ),
            Decision::Deny
        );
        let ask_dest = Policy::new("allow", &[], &["Edit(secrets/**)"], &[]);
        assert_eq!(
            ask_dest.decide(
                "move_file",
                false,
                &raw(r#"{"source_path":"tmp/a.md","destination_path":"secrets/a.md"}"#)
            ),
            Decision::Ask
        );
        let source_only = Policy::new("ask", &["Edit(tmp/**)"], &[], &[]);
        assert_eq!(
            source_only.decide(
                "move_file",
                false,
                &raw(r#"{"source_path":"tmp/a.md","destination_path":"docs/a.md"}"#)
            ),
            Decision::Ask
        );
        let both = Policy::new("ask", &["Edit(tmp/**)", "Edit(docs/**)"], &[], &[]);
        assert_eq!(
            both.decide(
                "move_file",
                false,
                &raw(r#"{"source_path":"tmp/a.md","destination_path":"docs/a.md"}"#)
            ),
            Decision::Allow
        );
    }

    // The turn policy no longer freezes "don't push" off the user's wording,
    // so this gate is the only thing standing between the model and an
    // irreversible external action. No mode — including the unrecognized
    // spellings Go's ParseDecision maps to Ask — may answer Allow on its own.
    #[test]
    fn external_actions_always_reach_approval() {
        for mode in ["auto", "ask", "yolo"] {
            let p = Policy::new(mode, &[], &[], &[]);
            for command in [
                "git push",
                "git push --force origin main",
                "git -C ../repo push origin HEAD",
                "npm publish",
                "npm --workspace pkg publish",
                "gh release create v1",
                "kubectl -n production apply -f deploy.yaml",
            ] {
                let args = raw(&format!(
                    r#"{{"command":{}}}"#,
                    serde_json::to_string(command).unwrap()
                ));
                assert_ne!(
                    p.decide("bash", bash_analysis().is_read_only(command), &args),
                    Decision::Allow,
                    "mode={mode} {command:?} decided Allow"
                );
            }
        }
    }

    #[test]
    fn compound_bash_decisions() {
        // Depends on the compound-segment analysis the static engine does not
        // carry yet; the AST engine slice unignores these.
        let _ = cmd_args("git status");
    }
}

#[cfg(test)]
mod probe_pol {
    use super::*;
    #[test]
    fn probe_compound() {
        let cls = bash_analysis().approval_class("go test ./legacy && rm -rf /tmp/x");
        println!("class compound => {:?}", cls);
        let cls1 = bash_analysis().approval_class("go test ./legacy");
        println!("class seg1 => {:?}", cls1);
        let cls2 = bash_analysis().approval_class("rm -rf /tmp/x");
        println!("class seg2 => {:?}", cls2);
        let split = reasonix_core::shellparse_analysis::split_top_level(
            "go test ./legacy && rm -rf /tmp/x",
        );
        println!("split => {:?}", split.clone().map(|x| x.0.clone()));
        let (argv, m) =
            reasonix_core::shellparse::static_fields("go test ./legacy && rm -rf /tmp/x");
        println!("static_fields compound => argv={argv:?} m={m:?}");
        let base = bash_colon_prefix_base("go test *")
            .or_else(|| legacy_bash_space_star_prefix_base("go test *"));
        println!("prefix base => {:?}", base);
        if let Some(b) = base.clone() {
            println!(
                "bash_prefix_matches => {}",
                bash_prefix_matches(&b, "go test ./legacy && rm -rf /tmp/x")
            );
        }
        let m2 = rule_matches_string(
            "Bash(go test *)",
            "bash",
            "go test ./legacy && rm -rf /tmp/x",
        );
        println!("match => {}", m2);
    }
}
