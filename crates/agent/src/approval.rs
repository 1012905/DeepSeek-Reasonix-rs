//! The interactive approval gate and ask broker (Go: `control` approval
//! bridge + orchestration's Ask half). Both are host-installed seams: a run
//! with nobody watching keeps the Go-nil seams and the headless posture.
//!
//! The gate composes `reasonix_safety::permission::Gate`: rule evaluation,
//! the read-only posture and refusal identities stay with their owner. The
//! broker here adds the host half — one decision in flight, ids a frontend
//! can answer, session grants, and receipts that reach the transcript and
//! the SSE notice.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reasonix_contract::eventwire::approval::Approval as WireApproval;
use reasonix_contract::eventwire::ask::{Ask, AskQuestion};
use reasonix_contract::eventwire::event::Event;
use reasonix_contract::provider::DecisionReceipt;
use reasonix_safety::permission::gate::{Checker, Verdict as SafetyVerdict};
use reasonix_safety::permission::{Approver, Decision, GateError, Policy, subject};
use reasonix_state::sessionstore::Session;
use serde_json::value::RawValue;

use crate::sink::{Sink, kind, level, notice_code};

/// Tool-approval postures (Go: `ToolApproval*`). [`parse_mode`] is the one
/// canonicalizer a frontend uses, so aliases cannot drift from the kernel.
pub mod mode {
    pub const ASK: &str = "ask";
    pub const AUTO: &str = "auto";
    pub const DONT_ASK: &str = "dontAsk";
    pub const READ_ONLY: &str = "readOnly";
    pub const YOLO: &str = "yolo";
}

/// Canonicalizes a posture name, reporting whether the name is one at all
/// (Go: `ParseToolApprovalMode`). An empty or unknown name is refused, never
/// silently read as Ask.
pub fn parse_mode(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ask" => Some(mode::ASK),
        "auto" | "approve" | "allow" => Some(mode::AUTO),
        "dontask" | "dont-ask" | "deny" => Some(mode::DONT_ASK),
        "readonly" | "read-only" => Some(mode::READ_ONLY),
        "yolo" | "full" | "full-access" | "bypass" => Some(mode::YOLO),
        _ => None,
    }
}

/// Shapes the base policy for a posture (Go: `newInteractiveGate`). A mode
/// moves the writer fallback; rules stay, so an explicit Ask rule survives
/// auto and a deny rule survives every mode. `readOnly` additionally refuses
/// every writer the policy would otherwise allow.
pub fn policy_for_mode(base: &Policy, mode: &str) -> Policy {
    let mut policy = base.clone();
    match mode {
        mode::AUTO | mode::YOLO => policy.mode = Decision::Allow,
        mode::DONT_ASK => policy.mode = Decision::Deny,
        mode::READ_ONLY => {
            policy.mode = Decision::Deny;
            policy.read_only = true;
        }
        _ => policy.mode = Decision::Ask,
    }
    policy
}

/// Receipt outcomes (Go: `ApproveFrom`'s outcome, `askOutcomeTimedOut`).
/// `timed_out` is the host's own answer, never a person's.
pub mod outcome {
    pub const ALLOW_ONCE: &str = "allow_once";
    pub const ALLOW_SESSION: &str = "allow_session";
    pub const ALLOW_PERSIST: &str = "allow_persist";
    pub const DENY: &str = "deny";
    pub const ASK_ANSWERED: &str = "answered";
    pub const TIMED_OUT: &str = "timed_out";
}

/// A decision about one pending tool prompt (Go: `Approve`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct DecisionInput {
    pub allow: bool,
    pub session: bool,
    pub persist: bool,
}

/// A structured answer to one question, in wire shape (Go: `event.AskAnswer`).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct AskAnswer {
    #[serde(rename = "questionId")]
    pub id: String,
    pub selected: Vec<String>,
}

/// The typed ask flow. `timed_out` carries the host's timeout as an identity
/// (Go: `AskTimedOutError`): a skip is never an empty answer a model can read
/// as a person's choice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskResult {
    pub answers: Vec<AskAnswer>,
    pub timed_out: bool,
    pub id: String,
}

