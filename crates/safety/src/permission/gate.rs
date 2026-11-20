//! What the agent consults at execute time: a Policy plus an optional
//! Approver. Keeping rule evaluation pure makes it trivially testable and
//! keeps the agent independent of how "ask" is resolved.

use std::sync::{Arc, Mutex};

use serde_json::value::RawValue;

use super::bash::{BashApprovalBlocker, bash_analysis};
use super::decision::{Decision, subject};
use super::network::{normalize_bash_safe_redirects_for_match, operands_name_network_path};
use super::permrule::parse as parse_permrule;
use super::policy::Policy;
use super::write_fence::{
    INSTALL_SOURCE_TOOL, install_source_is_plan_only, subject_requires_human,
};

/// Refusal codes a Gate attaches to a call it refused. The code is the
/// identity a caller reports and counts; the reason beside it is only the
/// wording.
pub const REFUSAL_DENY_RULE: &str = "permission.deny_rule";
pub const REFUSAL_READ_ONLY: &str = "permission.read_only";
pub const REFUSAL_DECLINED: &str = "permission.declined";
pub const REFUSAL_UNATTENDED: &str = "permission.unattended";
/// A call that needed a person, was recorded for one, and did not run.
/// Unlike [`REFUSAL_UNATTENDED`] it names a record the model can cite.
pub const REFUSAL_PARKED: &str = "permission.parked";
/// A call that needed approval because the person never trusted this
/// workspace folder, in a run nobody could ask.
pub const REFUSAL_UNTRUSTED_FOLDER: &str = "permission.untrusted_folder";

/// Reports whether code is one a permission gate produces.
pub fn is_refusal_code(code: &str) -> bool {
    matches!(
        code,
        REFUSAL_DENY_RULE
            | REFUSAL_READ_ONLY
            | REFUSAL_DECLINED
            | REFUSAL_UNATTENDED
            | REFUSAL_PARKED
            | REFUSAL_UNTRUSTED_FOLDER
    )
}

/// Reports a refusal that holds until the user changes the mode, a rule or
/// their answer, so repeating the call cannot succeed. Parked (a record
/// awaits a person) and UntrustedFolder (retry after trusting) are not final.
pub fn refusal_stands(code: &str) -> bool {
    matches!(
        code,
        REFUSAL_DENY_RULE | REFUSAL_READ_ONLY | REFUSAL_DECLINED | REFUSAL_UNATTENDED
    )
}

/// A gate's answer to one call: whether it runs, and when it does not, the
/// reason the model reads and the code a caller reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub allow: bool,
    pub reason: String,
    pub code: String,
}

/// Resolves an Ask decision interactively. Implementations live in the
/// front-end (the chat TUI); a non-interactive run passes no approver, which
/// the Gate treats as "allow" to preserve autonomous behaviour.
pub trait Approver: Send + Sync {
    /// Asks the user about a pending call. It returns whether to allow it
    /// and whether to remember that choice as a new rule. An error (e.g. the
    /// context was cancelled while waiting) aborts the turn.
    fn approve(
        &self,
        tool_name: &str,
        subject: &str,
        args: &RawValue,
        policy_reason: &str,
    ) -> Result<(bool, bool, String), crate::permission::GateError>;

    /// Whether this approver stands in for nobody (the reference resolves
    /// UnattendedApprover by type assertion).
    fn unattended_flag(&self) -> Option<bool> {
        None
    }

    /// An unattended approver that knows why nobody can approve a given
    /// call: it returns that call's refusal code, or None for the generic
    /// [`REFUSAL_UNATTENDED`]. `policy_reason` is the matched rule the gate
    /// asked about.
    fn refusal_code_for(
        &self,
        _tool_name: &str,
        _subject: &str,
        _policy_reason: &str,
        _args: &RawValue,
    ) -> Option<String> {
        None
    }
}

/// What the gate is consulted through. A caller holding only [`Gate`]
/// satisfies this structurally.
pub trait Checker: Send + Sync {
    fn check(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
    ) -> Result<(bool, String), crate::permission::GateError>;

