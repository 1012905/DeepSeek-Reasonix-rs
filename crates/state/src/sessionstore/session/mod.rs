//! The in-memory session (Go: session.go Session and sessionState,
//! composition.go).
//!
//! The session owns the authoritative message log plus the persistence
//! bookkeeping (baseline, rewrite epoch, dirty flags) that lets saves
//! fast-path. History paging state (SESS-099) is not carried yet.

use std::sync::{Arc, RwLock};

use reasonix_contract::agentcomposition::{Binding, Summary};
use reasonix_contract::provider::{
    DecisionReceipt, LOCAL_ONLY_TOOL_ID, LOCAL_ONLY_TOOL_NAME, Message, Role, ToolCall,
};

use crate::sessionstore::session_lease::SessionWriteAuthority;

mod baseline;
mod canonical;
mod checkpoint;
mod digest;
mod load;
mod observer;
mod recovery;
mod save;
pub(crate) mod storage_eq;

pub use baseline::PersistedState;
pub use canonical::canonical_session_path;
pub(crate) use canonical::{canonical_session_save_path, with_session_save_path_lock};
pub(crate) use checkpoint::{session_artifact_exists, write_session_messages};
pub(crate) use digest::message_for_session_identity;
pub use digest::{digest_and_size_session_messages, digest_session_messages};
pub use load::{
    is_cleanup_pending, is_visible_session, load_session, load_session_messages,
    load_session_unlocked, prepare_session_path,
};
#[cfg(test)]
pub(crate) use load::{
    load_session_messages_from_jsonl, load_session_messages_from_jsonl_keeping_decoded_tail,
};
pub use observer::{SessionPersistEvent, SessionPersistObserver};
pub(crate) use recovery::recovery_parent_stem;
pub use recovery::{RecoveryBranchError, RecoveryBranchInfo, RecoveryBranchOptions};
pub(crate) use save::SessionSaveMode;
pub use save::{LoadError, SaveError, WriteMessagesError};

/// The in-memory session (Go: Session).
///
/// The authoritative log lives behind a RwLock; every method locks
/// exactly once. A poisoned lock is recovered from because no
/// critical section panics while holding it (Go relies on the same
/// property and simply ignores poisoning).
pub struct Session {
    state: RwLock<SessionState>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Diagnostics only: never block on a lock the current thread or a
        // panicked writer already holds.
        match self.state.try_read() {
            Ok(st) => f
                .debug_struct("Session")
                .field("version", &st.version)
                .field("rewrite_version", &st.rewrite_version)
                .field("messages", &st.messages.len())
                .finish_non_exhaustive(),
            Err(_) => f.debug_struct("Session").finish_non_exhaustive(),
        }
    }
}

/// Internal mutable state (Go: sessionState minus the mutex).
#[derive(Default)]
struct SessionState {
    messages: Vec<Message>,
    version: u64,
    rewrite_version: i64,
    persisted_rewrite_version: i64,
    persisted: baseline::SessionPersistState,
    normalized_dirty: bool,
    event_log_damaged: bool,
    raw_messages: Vec<Message>,
    pending_content_reasons: Vec<String>,
    persist_observer: Option<Arc<dyn SessionPersistObserver>>,
    recovery_lane: String,
    write_auth: Option<Arc<SessionWriteAuthority>>,
    auth_required: bool,
}

impl Session {
    /// Creates a session with an optional leading system message (Go:
    /// NewSession). The initial version is 0.
    pub fn new(system: &str) -> Session {
        let mut st = SessionState::default();
        if !system.is_empty() {
            st.messages.push(Message {
                role: Role::SYSTEM,
                content: system.to_string(),
                ..Default::default()
            });
        }
        Session {
            state: RwLock::new(st),
        }
    }

    /// Builds a session carrying exactly `records` with zeroed counters
    /// and no persistence baseline (Go: DecodeNativeArchive's
    /// `s := NewSession(""); s.Messages = archive.Records`).
    pub(crate) fn from_archive_records(records: Vec<Message>) -> Session {
        Session {
            state: RwLock::new(SessionState {
                messages: records,
                ..SessionState::default()
            }),
        }
    }