/// How long a prompt waits before the host answers "blocked" itself. Zero
/// waits indefinitely, which is correct for a terminal with a person at it
/// (Go: `approvalTimeout == 0`). A headless caller must set it so a walked-
/// away user cannot wedge the session.
#[derive(Debug, Clone, Copy)]
pub struct ApprovalConfig {
    pub mode: &'static str,
    pub timeout: Duration,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            mode: mode::ASK,
            timeout: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Approved {
    allow: bool,
    session: bool,
    persist: bool,
}

enum Reply {
    Approval(Sender<Approved>),
    Ask(Sender<Vec<AskAnswer>>),
}

struct Pending {
    tool: String,
    subject: String,
    reply: Option<Reply>,
}

/// The bookkeeping half of approval (Go: `approvalManager`): it touches only
/// its own state and never calls back into the agent.
struct Manager {
    next: u64,
    prefix: String,
    pending: HashMap<String, Pending>,
    grants: Vec<(String, String)>,
}

impl Manager {
    fn new() -> Self {
        Self {
            next: 0,
            prefix: random_prefix(),
            pending: HashMap::new(),
            grants: Vec::new(),
        }
    }

    fn issue(&mut self) -> String {
        self.next += 1;
        format!("{}-{}", self.prefix, self.next)
    }

    fn grant_allows(&self, tool: &str, subject: &str) -> bool {
        self.grants.iter().any(|(t, s)| {
            reasonix_safety::permission::session_grant_matches(
                &reasonix_safety::permission::session_grant_rule_for_scope(t, s),
                tool,
                subject,
            )
        })
    }

    fn grant(&mut self, tool: &str, subject: &str) {
        if !self.grant_allows(tool, subject) {
            self.grants.push((tool.to_owned(), subject.to_owned()));
        }
    }
}

fn random_prefix() -> String {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("CSPRNG for prompt ids");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    s.chars().take(max).collect()
}

fn ask_subject(questions: &[AskQuestion]) -> String {
    let parts: Vec<String> = questions
        .iter()
        .map(|q| {
            let prompt = if q.prompt.trim().is_empty() {
                q.header.trim()
            } else {
                q.prompt.trim()
            };
            format!("{prompt}: (pending)")
        })
        .collect();
    clip(&parts.join(" | "), 240)
}

/// The host-owned approval surface. One broker serves both the Gate (tool
/// decisions) and the Asker (the `ask` tool); they share the id space and
/// the one-decision-in-flight rule.
#[derive(Clone)]
pub struct ApprovalBroker {
    manager: Arc<Mutex<Manager>>,
    /// Held across a blocking wait; answer paths never take it.
    prompt_serial: Arc<Mutex<()>>,
    config: ApprovalConfig,
    session: Arc<Session>,
    sink: Arc<dyn Sink>,
}

impl ApprovalBroker {
    pub fn new(config: ApprovalConfig, session: Arc<Session>, sink: Arc<dyn Sink>) -> Arc<Self> {
        Arc::new(Self {
            manager: Arc::new(Mutex::new(Manager::new())),
            prompt_serial: Arc::new(Mutex::new(())),
            config,
            session,
            sink,
        })
    }

    /// Answers a pending tool prompt by id. Unknown or answered ids are
    /// ignored (Go: `Approve` ignores expired identities).
    pub fn decide(&self, id: &str, input: DecisionInput) -> bool {
        let (reply, grant) = {
            let mut m = self.manager.lock().expect("approval manager lock");
            let Some(p) = m.pending.remove(id) else {
                return false;
            };
            let grant = match (&p.reply, input.allow && input.session) {
                (Some(Reply::Approval(_)), true) => Some((p.tool.clone(), p.subject.clone())),
                _ => None,
            };
            (p.reply, grant)
        };
        if let Some(Reply::Approval(tx)) = reply {
            let _ = tx.send(Approved {
                allow: input.allow,
                session: input.session,
                persist: input.persist,
            });
        }
        if let Some((tool, subject_name)) = grant {
            let mut m = self.manager.lock().expect("approval manager lock");
            m.grant(&tool, &subject_name);
        }
        true
    }