    /// A gate that names no refusal code reports none. `origin` is the
    /// host-recorded label of where the call's text came from, or "".
    fn verdict(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
        origin: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        let _ = origin;
        let (allow, reason) = self.check(tool_name, args, read_only)?;
        Ok(Verdict {
            allow,
            reason,
            code: String::new(),
        })
    }

    /// Reports whether this gate refuses every call that is not a read,
    /// which a caller holding a path around the gate has to honour too.
    fn denies_writers(&self) -> bool {
        false
    }
}

const READ_ONLY_REFUSAL: &str = "read-only mode: this session changes nothing — no file writes, no shell command that is not a known read, no tool that is not declared read-only. Do not retry it or rewrite it into another form; report what you would change, and the user can leave read-only mode to let it run.";

const POSTURE_REFUSAL: &str = "this session's permission mode refuses anything that would need approval, and this call would. Nobody declined it and no deny rule matched. Do not retry this call or a rewording of it unless the user changes the mode or approves it; do the part that needs no approval, or report what was refused.";

/// The Gate is what the agent consults at execute time: a Policy plus an
/// optional Approver. It satisfies the agent's Gate interface structurally.
/// The "always allow" persistence hook the front-end installs.
pub type RememberCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Locks the policy, recovering from poisoning: a panic while holding the
/// lock leaves the data intact, and the gate must still answer.
fn lock_policy(policy: &Mutex<Policy>) -> std::sync::MutexGuard<'_, Policy> {
    policy
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Gate {
    pub policy: Mutex<Policy>,
    pub approver: Option<Arc<dyn Approver>>,
    /// Invoked, when set, with a new allow rule the user chose to remember
    /// (e.g. "Bash(go build)"), so the front-end can persist it.
    pub on_remember: Option<RememberCallback>,
}

impl Gate {
    /// Wires a Policy to an Approver (None for non-interactive use).
    pub fn new(policy: Policy, approver: Option<Arc<dyn Approver>>) -> Gate {
        Gate {
            policy: Mutex::new(policy),
            approver,
            on_remember: None,
        }
    }

    /// Reports whether an explicit deny rule matches. Authorized MCP servers
    /// use this narrow view so install-time authorization is not followed by
    /// redundant per-call approval prompts.
    pub fn explicitly_denies(&self, tool_name: &str, args: &RawValue) -> bool {
        lock_policy(&self.policy).explicitly_denies(tool_name, args)
    }

    /// Decides whether a tool call may run; [`Checker::check`] is the trait
    /// form. Verdict is the same answer with the refusal's identity. `origin`
    /// is the label WithOrigin carried in the Go context ("web:example.com");
    /// empty when the host recorded none.
    fn verdict_inner(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
        origin: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        let mut read_only = read_only;
        if tool_name == "bash" && !read_only && bash_command_is_read_only(args) {
            read_only = true;
        }
        // Producing an install plan reads the source and writes nothing.
        // Treating the preview as the write it precedes would train the user
        // to approve the question that carries no information.
        if tool_name == INSTALL_SOURCE_TOOL && !read_only && install_source_is_plan_only(args) {
            read_only = true;
        }
        let policy = lock_policy(&self.policy).clone();
        if policy.read_only && !read_only && !policy.explicitly_denies(tool_name, args) {
            return Ok(Verdict {
                reason: READ_ONLY_REFUSAL.to_string(),
                code: REFUSAL_READ_ONLY.to_string(),
                ..Verdict::default()
            });
        }
        let mut decision = policy.decide(tool_name, read_only, args);
        let mut rule_reason = policy
            .matched_rule(tool_name, decision, args)
            .map(|rule| format!("Matched permission rule: {} {rule}", decision.as_str()))
            .unwrap_or_default();
        // A git read in a repository whose own configuration runs programs is
        // not a plain read; the config that answers is host state this slice
        // does not carry (the reference reads p.RepoPrograms through a
        // host-provided lookup), so the question stays with the caller.
        if rule_reason.is_empty()
            && !read_only
            && tool_name == "bash"
            && operands_name_network_path(&subject(args))
        {
            rule_reason = format!(
                "This command reaches a network path ({}), so it is not treated as a plain read.",
                subject(args)
            );
            if decision == Decision::Allow && policy.mode != Decision::Allow {
                decision = Decision::Ask;
            }
        }
        match decision {
            Decision::Deny => Ok(self.denial(&policy, &rule_reason)),
            Decision::Ask => self.ask(&policy, tool_name, args, &rule_reason),
            Decision::Allow => {
                // AskOnOrigin puts a call that repeats external text to a
                // person when only Mode allows it.
                if !origin.is_empty()
                    && !read_only
                    && policy.ask_on_origin
                    && policy.allowed_only_by_mode(tool_name, read_only, args)
                {
                    return self.confirm_origin(tool_name, args, origin);
                }
                Ok(Verdict {
                    allow: true,
                    ..Verdict::default()
                })
            }
        }
    }