    /// Read-locks the state, recovering from a poisoned lock the way
    /// the Go code does by never panicking inside a critical section.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, SessionState> {
        self.state.read().unwrap_or_else(|err| err.into_inner())
    }

    /// Write-locks the state with the same poison recovery.
    fn write(&self) -> std::sync::RwLockWriteGuard<'_, SessionState> {
        self.state.write().unwrap_or_else(|err| err.into_inner())
    }

    /// Bind a write authority (Go: `BindWriteAuthority`): a `Some`
    /// token permanently arms the requirement; `None` only clears the
    /// bound token.
    pub fn bind_write_authority(&self, authority: Option<Arc<SessionWriteAuthority>>) {
        let mut state = self.write();
        if authority.is_some() {
            state.auth_required = true;
        }
        state.write_auth = authority;
    }

    /// Arm the write-authority requirement for future saves (Go:
    /// `RequireWriteAuthority`).
    pub fn require_write_authority(&self) {
        self.write().auth_required = true;
    }

    /// Whether saves require a write authority (Go:
    /// `WriteAuthorityRequired`).
    pub fn write_authority_required(&self) -> bool {
        self.read().auth_required
    }

    /// The bound write authority, if any (Go: `WriteAuthority`).
    pub fn write_authority(&self) -> Option<Arc<SessionWriteAuthority>> {
        self.read().write_auth.clone()
    }

    /// Clear the bound write authority (Go: `ClearWriteAuthority`):
    /// the armed requirement itself stays.
    pub fn clear_write_authority(&self) {
        self.write().write_auth = None;
    }

    /// Appends a message (Go: Add).
    pub fn add(&self, m: Message) {
        self.add_indexed(m);
    }

    /// Appends a message and returns the index it took (Go:
    /// AddIndexed). A user message starts any bound composition.
    pub fn add_indexed(&self, m: Message) -> usize {
        let mut st = self.write();
        if m.role == Role::USER && !m.local_only {
            mark_composition_started_locked(&mut st);
        }
        st.messages.push(m);
        st.version += 1;
        st.messages.len() - 1
    }

    /// Attaches a decision receipt to the newest eligible assistant
    /// message without inserting a standalone one (Go:
    /// AddDecisionReceipt). The scan stops at the latest user
    /// message; before any assistant message exists the receipt lands
    /// on a local-only sentinel so it never reaches the provider.
    pub fn add_decision_receipt(&self, receipt: DecisionReceipt) {
        let mut st = self.write();
        let mut target = None;
        for i in (0..st.messages.len()).rev() {
            let m = &st.messages[i];
            if m.role == Role::USER && !m.local_only {
                break;
            }
            if m.role == Role::ASSISTANT && !m.local_only {
                target = Some(i);
                break;
            }
        }
        match target {
            Some(i) => {
                let mut receipts = st.messages[i].decision_receipts.clone();
                receipts.push(Some(receipt));
                st.messages[i].decision_receipts = receipts;
                // The snapshot on disk already contains this assistant
                // message, so the save must rewrite it rather than
                // append past it.
                st.rewrite_version += 1;
                st.version += 1;
            }
            None => {
                st.messages.push(Message {
                    role: Role::TOOL,
                    tool_call_id: LOCAL_ONLY_TOOL_ID.to_string(),
                    name: LOCAL_ONLY_TOOL_NAME.to_string(),
                    local_only: true,
                    decision_receipt: Some(receipt),
                    ..Default::default()
                });
                st.version += 1;
            }
        }
    }

    /// Refreshes the diff summary of a tool call on the newest
    /// assistant message carrying it (Go: UpdateToolCallPreview).
    pub fn update_tool_call_preview(&self, call: &ToolCall) -> bool {
        if call.id.is_empty() {
            return false;
        }
        let mut st = self.write();
        for i in (0..st.messages.len()).rev() {
            if st.messages[i].role != Role::ASSISTANT {
                continue;
            }
            let Some(j) = st.messages[i]
                .tool_calls
                .iter()
                .position(|c| c.id == call.id)
            else {
                continue;
            };
            let mut calls = st.messages[i].tool_calls.clone();
            calls[j].diff = call.diff.clone();
            calls[j].added = call.added;
            calls[j].removed = call.removed;
            st.messages[i].tool_calls = calls;
            st.rewrite_version += 1;
            st.version += 1;
            return true;
        }
        false
    }

    /// Records the resolved identity of a tool call (Go:
    /// UpdateToolCallResolution).
    pub fn update_tool_call_resolution(&self, call: &ToolCall) -> bool {
        if call.id.is_empty() || call.resolved_read_only.is_none() {
            return false;
        }
        let mut st = self.write();
        for i in (0..st.messages.len()).rev() {
            if st.messages[i].role != Role::ASSISTANT {
                continue;
            }
            let Some(j) = st.messages[i]
                .tool_calls
                .iter()
                .position(|c| c.id == call.id)
            else {
                continue;
            };
            let mut calls = st.messages[i].tool_calls.clone();
            calls[j].resolved_name = call.resolved_name.clone();
            calls[j].capability_id = call.capability_id.clone();
            calls[j].resolved_read_only = call.resolved_read_only;
            st.messages[i].tool_calls = calls;
            st.rewrite_version += 1;
            st.version += 1;
            return true;
        }
        false
    }

    /// Replaces the whole log, preserving any bound composition (Go:
    /// Replace). History paging state resets (deferred, SESS-029).
    pub fn replace(&self, msgs: Vec<Message>) {
        let mut st = self.write();
        st.messages = preserve_composition_locked(&st.messages, msgs);
        st.version += 1;
    }

    /// Replaces the log as a content rewrite and records why (Go:
    /// Rewrite).
    pub fn rewrite(&self, msgs: Vec<Message>, reason: &str) {
        let mut st = self.write();
        st.messages = preserve_composition_locked(&st.messages, msgs);
        st.rewrite_version += 1;
        st.version += 1;
        if !reason.is_empty() {
            st.pending_content_reasons.push(reason.to_string());
        }
    }

    /// Replaces local-only metadata without touching provider-visible
    /// content (Go: ReplaceLocalMetadata).
    pub fn replace_local_metadata(&self, msgs: Vec<Message>) {
        let mut st = self.write();
        st.messages = preserve_composition_locked(&st.messages, msgs);
        st.rewrite_version += 1;
        st.version += 1;
    }

    /// Returns and clears the recorded content-rewrite reasons (Go:
    /// DrainContentRewriteReasons).
    pub fn drain_content_rewrite_reasons(&self) -> Vec<String> {
        let mut st = self.write();
        std::mem::take(&mut st.pending_content_reasons)
    }

    /// Records a reason a content rewrite happened (Go:
    /// NoteContentRewrite).
    pub fn note_content_rewrite(&self, reason: &str) {
        if reason.is_empty() {
            return;
        }
        let mut st = self.write();
        st.pending_content_reasons.push(reason.to_string());
    }

    /// Copies the whole log (Go: Snapshot).
    pub fn snapshot(&self) -> Vec<Message> {
        self.snapshot_with_version().0
    }

    /// Number of stored messages (Go: Len).
    pub fn len(&self) -> usize {
        self.read().messages.len()
    }

    /// Whether the session stores no messages.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copies messages in the half-open index range, clamped to the
    /// stored log (Go: MessageRange).
    pub fn message_range(&self, start: i64, end: i64) -> Vec<Message> {
        let st = self.read();
        let total = st.messages.len() as i64;
        let start = start.max(0);
        let end = end.min(total);
        if start >= end {
            return Vec::new();
        }
        st.messages[start as usize..end as usize].to_vec()
    }

    /// Clones the session with a different log (Go:
    /// CloneWithMessages). The persistence baseline and rewrite epoch
    /// carry over so the clone can still fast-path; the observer, the
    /// raw-message cache, and the recovery lane do not.
    pub fn clone_with_messages(&self, msgs: Vec<Message>) -> Session {
        let st = self.read();
        clone_locked(&st, msgs)
    }

    /// Clones with the log only when it is the same persisted history,
    /// optionally with a refreshed leading system prompt (Go:
    /// CloneWithMessagesIfCompatible).
    pub fn clone_with_messages_if_compatible(&self, msgs: Vec<Message>) -> Option<Session> {
        let st = self.read();
        if !storage_eq::messages_compatible_for_storage_baseline(&st.messages, &msgs) {
            return None;
        }
        Some(clone_locked(&st, msgs))
    }

    /// Messages plus the version and rewrite epoch they were captured
    /// under, in one lock window (Go: SnapshotWithVersion).
    pub fn snapshot_with_version(&self) -> (Vec<Message>, u64, i64) {
        let st = self.read();
        (st.messages.clone(), st.version, st.rewrite_version)
    }

    /// Messages plus the version they were captured under (Go:
    /// SnapshotMessagesVersion).
    pub fn snapshot_messages_version(&self) -> (Vec<Message>, u64) {
        let st = self.read();
        (st.messages.clone(), st.version)
    }

    /// Copies the log from an index on, with the total length and
    /// counters (Go: SnapshotTail).
    pub fn snapshot_tail(&self, from: i64) -> (Vec<Message>, usize, u64, i64) {
        let st = self.read();
        let total = st.messages.len();
        let tail = if from >= 0 && (from as usize) < total {
            st.messages[from as usize..].to_vec()
        } else {
            Vec::new()
        };
        (tail, total, st.version, st.rewrite_version)
    }

    /// The append/rewrite counter (Go: TranscriptVersion).
    pub fn transcript_version(&self) -> u64 {
        self.read().version
    }

    /// The rewrite counter (Go: RewriteVersion).
    pub fn rewrite_version(&self) -> i64 {
        self.read().rewrite_version
    }

    /// Whether the history was rewritten in memory since the last
    /// full save (Go: NeedsRewriteSave).
    pub fn needs_rewrite_save(&self) -> bool {
        let st = self.read();
        st.rewrite_version > st.persisted_rewrite_version
    }

    /// Whether the session differs from what was last persisted to
    /// path (Go: HasUnsavedChanges). An undigestable log counts as
    /// unsaved rather than failing the caller.
    pub fn has_unsaved_changes(&self, path: &str) -> bool {
        if path.trim().is_empty() {
            return false;
        }
        let (msgs, _, rewrite_version) = self.snapshot_with_version();
        let Ok(digest) = digest::digest_session_messages(&msgs) else {
            return true;
        };
        let key = canonical::canonical_session_save_path(path);
        let st = self.read();
        if !st.persisted.ok || st.persisted.path != key {
            return true;
        }
        if st.normalized_dirty
            || st.event_log_damaged
            || rewrite_version > st.persisted_rewrite_version
        {
            return true;
        }
        digest != st.persisted.digest
    }

    /// Bumps the rewrite version (Go: IncrementRewrite).
    pub fn increment_rewrite(&self) {
        let mut st = self.write();
        st.rewrite_version += 1;
        st.version += 1;
    }

    /// Whether the session carries at least one non-system message
    /// (Go: HasContent).
    pub fn has_content(&self) -> bool {
        let st = self.read();
        st.messages.iter().any(|m| m.role != Role::SYSTEM)
    }

    /// Whether the log starts with a system message (Go:
    /// HasSystemMessage).
    pub fn has_system_message(&self) -> bool {
        let st = self.read();
        !st.messages.is_empty() && st.messages[0].role == Role::SYSTEM
    }

    /// Summary of the first system message that carries a composition
    /// binding (Go: CompositionSummary). System messages without a
    /// binding are skipped, not stopped at.
    pub fn composition_summary(&self) -> Option<Summary> {
        let st = self.read();
        st.messages
            .iter()
            .filter(|m| m.role == Role::SYSTEM)
            .find_map(|m| m.agent_composition.as_ref())
            .map(|binding| binding.summary())
    }

    /// The first system message's composition binding, cloned (Go:
    /// Composition).
    pub fn composition(&self) -> Option<Binding> {
        let st = self.read();
        st.messages
            .iter()
            .filter(|m| m.role == Role::SYSTEM)
            .find_map(|m| m.agent_composition.as_ref())
            .cloned()
    }

    /// Attaches a live composition binding to the first system
    /// message, prepending one when absent (Go: BindComposition).
    pub fn bind_composition(&self, binding: &Binding) {
        let mut st = self.write();
        if let Some(i) = st.messages.iter().position(|m| m.role == Role::SYSTEM) {
            st.messages[i].agent_composition = Some(binding.clone());
            st.rewrite_version += 1;
            st.version += 1;
            return;
        }
        st.messages.insert(
            0,
            Message {
                role: Role::SYSTEM,
                agent_composition: Some(binding.clone()),
                ..Default::default()
            },
        );
        st.rewrite_version += 1;
        st.version += 1;
    }

    /// The unnormalized messages captured before a normalization
    /// rewrite (Go: rawMessages); cleared by a verified save.
    pub fn raw_messages(&self) -> Vec<Message> {
        self.read().raw_messages.clone()
    }

    /// The isolated recovery lane name, if any (Go: recoveryLane).
    pub fn recovery_lane(&self) -> String {
        self.read().recovery_lane.clone()
    }

    /// Records the isolated recovery lane this session belongs to
    /// (Go: recoveryLane assignment during recovery).
    pub fn set_recovery_lane(&self, lane: &str) {
        self.write().recovery_lane = lane.to_string();
    }
}