    /// Answers a pending structured question by id (Go: `AnswerQuestion`).
    pub fn answer(&self, id: &str, answers: Vec<AskAnswer>) -> bool {
        let reply = {
            let mut m = self.manager.lock().expect("approval manager lock");
            m.pending.remove(id).and_then(|p| p.reply)
        };
        match reply {
            Some(Reply::Ask(tx)) => {
                let _ = tx.send(answers);
                true
            }
            _ => false,
        }
    }

    fn record(&self, id: &str, kind: &str, tool: &str, subject: &str, outcome: &str) {
        let receipt = DecisionReceipt {
            id: id.to_owned(),
            kind: kind.to_owned(),
            tool: tool.to_owned(),
            subject: clip(subject.trim(), 240),
            outcome: outcome.to_owned(),
            ..DecisionReceipt::default()
        };
        self.session.add_decision_receipt(receipt);
        self.sink.emit(Event {
            kind: kind::NOTICE.to_owned(),
            code: notice_code::DECISION_RECEIPT.to_owned(),
            level: level::INFO.to_owned(),
            text: format!("Decision recorded: {outcome}"),
            decision_receipt: Some(reasonix_contract::eventwire::event::DecisionReceipt {
                id: id.to_owned(),
                kind: kind.to_owned(),
                tool: tool.to_owned(),
                subject: clip(subject.trim(), 240),
                outcome: outcome.to_owned(),
                ..Default::default()
            }),
            ..Event::default()
        });
    }

    fn wait_approval(&self, id: &str, rx: Receiver<Approved>) -> Result<Approved, GateError> {
        let approved = if self.config.timeout.is_zero() {
            rx.recv().ok()
        } else {
            let r = rx.recv_timeout(self.config.timeout);
            r.ok()
        };
        match approved {
            Some(a) => Ok(a),
            None => {
                let _ = self
                    .manager
                    .lock()
                    .expect("approval manager lock")
                    .pending
                    .remove(id);
                Err(GateError::aborted("approval timed out"))
            }
        }
    }

    fn wait_ask(&self, id: &str, rx: Receiver<Vec<AskAnswer>>) -> (Vec<AskAnswer>, bool) {
        let (answers, timed_out) = if self.config.timeout.is_zero() {
            (rx.recv().ok(), false)
        } else {
            match rx.recv_timeout(self.config.timeout) {
                Ok(a) => (Some(a), false),
                Err(RecvTimeoutError::Timeout) => (None, true),
                Err(RecvTimeoutError::Disconnected) => (None, false),
            }
        };
        let _ = self
            .manager
            .lock()
            .expect("approval manager lock")
            .pending
            .remove(id);
        (answers.unwrap_or_default(), timed_out)
    }

    /// The typed ask flow. A host building an `ask` tool calls this rather
    /// than the plain [`crate::Asker`] seam so a skip stays distinguishable.
    pub fn ask_questions(&self, questions: &[AskQuestion]) -> AskResult {
        let serial = self.prompt_serial.lock().unwrap_or_else(|e| e.into_inner());
        let subject = ask_subject(questions);
        let (id, rx) = {
            let mut m = self.manager.lock().expect("approval manager lock");
            let id = m.issue();
            let (tx, rx) = mpsc::channel();
            m.pending.insert(
                id.clone(),
                Pending {
                    tool: String::new(),
                    subject: subject.clone(),
                    reply: Some(Reply::Ask(tx)),
                },
            );
            (id, rx)
        };
        self.sink.emit(Event {
            kind: kind::ASK_REQUEST.to_owned(),
            ask: Some(Ask {
                id: id.clone(),
                questions: questions.to_vec(),
                ..Ask::default()
            }),
            ..Event::default()
        });
        let (answers, timed_out) = self.wait_ask(&id, rx);
        let outcome = if timed_out {
            outcome::TIMED_OUT
        } else {
            outcome::ASK_ANSWERED
        };
        self.record(&id, "ask", "", &subject, outcome);
        drop(serial);
        AskResult {
            answers,
            timed_out,
            id,
        }
    }