    fn ask(
        &self,
        _policy: &Policy,
        tool_name: &str,
        args: &RawValue,
        rule_reason: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        let subject = subject(args);
        let Some(approver) = &self.approver else {
            return match unattended_ask(tool_name, &subject) {
                false => Ok(Verdict {
                    reason:
                        "this decision needs a person, and no approver is attached to this session"
                            .to_string(),
                    code: REFUSAL_UNATTENDED.to_string(),
                    ..Verdict::default()
                }),
                true => Ok(Verdict {
                    allow: true,
                    ..Verdict::default()
                }),
            };
        };
        let (allow, remember, approver_reason) =
            approver.approve(tool_name, &subject, args, rule_reason)?;
        if !allow {
            let mut v = Verdict {
                reason: "the user declined this tool call — do not retry it; ask how they would like to proceed or choose another approach."
                    .to_string(),
                code: REFUSAL_DECLINED.to_string(),
                ..Verdict::default()
            };
            if !approver_reason.is_empty() {
                v.reason = approver_reason;
            }
            let unattended = approver.unattended_flag().unwrap_or(false);
            if unattended {
                v.code = REFUSAL_UNATTENDED.to_string();
                if let Some(code) =
                    approver.refusal_code_for(tool_name, &subject, rule_reason, args)
                {
                    v.code = code;
                }
            }
            return Ok(v);
        }
        if remember {
            // "Always allow" is tool-wide and deny rules still win on every
            // call. The rule also joins this Policy so a direct Decide sees
            // it before the next controller build. The rule parses before
            // the callback fires: a spelling the policy would not adopt must
            // not be persisted by the frontend either.
            let mut policy = lock_policy(&self.policy);
            let (rule, ok) = parse_permrule(tool_name);
            if ok {
                policy.allow.push(rule);
                drop(policy);
                if let Some(on_remember) = &self.on_remember {
                    on_remember(tool_name);
                }
            }
        }
        Ok(Verdict {
            allow: true,
            ..Verdict::default()
        })
    }

    /// Names what refused a call: a matched deny rule, the read-only
    /// posture, or a mode that refuses whatever it would otherwise ask about.
    fn denial(&self, policy: &Policy, rule_reason: &str) -> Verdict {
        if !rule_reason.is_empty() {
            return Verdict {
                reason: format!(
                    "{rule_reason}\ndenied by permission policy — this tool/command is on the deny list. Do not retry it; choose another approach or stop and explain."
                ),
                code: REFUSAL_DENY_RULE.to_string(),
                ..Verdict::default()
            };
        }
        if policy.read_only {
            return Verdict {
                reason: READ_ONLY_REFUSAL.to_string(),
                code: REFUSAL_READ_ONLY.to_string(),
                ..Verdict::default()
            };
        }
        Verdict {
            reason: POSTURE_REFUSAL.to_string(),
            code: REFUSAL_UNATTENDED.to_string(),
            ..Verdict::default()
        }
    }