/// Shared body of the clone family (Go: the construction at the end
/// of CloneWithMessages / CloneWithMessagesIfCompatible).
fn clone_locked(st: &SessionState, msgs: Vec<Message>) -> Session {
    let mut version = st.version;
    if !storage_eq::messages_equal_for_storage_list(&st.messages, &msgs) {
        version += 1;
    }
    Session {
        state: RwLock::new(SessionState {
            messages: msgs,
            version,
            rewrite_version: st.rewrite_version,
            persisted_rewrite_version: st.persisted_rewrite_version,
            persisted: st.persisted.clone(),
            normalized_dirty: st.normalized_dirty,
            event_log_damaged: st.event_log_damaged,
            pending_content_reasons: st.pending_content_reasons.clone(),
            ..Default::default()
        }),
    }
}

/// Starts the first bound composition (Go:
/// markCompositionStartedLocked). Any role counts; only the rewrite
/// epoch moves.
fn mark_composition_started_locked(st: &mut SessionState) {
    let Some(i) = st
        .messages
        .iter()
        .position(|m| m.agent_composition.as_ref().is_some_and(|b| !b.started))
    else {
        return;
    };
    let mut binding = st.messages[i]
        .agent_composition
        .clone()
        .expect("positioned on a present binding");
    binding.started = true;
    st.messages[i].agent_composition = Some(binding);
    st.rewrite_version += 1;
}