    fn run_approval(&self, tool: &str, subject: &str, reason: &str) -> Result<Approved, GateError> {
        let allowed = Approved {
            allow: true,
            session: false,
            persist: false,
        };
        // YOLO stands in for nobody on an ordinary tool prompt (Go:
        // `preApproved`). Fresh-human decisions and the ask tool are never
        // answered by a posture.
        if self.config.mode == mode::YOLO && !requires_fresh_human(tool) {
            return Ok(allowed);
        }
        {
            let m = self.manager.lock().expect("approval manager lock");
            if m.grant_allows(tool, subject) {
                return Ok(allowed);
            }
        }
        let serial = self.prompt_serial.lock().unwrap_or_else(|e| e.into_inner());
        let (view_tx, rx) = mpsc::channel();
        let id = {
            let mut m = self.manager.lock().expect("approval manager lock");
            if m.grant_allows(tool, subject) {
                return Ok(allowed);
            }
            let id = m.issue();
            m.pending.insert(
                id.clone(),
                Pending {
                    tool: tool.to_owned(),
                    subject: subject.to_owned(),
                    reply: Some(Reply::Approval(view_tx)),
                },
            );
            id
        };
        self.sink.emit(Event {
            kind: kind::APPROVAL_REQUEST.to_owned(),
            approval: Some(WireApproval {
                id: id.clone(),
                tool: tool.to_owned(),
                subject: subject.to_owned(),
                reason: reason.to_owned(),
                reason_code: explicit_reason_code(tool, subject),
                allows_session: !requires_fresh_human(tool),
                ..WireApproval::default()
            }),
            ..Event::default()
        });
        let approved = self.wait_approval(&id, rx);
        drop(serial);
        // A timeout is the host's own denial; the record names it so a
        // frontend can settle the card it showed (Go: `endUnansweredAsk`
        // records a receipt even when its answer never arrived).
        let outcome = match &approved {
            Ok(a) if a.allow => match (a.persist, a.session) {
                (true, _) => outcome::ALLOW_PERSIST,
                (false, true) => outcome::ALLOW_SESSION,
                (false, false) => outcome::ALLOW_ONCE,
            },
            _ => outcome::DENY,
        };
        self.record(&id, "tool", tool, subject, outcome);
        approved
    }
}

/// A call whose shape requires a fresh person cannot be answered by a
/// session grant or a remembered rule (Go: `RequiresFreshHumanApprovalTool`).
fn requires_fresh_human(tool: &str) -> bool {
    matches!(
        tool,
        "plan_approval"
            | "remember"
            | "forget"
            | "mod_check"
            | "origin_check"
            | reasonix_safety::permission::EXTEND_WRITE_PATHS
            | reasonix_safety::permission::INSTALL_SOURCE_TOOL
            | reasonix_safety::permission::NETWORK_EGRESS
            | reasonix_safety::permission::PAID_SEARCH
            | reasonix_safety::permission::MCP_SIGNIN
    )
}

/// Names why a call needs a person rather than auto or a broad rule (Go:
/// `ExplicitApprovalCode`). The code is the identity a window renders; the
/// decision stays with the host.
fn explicit_reason_code(tool: &str, subject: &str) -> String {
    if tool.eq_ignore_ascii_case("bash")
        && reasonix_safety::permission::gate::bash_subject_approval_blocker(subject)
            != reasonix_safety::permission::BashApprovalBlocker::None
    {
        return "dynamic_bash".to_owned();
    }
    if reasonix_safety::permission::rule_group::is_browser_tool(tool)
        && subject.starts_with(reasonix_safety::permission::BROWSER_SCRIPT_PREFIX)
    {
        return "browser_script".to_owned();
    }
    if reasonix_safety::permission::rule_group::is_computer_tool(tool) {
        return "computer_use".to_owned();
    }
    String::new()
}

impl Approver for ApprovalBroker {
    fn approve(
        &self,
        tool_name: &str,
        subject: &str,
        _args: &RawValue,
        policy_reason: &str,
    ) -> Result<(bool, bool, String), GateError> {
        let approved = self.run_approval(tool_name, subject, policy_reason)?;
        Ok((approved.allow, approved.persist, String::new()))
    }
}

/// The agent's per-call Gate seam backed by the composed safety gate.
pub struct InteractiveGate {
    safety: Arc<reasonix_safety::permission::Gate>,
}

impl InteractiveGate {
    pub fn new(broker: &Arc<ApprovalBroker>, base: &Policy, config: &ApprovalConfig) -> Self {
        // A posture with nobody to answer refuses every would-ask call
        // instead of opening a prompt with no decision behind it.
        let approver: Option<Arc<dyn Approver>> = match config.mode {
            mode::DONT_ASK | mode::READ_ONLY => None,
            _ => Some(broker.clone()),
        };
        Self {
            safety: Arc::new(reasonix_safety::permission::Gate::new(
                policy_for_mode(base, config.mode),
                approver,
            )),
        }
    }