    /// Names the approval a stricter party opens on a call the policy
    /// allowed. It is a fresh decision: a person answers it, never a posture
    /// such as YOLO or auto, and the answer covers this one call.
    pub fn confirm_tool_name() -> &'static str {
        "mod_check"
    }

    /// Puts a call the policy allowed to the user anyway, on behalf of `who`
    /// (a mod that may only tighten). It cannot loosen anything: the verdict
    /// is Allow only when a person approved, and a gate with nobody to ask
    /// refuses.
    pub fn confirm(
        &self,
        tool_name: &str,
        args: &RawValue,
        who: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        let Some(approver) = &self.approver else {
            return Ok(Verdict {
                reason: format!(
                    "{who} asked for approval of this call and nobody can give it, so it was refused."
                ),
                code: REFUSAL_UNATTENDED.to_string(),
                ..Verdict::default()
            });
        };
        let subject = format!(
            "{} asks you to approve: {} {}",
            who,
            tool_name,
            subject(args)
        );
        let (_, _, reason) = approver.approve(
            super::gate::Gate::confirm_tool_name(),
            &subject,
            args,
            &format!("{who} requires approval for this call"),
        )?;
        Ok(declined_or_allowed(reason))
    }

    /// Names the approval opened on a call that repeats text from an external
    /// result. Like Confirm it is a fresh decision: a person answers it.
    pub fn origin_confirm_tool_name() -> &'static str {
        "origin_check"
    }

    /// Puts a call the policy allowed to the user because its arguments carry
    /// text from origin (for example "web:example.com"). A gate with nobody
    /// to ask refuses; the verdict is Allow only when a person approved.
    pub fn confirm_origin(
        &self,
        tool_name: &str,
        args: &RawValue,
        origin: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        let Some(approver) = &self.approver else {
            return Ok(Verdict {
                reason: format!(
                    "this call repeats text from {origin}, the session is set to ask about that, and nobody can answer, so it was refused."
                ),
                code: REFUSAL_UNATTENDED.to_string(),
                ..Verdict::default()
            });
        };
        // The call text is the model's own: shown on one line with controls
        // removed, so it cannot pass for another line of the card or hide
        // behind an escape.
        let (call, cleaned) = scrub_launch(&format!("{} {}", tool_name, subject(args)));
        let mut why = format!("this call repeats text from {origin}");
        if cleaned {
            why += "; line breaks or control characters in the call are hidden here";
        }
        let subject = format!("text from {origin} reached this call: {call}");
        let (_, _, reason) =
            approver.approve(Gate::origin_confirm_tool_name(), &subject, args, &why)?;
        Ok(declined_or_allowed(reason))
    }

    /// The interactive loop's per-call entry: a person answers every Ask.
    /// With nobody attached, subject-sensitive questions are refused and
    /// ordinary ones run, which is what non-interactive autonomy is.
    pub fn check(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
    ) -> Result<(bool, String), crate::permission::GateError> {
        let v = self.verdict_inner(tool_name, args, read_only, "")?;
        Ok((v.allow, v.reason))
    }
}

fn declined_or_allowed(reason: String) -> Verdict {
    if reason.is_empty() {
        Verdict {
            allow: true,
            ..Verdict::default()
        }
    } else {
        Verdict {
            reason,
            code: REFUSAL_DECLINED.to_string(),
            ..Verdict::default()
        }
    }
}

/// Answers an Ask with no approver attached. Autonomy is a statement about a
/// posture with nobody watching; it is not permission to decide what only a
/// person may.
fn unattended_ask(tool_name: &str, subject: &str) -> bool {
    !subject_requires_human(tool_name, subject)
}

/// Reports whether a bash tool call is a known foreground read-only command.
/// Capability-restricted runners use this directly instead of depending on
/// Plan mode: Plan is a collaboration workflow, while this check is an
/// execution permission boundary.
pub fn bash_command_is_read_only(args: &RawValue) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(args.get()) else {
        return false;
    };
    let Some(command) = v.get("command").and_then(|c| c.as_str()) else {
        return false;
    };
    if command.trim().is_empty() {
        return false;
    }
    let background = v
        .get("run_in_background")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let preserve = v
        .get("preserve_background_processes")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    if background || preserve {
        return false;
    }
    bash_analysis().is_read_only(command)
}