/// Carries the current composition binding into a replacement log
/// (Go: preserveCompositionLocked). Without a binding the new log is
/// returned untouched.
fn preserve_composition_locked(current: &[Message], msgs: Vec<Message>) -> Vec<Message> {
    let Some(binding) = current.iter().find_map(|m| m.agent_composition.as_ref()) else {
        return msgs;
    };
    let binding = binding.clone();
    let mut msgs = msgs;
    for m in &mut msgs {
        m.agent_composition = None;
    }
    match msgs.iter().position(|m| m.role == Role::SYSTEM) {
        Some(i) => msgs[i].agent_composition = Some(binding),
        None => msgs.insert(
            0,
            Message {
                role: Role::SYSTEM,
                agent_composition: Some(binding),
                ..Default::default()
            },
        ),
    }
    msgs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn assistant_msg(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn new_session_builds_system_message() {
        let s = Session::new("sys prompt");
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
        assert!(s.has_system_message());
        let st = s.snapshot();
        assert_eq!(st[0].role, Role::SYSTEM);
        assert_eq!(st[0].content, "sys prompt");
        assert_eq!(s.transcript_version(), 0);

        let empty = Session::new("");
        assert!(empty.is_empty());
        assert!(!empty.has_system_message());
        assert!(!empty.has_content());
    }

    #[test]
    fn add_indexes_and_bumps_version() {
        let s = Session::new("");
        assert_eq!(s.add_indexed(user_msg("one")), 0);
        assert_eq!(s.add_indexed(user_msg("two")), 1);
        assert_eq!(s.len(), 2);
        assert_eq!(s.transcript_version(), 2);
        assert_eq!(s.snapshot()[1].content, "two");
    }

    #[test]
    fn add_decision_receipt_attaches_to_newest_assistant() {
        let s = Session::new("");
        s.add(user_msg("q"));
        s.add(assistant_msg("a"));
        s.add(user_msg("q2"));
        s.add(assistant_msg("a2"));
        s.add_decision_receipt(DecisionReceipt::default());
        let st = s.snapshot();
        assert_eq!(st[3].decision_receipts.len(), 1);
        assert!(st[3].decision_receipts[0].is_some());
        assert_eq!(s.rewrite_version(), 1);
        assert_eq!(s.transcript_version(), 5);
    }

    #[test]
    fn add_decision_receipt_stops_at_user_and_falls_back() {
        let s = Session::new("");
        s.add(assistant_msg("older"));
        s.add(user_msg("newer"));
        s.add_decision_receipt(DecisionReceipt::default());
        let st = s.snapshot();
        // The scan stops at the user message, so the receipt cannot
        // reach the older assistant turn and lands on a sentinel.
        assert_eq!(st.len(), 3);
        assert!(st[0].decision_receipts.is_empty());
        assert_eq!(st[2].role, Role::TOOL);
        assert!(st[2].local_only);
        assert_eq!(st[2].name, LOCAL_ONLY_TOOL_NAME);
        assert_eq!(st[2].tool_call_id, LOCAL_ONLY_TOOL_ID);
        assert!(st[2].decision_receipt.is_some());
        assert_eq!(s.rewrite_version(), 0);
        assert_eq!(s.transcript_version(), 3);
    }

    #[test]
    fn add_decision_receipt_fallback_sentinel_before_any_assistant() {
        let s = Session::new("");
        s.add_decision_receipt(DecisionReceipt::default());
        let st = s.snapshot();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].role, Role::TOOL);
        assert!(st[0].local_only);
        assert!(st[0].decision_receipt.is_some());
        assert_eq!(s.rewrite_version(), 0);
        assert_eq!(s.transcript_version(), 1);
    }

    #[test]
    fn update_tool_call_preview_updates_latest_match() {
        let s = Session::new("");
        let mut first = assistant_msg("a");
        first.tool_calls.push(ToolCall {
            id: "t1".to_string(),
            ..Default::default()
        });
        s.add(first);
        let mut second = assistant_msg("b");
        second.tool_calls.push(ToolCall {
            id: "t1".to_string(),
            ..Default::default()
        });
        second.tool_calls.push(ToolCall {
            id: "t2".to_string(),
            ..Default::default()
        });
        s.add(second);

        let call = ToolCall {
            id: "t1".to_string(),
            diff: "d".to_string(),
            added: 3,
            removed: 1,
            ..Default::default()
        };
        assert!(s.update_tool_call_preview(&call));
        let st = s.snapshot();
        // The newest assistant message wins.
        assert_eq!(st[1].tool_calls[0].diff, "d");
        assert_eq!(st[1].tool_calls[0].added, 3);
        assert_eq!(st[1].tool_calls[0].removed, 1);
        assert_eq!(st[0].tool_calls[0].diff, "");
        assert_eq!(s.rewrite_version(), 1);
        assert_eq!(s.transcript_version(), 3);

        let missing = ToolCall {
            id: "nope".to_string(),
            ..Default::default()
        };
        assert!(!s.update_tool_call_preview(&missing));
        assert!(!s.update_tool_call_preview(&ToolCall::default()));
        assert_eq!(s.transcript_version(), 3);
    }

    #[test]
    fn update_tool_call_resolution_records_identity() {
        let s = Session::new("");
        let mut a = assistant_msg("a");
        a.tool_calls.push(ToolCall {
            id: "t1".to_string(),
            ..Default::default()
        });
        s.add(a);

        let call = ToolCall {
            id: "t1".to_string(),
            resolved_name: "read_file".to_string(),
            capability_id: "cap".to_string(),
            resolved_read_only: Some(true),
            ..Default::default()
        };
        assert!(s.update_tool_call_resolution(&call));
        let tc = &s.snapshot()[0].tool_calls[0];
        assert_eq!(tc.resolved_name, "read_file");
        assert_eq!(tc.capability_id, "cap");
        assert_eq!(tc.resolved_read_only, Some(true));
        assert_eq!(s.rewrite_version(), 1);

        // Without a resolved read-only flag the update is refused.
        let partial = ToolCall {
            id: "t1".to_string(),
            ..Default::default()
        };
        assert!(!s.update_tool_call_resolution(&partial));
        assert_eq!(s.transcript_version(), 2);
    }

    #[test]
    fn replace_preserves_composition_and_bumps_version() {
        let s = Session::new("sys");
        s.bind_composition(&Binding::default());
        let rewrite_before = s.rewrite_version();
        s.replace(vec![user_msg("next")]);
        let st = s.snapshot();
        // The composition rides to the front as a system message.
        assert_eq!(st.len(), 2);
        assert_eq!(st[0].role, Role::SYSTEM);
        assert!(st[0].agent_composition.is_some());
        assert_eq!(st[1].content, "next");
        assert_eq!(s.transcript_version(), 2);
        assert_eq!(s.rewrite_version(), rewrite_before);
    }

    #[test]
    fn rewrite_records_reason_and_bumps_rewrite_epoch() {
        let s = Session::new("");
        s.rewrite(vec![user_msg("trimmed")], "compaction");
        assert_eq!(s.rewrite_version(), 1);
        assert_eq!(s.transcript_version(), 1);
        assert_eq!(
            s.drain_content_rewrite_reasons(),
            vec!["compaction".to_string()]
        );
        assert!(s.drain_content_rewrite_reasons().is_empty());
        s.note_content_rewrite("rewind");
        s.note_content_rewrite("");
        assert_eq!(
            s.drain_content_rewrite_reasons(),
            vec!["rewind".to_string()]
        );
    }

    #[test]
    fn replace_local_metadata_bumps_rewrite() {
        let s = Session::new("");
        s.add(user_msg("q"));
        s.replace_local_metadata(vec![user_msg("q")]);
        assert_eq!(s.rewrite_version(), 1);
        assert_eq!(s.transcript_version(), 2);
    }

    #[test]
    fn message_range_clamps() {
        let s = Session::new("");
        s.add(user_msg("a"));
        s.add(user_msg("b"));
        s.add(user_msg("c"));
        assert_eq!(s.message_range(-1, 2).len(), 2);
        assert_eq!(s.message_range(1, 99).len(), 2);
        assert_eq!(s.message_range(1, 99)[0].content, "b");
        assert!(s.message_range(2, 1).is_empty());
        assert!(s.message_range(5, 99).is_empty());
        assert_eq!(s.message_range(0, 3).len(), 3);
    }

    #[test]
    fn snapshot_tail_returns_tail_and_counters() {
        let s = Session::new("");
        s.add(user_msg("a"));
        s.add(user_msg("b"));
        let (tail, total, version, rewrite) = s.snapshot_tail(1);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].content, "b");
        assert_eq!(total, 2);
        assert_eq!(version, 2);
        assert_eq!(rewrite, 0);
        let (empty, total, _, _) = s.snapshot_tail(5);
        assert!(empty.is_empty());
        assert_eq!(total, 2);
        let (all, _, _, _) = s.snapshot_tail(0);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn clone_with_messages_carries_baseline_and_bumps_version_on_change() {
        let s = Session::new("");
        s.add(user_msg("q"));
        s.mark_persisted(
            "p.jsonl",
            &[1u8; 32],
            s.transcript_version(),
            4,
            s.rewrite_version(),
        );

        let same = s.clone_with_messages(s.snapshot());
        assert_eq!(same.transcript_version(), 1);
        let state = same.persisted_state("p.jsonl").unwrap();
        assert_eq!(state.revision, 4);

        let changed = s.clone_with_messages(vec![user_msg("other")]);
        assert_eq!(changed.transcript_version(), 2);
    }

    #[test]
    fn clone_with_messages_if_compatible_guards_history() {
        let s = Session::new("sys");
        s.add(user_msg("q"));
        assert_eq!(s.transcript_version(), 1);

        let same = s.clone_with_messages_if_compatible(s.snapshot());
        assert!(same.is_some());
        assert_eq!(same.unwrap().transcript_version(), 1);

        // A refreshed leading system prompt is compatible...
        let mut refreshed = s.snapshot();
        refreshed[0].content = "sys v2".to_string();
        let ok = s.clone_with_messages_if_compatible(refreshed);
        assert!(ok.is_some());
        assert_eq!(ok.unwrap().transcript_version(), 2);

        // ...but a different history is not.
        let bad = s.clone_with_messages_if_compatible(vec![user_msg("different")]);
        assert!(bad.is_none());
    }

    #[test]
    fn needs_rewrite_save_and_increment_rewrite() {
        let s = Session::new("");
        assert!(!s.needs_rewrite_save());
        s.increment_rewrite();
        assert!(s.needs_rewrite_save());
        assert_eq!(s.transcript_version(), 1);
        assert_eq!(s.rewrite_version(), 1);
    }

    #[test]
    fn has_unsaved_changes_matrix() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let path = dir.path().join("s.jsonl");
        let path_str = path.to_str().unwrap();

        let s = Session::new("sys");
        s.add(user_msg("q"));
        assert!(!s.has_unsaved_changes(""));
        assert!(s.has_unsaved_changes(path_str));

        let (msgs, version, rewrite) = s.snapshot_with_version();
        let digest = digest_session_messages(&msgs).unwrap();
        s.mark_persisted(path_str, &digest, version, 7, rewrite);
        assert!(!s.has_unsaved_changes(path_str));

        let state = s.persisted_state(path_str).unwrap();
        assert_eq!(state.revision, 7);
        assert_eq!(
            state.digest_hex,
            crate::sessionstore::eventlog::writer_id::digest_string(&digest)
        );
        assert!(state.append_only_tail);
        assert!(state.unchanged_since_persisted);

        s.add(user_msg("more"));
        assert!(s.has_unsaved_changes(path_str));
    }

    #[test]
    fn has_content_and_has_system_message() {
        let s = Session::new("sys");
        assert!(!s.has_content());
        assert!(s.has_system_message());
        s.add(user_msg("q"));
        assert!(s.has_content());

        let t = Session::new("");
        t.add(assistant_msg("a"));
        assert!(t.has_content());
        assert!(!t.has_system_message());
    }

    #[test]
    fn composition_bind_mark_and_preserve() {
        // Without a system message binding prepends one.
        let s = Session::new("");
        let binding = Binding {
            started: false,
            ..Default::default()
        };
        s.bind_composition(&binding);
        let st = s.snapshot();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].role, Role::SYSTEM);
        assert!(st[0].agent_composition.is_some());
        assert_eq!(s.rewrite_version(), 1);
        assert_eq!(s.transcript_version(), 1);

        assert!(s.composition_summary().is_some());
        assert_eq!(s.composition().unwrap().summary(), binding.summary());

        // A user message starts the composition: only the rewrite
        // epoch moves beyond the add itself.
        s.add(user_msg("q"));
        assert!(s.composition().unwrap().started);
        assert_eq!(s.rewrite_version(), 2);
        assert_eq!(s.transcript_version(), 2);

        // Binding lands on an existing system message.
        let t = Session::new("sys");
        t.bind_composition(&binding);
        let tst = t.snapshot();
        assert_eq!(tst.len(), 1);
        assert!(tst[0].agent_composition.is_some());
        assert_eq!(t.rewrite_version(), 1);
        assert_eq!(t.transcript_version(), 1);

        // Replace keeps the binding on the system message.
        t.replace(vec![user_msg("next")]);
        let tst = t.snapshot();
        assert_eq!(tst.len(), 2);
        assert_eq!(tst[0].role, Role::SYSTEM);
        assert!(tst[0].agent_composition.is_some());

        // Rewrite clears stray bindings from the new log and keeps
        // exactly one on the leading system message.
        let mut stray = user_msg("u");
        stray.agent_composition = Some(Binding::default());
        t.rewrite(vec![stray, user_msg("v")], "prune");
        let tst = t.snapshot();
        assert_eq!(tst.len(), 3);
        assert!(tst[0].agent_composition.is_some());
        assert!(tst[1].agent_composition.is_none());
        assert!(tst[2].agent_composition.is_none());
        assert_eq!(t.drain_content_rewrite_reasons(), vec!["prune".to_string()]);
    }
}