    /// The safety verdict with its refusal code (Go: `permission.VerdictOf`).
    pub fn verdict(&self, tool: &str, args: &serde_json::Value, read_only: bool) -> SafetyVerdict {
        let raw = match serde_json::to_string(args) {
            Ok(s) => s,
            Err(_) => return SafetyVerdict::default(),
        };
        let Ok(raw) = RawValue::from_string(raw) else {
            return SafetyVerdict::default();
        };
        match self.safety.verdict(tool, &raw, read_only, "") {
            Ok(v) => v,
            Err(_) => SafetyVerdict {
                reason: "no answer within the approval timeout: nobody answered, so the call was blocked"
                    .to_owned(),
                code: reasonix_safety::permission::REFUSAL_UNATTENDED.to_owned(),
                ..SafetyVerdict::default()
            },
        }
    }
}

impl crate::seams::Gate for InteractiveGate {
    fn check(&self, tool: &str, args: &serde_json::Value, read_only: bool) -> Result<bool, String> {
        Ok(self.verdict(tool, args, read_only).allow)
    }

    fn check_verdict(
        &self,
        tool: &str,
        args: &serde_json::Value,
        read_only: bool,
    ) -> crate::seams::GateVerdict {
        let v = self.verdict(tool, args, read_only);
        crate::seams::GateVerdict {
            allow: v.allow,
            reason: v.reason,
            refusal_code: v.code,
        }
    }
}

/// The agent's [`crate::Asker`] seam backed by the same broker.
pub struct BrokerAsker(pub Arc<ApprovalBroker>);

impl crate::seams::Asker for BrokerAsker {
    fn ask(&self, questions: &[AskQuestion]) -> Vec<crate::seams::AskAnswer> {
        self.0
            .ask_questions(questions)
            .answers
            .into_iter()
            .map(|a| crate::seams::AskAnswer {
                question_id: a.id,
                selected: a.selected,
                value: None,
            })
            .collect()
    }
}

/// The call's subject, read from args by the owner of rule matching.
pub fn call_subject(args: &serde_json::Value) -> String {
    let Ok(raw) = serde_json::to_string(args) else {
        return String::new();
    };
    match RawValue::from_string(raw) {
        Ok(raw) => subject(&raw),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_accepts_every_alias_and_refuses_unknown() {
        for (raw, want) in [
            ("ask", mode::ASK),
            (" Auto ", mode::AUTO),
            ("allow", mode::AUTO),
            ("dontAsk", mode::DONT_ASK),
            ("dont-ask", mode::DONT_ASK),
            ("deny", mode::DONT_ASK),
            ("YOLO", mode::YOLO),
            ("bypass", mode::YOLO),
            ("readOnly", mode::READ_ONLY),
            ("read-only", mode::READ_ONLY),
        ] {
            assert_eq!(parse_mode(raw), Some(want), "{raw}");
        }
        assert_eq!(parse_mode("surprise"), None);
        assert_eq!(parse_mode(""), None);
    }

    #[test]
    fn policy_mode_shapes_only_the_fallback() {
        let base = Policy::new("ask", &[], &["bash(rm*)"], &["bash(git push*)"]);
        let auto = policy_for_mode(&base, mode::AUTO);
        assert_eq!(auto.mode, Decision::Allow);
        assert_eq!(auto.ask.len(), 1, "explicit ask stays under auto");
        assert_eq!(auto.deny.len(), 1, "deny stays under every mode");
        let ro = policy_for_mode(&base, mode::READ_ONLY);
        assert!(ro.read_only);
    }
}