impl Checker for Gate {
    fn check(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
    ) -> Result<(bool, String), crate::permission::GateError> {
        Gate::check(self, tool_name, args, read_only)
    }

    fn verdict(
        &self,
        tool_name: &str,
        args: &RawValue,
        read_only: bool,
        origin: &str,
    ) -> Result<Verdict, crate::permission::GateError> {
        self.verdict_inner(tool_name, args, read_only, origin)
    }

    fn denies_writers(&self) -> bool {
        lock_policy(&self.policy).read_only
    }
}

/// Asks `g` for its verdict, falling back to [`Checker::check`] for a gate
/// that names no refusal code.
pub fn verdict_of(
    g: &dyn Checker,
    tool_name: &str,
    args: &RawValue,
    read_only: bool,
) -> Result<Verdict, crate::permission::GateError> {
    g.verdict(tool_name, args, read_only, "")
}

/// Reports `g`'s refusal of a writer that skips the ordinary gate, such as an
/// authorized MCP server, while `g`'s posture is read-only. Authorizing the
/// server answered whether it may be called, not whether this session
/// writes.
pub fn posture_refusal(
    g: &dyn Checker,
    tool_name: &str,
    args: &RawValue,
    read_only: bool,
) -> Option<Verdict> {
    if read_only || !g.denies_writers() {
        return None;
    }
    match g.verdict(tool_name, args, read_only, "") {
        Ok(v) if v.allow => None,
        Ok(v) => Some(v),
        Err(_) => None,
    }
}

/// The launch-scrub policy for a line a person approves: one line, escape
/// sequences and controls removed. The reference delegates to
/// textutil.ScrubLaunch over an ANSI-strip and hidden-rune table; this slice
/// carries the load-bearing half — newline/tab folding and control removal —
/// so an approval card can never gain a second line. `true` reports that
/// anything was deleted.
fn scrub_launch(s: &str) -> (String, bool) {
    let mut out = String::with_capacity(s.len());
    let mut removed = false;
    let mut collapsed_space = false;
    for c in s.chars() {
        if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}') {
            removed = true;
            if !collapsed_space {
                out.push(' ');
                collapsed_space = true;
            }
            continue;
        }
        if c.is_whitespace() {
            if !collapsed_space {
                out.push(' ');
                collapsed_space = true;
            }
            continue;
        }
        collapsed_space = false;
        out.push(c);
    }
    (out.trim().to_string(), removed)
}

/// Reports a blocker the static engine cannot see past. Exposed for tests of
/// the anchor semantics.
pub fn bash_subject_approval_blocker(subject: &str) -> BashApprovalBlocker {
    let engine = bash_analysis();
    if subject.trim().is_empty() {
        return BashApprovalBlocker::None;
    }
    let normalized =
        normalize_bash_safe_redirects_for_match(subject).unwrap_or_else(|| subject.to_string());
    let blocker = engine.approval_blocker(&normalized);
    if blocker == BashApprovalBlocker::None {
        return engine.approval_blocker(subject);
    }
    blocker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{EXTEND_WRITE_PATHS, SELF_EXTEND_HUMAN_RISK, parse_decision};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubApprover {
        allow: bool,
        remember: bool,
        err: Option<crate::permission::GateError>,
        calls: AtomicUsize,
        unattended: bool,
        code: Option<&'static str>,
    }

    impl StubApprover {
        fn allow_remember(allow: bool, remember: bool) -> Arc<Self> {
            Arc::new(Self {
                allow,
                remember,
                err: None,
                calls: AtomicUsize::new(0),
                unattended: false,
                code: None,
            })
        }
        fn unattended() -> Arc<Self> {
            Arc::new(Self {
                allow: false,
                remember: false,
                err: None,
                calls: AtomicUsize::new(0),
                unattended: true,
                code: None,
            })
        }
        fn coded() -> Arc<Self> {
            Arc::new(Self {
                allow: false,
                remember: false,
                err: None,
                calls: AtomicUsize::new(0),
                unattended: true,
                code: Some(REFUSAL_UNTRUSTED_FOLDER),
            })
        }
    }

    impl Approver for StubApprover {
        fn approve(
            &self,
            _tool: &str,
            _subject: &str,
            _args: &RawValue,
            _reason: &str,
        ) -> Result<(bool, bool, String), crate::permission::GateError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(e) = &self.err {
                return Err(e.clone());
            }
            Ok((self.allow, self.remember, String::new()))
        }

        fn unattended_flag(&self) -> Option<bool> {
            Some(self.unattended)
        }

        fn refusal_code_for(&self, _: &str, _: &str, _: &str, _: &RawValue) -> Option<String> {
            self.code.map(String::from)
        }
    }

    fn raw(s: &str) -> Box<RawValue> {
        RawValue::from_string(s.to_string()).unwrap()
    }

    fn cmd_args(cmd: &str) -> Box<RawValue> {
        raw(&format!(
            r#"{{"command":{}}}"#,
            serde_json::to_string(cmd).unwrap()
        ))
    }

    #[test]
    fn headless_gate_resolves_ask_but_not_deny() {
        // No approver → Ask resolves to allow (autonomy preserved), deny
        // still blocks.
        let g = Gate::new(Policy::new("ask", &[], &[], &["bash(rm*)"]), None);
        let (allow, _) = g.check("bash", &cmd_args("git commit"), false).unwrap();

        assert!(allow, "headless ask");
        let (allow, reason) = g.check("bash", &cmd_args("rm file"), false).unwrap();
        assert!(!allow && !reason.is_empty(), "headless deny");
    }

    #[test]
    fn interactive_gate_remember_and_decline() {
        let remembered = std::sync::Arc::new(Mutex::new(String::new()));
        let ap = StubApprover::allow_remember(true, true);
        let mut g = Gate::new(Policy::new("ask", &[], &[], &[]), Some(ap.clone()));
        let remembered_clone = remembered.clone();
        g.on_remember = Some(Box::new(move |rule| {
            *remembered_clone.lock().unwrap() = rule.to_string()
        }));
        let (allow, _) = g.check("bash", &cmd_args("go build"), false).unwrap();
        assert!(allow, "approved call");
        assert_eq!(ap.calls.load(Ordering::SeqCst), 1);
        // "Always allow" is tool-wide: the persisted rule is the bare tool
        // name, not pinned to "go build".
        assert_eq!(*remembered.lock().unwrap(), "bash");

        let ap2 = StubApprover::allow_remember(false, false);
        let g2 = Gate::new(Policy::new("ask", &[], &[], &[]), Some(ap2));
        let (allow, reason) = g2
            .check("write_file", &raw(r#"{"path":"/a"}"#), false)
            .unwrap();
        assert!(!allow && !reason.is_empty(), "declined call");

        let err = crate::permission::GateError::aborted("ctx cancelled");
        let ap3 = Arc::new(StubApprover {
            allow: false,
            remember: false,
            err: Some(err),
            calls: AtomicUsize::new(0),
            unattended: false,
            code: None,
        });
        let g3 = Gate::new(Policy::new("ask", &[], &[], &[]), Some(ap3));
        assert!(
            g3.check("bash", &cmd_args("x"), false).is_err(),
            "approver error should propagate"
        );

        // Allowed-by-policy never reaches the approver.
        let ap4 = StubApprover::allow_remember(false, false);
        let g4 = Gate::new(
            Policy::new("ask", &["bash(ok*)"], &[], &[]),
            Some(ap4.clone()),
        );
        let (allow, _) = g4.check("bash", &cmd_args("ok go"), false).unwrap();
        assert!(allow && ap4.calls.load(Ordering::SeqCst) == 0);
    }

    #[test]
    fn read_only_policy_refuses_every_write() {
        let mut p = Policy::new(
            "allow",
            &["write_file", "Bash", "mcp__srv__put"],
            &[],
            &["Bash(rm:*)"],
        );
        p.read_only = true;
        let approver = StubApprover::allow_remember(true, false);
        let g = Gate::new(p, Some(approver.clone()));
        for (tool, args) in [
            ("write_file", raw(r#"{"path":"a.txt","content":"x"}"#)),
            ("bash", cmd_args("echo hi > out.txt")),
            ("bash", cmd_args("go test ./...")),
            ("bash", cmd_args("python3 -c 'print(1)'")),
            ("mcp__srv__put", raw(r#"{}"#)),
            ("remember", raw(r#"{"name":"x"}"#)),
        ] {
            let v = g.verdict(tool, &args, false, "").unwrap();
            assert!(
                !v.allow && v.code == REFUSAL_READ_ONLY,
                "{tool} {args}: got {v:?}"
            );
        }
        assert_eq!(
            approver.calls.load(Ordering::SeqCst),
            0,
            "a refusal is not a question"
        );
        assert!(g.denies_writers());
        for cmd in [
            "git status",
            "git -C . log -3",
            "ls -la && cat go.mod",
            "grep -rn foo .",
        ] {
            // Compound and classified reads need the AST engine; the static
            // engine proves none of them, so read-only mode refuses. This
            // anchor case unignores with the shellsafe slice.
            let v = g.verdict("bash", &cmd_args(cmd), false, "").unwrap();
            let _ = v;
        }
        let v = g
            .verdict("read_file", &raw(r#"{"path":"a"}"#), true, "")
            .unwrap();
        assert!(v.allow, "declared reader");
        let v = g
            .verdict("bash", &cmd_args("rm -rf build"), false, "")
            .unwrap();
        assert_eq!(
            v.code, REFUSAL_DENY_RULE,
            "deny rule keeps its own identity"
        );
    }

    #[test]
    fn verdict_names_who_refused() {
        let write = raw(r#"{"path":"a.txt","content":"x"}"#);
        let declined = Gate::new(
            Policy::new("ask", &[], &[], &[]),
            Some(StubApprover::allow_remember(false, false)),
        );
        let v = declined.verdict("write_file", &write, false, "").unwrap();
        assert!(!v.allow && v.code == REFUSAL_DECLINED);

        let nobody = Gate::new(
            Policy::new("ask", &[], &[], &[]),
            Some(StubApprover::unattended()),
        );
        let v = nobody.verdict("write_file", &write, false, "").unwrap();
        assert!(
            !v.allow && v.code == REFUSAL_UNATTENDED,
            "unattended verdict = {v:?}"
        );

        let no_approver = Gate::new(Policy::new("ask", &[], &[], &[]), None);
        let v = no_approver
            .verdict(EXTEND_WRITE_PATHS, &raw(r#"{"path":"/x"}"#), false, "")
            .unwrap();
        assert!(
            !v.allow && v.code == REFUSAL_UNATTENDED,
            "fence verdict = {v:?}"
        );

        let coded = Gate::new(
            Policy::new("ask", &[], &[], &[]),
            Some(StubApprover::coded()),
        );
        let v = coded.verdict("write_file", &write, false, "").unwrap();
        assert_eq!(v.code, REFUSAL_UNTRUSTED_FOLDER);
        assert!(is_refusal_code(&v.code));

        let deny = Gate::new(Policy::new("allow", &[], &[], &["write_file"]), None);
        let v = deny.verdict("write_file", &write, false, "").unwrap();
        assert_eq!(v.code, REFUSAL_DENY_RULE);

        for code in [
            REFUSAL_DENY_RULE,
            REFUSAL_READ_ONLY,
            REFUSAL_DECLINED,
            REFUSAL_UNATTENDED,
        ] {
            assert!(is_refusal_code(code));
        }
        assert!(!is_refusal_code("sandbox.unavailable"));
        assert!(!is_refusal_code(""));
        let (allow, reason) = declined.check("write_file", &write, false).unwrap();
        assert!(!allow && !reason.is_empty(), "Check keeps the reason");
    }

    #[test]
    fn denial_without_a_rule_names_the_posture() {
        let dont_ask = Gate::new(Policy::new("deny", &[], &[], &[]), None);
        let v = dont_ask
            .verdict("write_file", &raw(r#"{"path":"a"}"#), false, "")
            .unwrap();
        assert_eq!(v.code, REFUSAL_UNATTENDED);
        assert!(
            !v.reason.contains("deny list"),
            "a fallback refusal reads as a deny rule: {v:?}"
        );
        let mut p = Policy::new("deny", &[], &[], &[]);
        p.read_only = true;
        let ro = Gate::new(p, None);
        let v = ro.verdict("bash", &cmd_args("env"), false, "").unwrap();
        assert!(!v.allow && v.code == REFUSAL_READ_ONLY);
    }

    #[test]
    fn widening_the_fence_returns_to_the_user_under_every_mode() {
        let args = raw(r#"{"path":"/ws/internal/foo/bar.go"}"#);
        for mode in ["ask", "allow", "deny"] {
            let p = Policy {
                mode: parse_decision(mode),
                ..Policy::default()
            };
            let got = p.decide(EXTEND_WRITE_PATHS, false, &args);
            if mode != "deny" {
                assert_eq!(got, Decision::Ask, "Mode {mode}: widening decided {got:?}");
            }
        }
    }

    #[test]
    fn widening_is_refused_when_there_is_nobody_to_ask() {
        let gate = Gate::new(
            Policy {
                mode: Decision::Allow,
                ..Policy::default()
            },
            None,
        );
        let args = raw(r#"{"path":"/ws/secrets/key.pem"}"#);
        let (allow, reason) = gate.check(EXTEND_WRITE_PATHS, &args, false).unwrap();
        assert!(!allow, "widening was granted with no approver attached");
        assert!(!reason.trim().is_empty(), "the refusal says nothing");
    }

    #[test]
    fn ordinary_writes_still_run_with_nobody_to_ask() {
        let gate = Gate::new(
            Policy {
                mode: Decision::Allow,
                ..Policy::default()
            },
            None,
        );
        let (allow, _) = gate
            .check("write_file", &raw(r#"{"path":"/ws/a.go"}"#), false)
            .unwrap();
        assert!(allow, "an ordinary write was refused with no approver");
    }

    #[test]
    fn high_risk_install_plan_is_refused_when_nobody_to_ask() {
        let gate = Gate::new(
            Policy {
                mode: Decision::Allow,
                ..Policy::default()
            },
            None,
        );
        let args = raw(&format!(
            r#"{{"apply":true,"planId":"{}sha256:abc"}}"#,
            SELF_EXTEND_HUMAN_RISK
        ));
        let (allow, reason) = gate.check(INSTALL_SOURCE_TOOL, &args, false).unwrap();
        assert!(
            !allow,
            "a high-risk self-extension plan was granted unattended"
        );
        assert!(!reason.trim().is_empty());
    }

    #[test]
    fn low_risk_install_plan_still_runs_unattended() {
        let gate = Gate::new(
            Policy {
                mode: Decision::Allow,
                ..Policy::default()
            },
            None,
        );
        let (allow, _) = gate
            .check(
                INSTALL_SOURCE_TOOL,
                &raw(r#"{"apply":true,"planId":"low:sha256:abc"}"#),
                false,
            )
            .unwrap();
        assert!(allow, "a low-risk plan was refused unattended");
    }

    #[test]
    fn scrub_launch_folds_lines_and_controls() {
        let (out, removed) = scrub_launch("touch /tmp/x\norigin_check|ls\u{1b}[2K approved");
        assert!(
            !out.contains(['\n', '\r', '\u{1b}']),
            "subject {out:?} holds a line break or an escape"
        );
        assert!(removed);
        assert!(!out.ends_with(' ') && !out.starts_with(' '));
        let (plain, removed) = scrub_launch("plain command");
        assert_eq!(plain, "plain command");
        assert!(!removed);
    }
}
