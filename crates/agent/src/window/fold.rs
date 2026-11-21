//! The fold transaction (Go: `compact_projection.go` `compactToProjection`,
//! `compact.go` planning half, `compact_commit.go`, `compact_group.go`,
//! `candidate_holds.go`, `checkpoint_rejected.go`, `kept_projection.go`,
//! `fold_boundary.go`).
//!
//! One content-driven checkpoint: plan the region, split what stays from
//! what is archived, land the fold verbatim in an archive file, judge the
//! candidate against the ceilings, then CAS-install the projection under
//! the compaction guard. The canonical transcript is never rewritten; a
//! noop folds nothing and names why. The archive half — transcript
//! rendering, the archive write, the pointer text — is
//! [`crate::window::fold_archive`].
//!
//! Named deferred: the extension interceptors (Go `PreCompact` /
//! `interceptCompactionPrepare` / `interceptCompactionComplete`), so
//! [`FoldError::HookRefused`] has no producer yet; the todo-identity tail
//! and the telemetry card, host-tail surfaces the composition wave owns.

use std::collections::HashSet;
use std::sync::{Arc, MutexGuard, Weak};

use reasonix_contract::eventwire::event::{Compaction, Event};
use reasonix_contract::provider::{Message, Role, model_messages, projection_messages};
use reasonix_state::sessionstore::compaction_state::{
    COMPACTION_STATE_SCHEMA_CURRENT, CompactionState, ContextMaintenanceReceipt, ContextProjection,
    covered_prefix_hash, provider_visible_fingerprint,
};

use crate::seams::fold_executor::{FoldError, FoldExecutor, FoldRequest, FoldResult};
use crate::window::maintenance::{CompactionNoopReason, failure_code, noop};

use super::Window;
use super::candidate::{accept_checkpoint_candidate, attribute, candidate_holds, fold_cannot_fit};
use super::estimate;
use super::fold_archive::{
    FoldSummary, fold_to_archive, format_summary_message, summary_content_hash,
};
use super::{is_compaction_summary, model_visible_from_projection, projection_valid};

/// nothing else (Go: `pinnedPrefixLen`). Every conversational message may
/// be archived.
pub fn pinned_prefix_len(msgs: &[Message]) -> usize {
    let mut i = 0;
    if i < msgs.len() && msgs[i].role == Role::SYSTEM {
        i += 1;
    }
    i
}

/// How far into `msgs` every tool call has its result (Go:
/// `closedPrefixEnd`). A fold may end only at such a point: anywhere else
/// hands the provider an assistant turn whose calls never resolve.
pub fn closed_prefix_end(msgs: &[Message]) -> usize {
    let mut open: HashSet<String> = HashSet::new();
    let mut end = 0;
    for (i, m) in msgs.iter().enumerate() {
        if m.role == Role::TOOL && !m.tool_call_id.is_empty() {
            open.remove(&m.tool_call_id);
        }
        for call in &m.tool_calls {
            open.insert(call.id.clone());
        }
        if open.is_empty() {
            end = i + 1;
        }
    }
    end
}

/// Go: `planCompaction` — everything after the system prompt; the caller
/// trims to the closed prefix of an active turn.
fn plan_compaction(msgs: &[Message], min: usize) -> Option<(usize, usize)> {
    let head = pinned_prefix_len(msgs);
    let start = msgs.len();
    if start - head < min {
        return None;
    }
    Some((head, start))
}

/// Returns `[head:start]` to fold (Go: `planFoldRegion`). A turn long
/// enough to reach the trigger has to be foldable from inside, so it ends
/// where every call the turn issued has its result — not a transaction in
/// flight.
pub(crate) fn plan_fold_region(
    window: &Window,
    msgs: &[Message],
) -> (usize, usize, bool, CompactionNoopReason) {
    let planned =
        plan_compaction(msgs, super::MIN_COMPACT_MESSAGES).or_else(|| plan_compaction(msgs, 1));
    let Some((head, mut start)) = planned else {
        return (0, 0, false, noop::NO_FOLDABLE_REGION);
    };
    let active = window.active_turn_start(msgs);
    if active >= head as i64 && active < start as i64 {
        let active = active as usize;
        let closed = closed_prefix_end(&msgs[active + 1..]);
        let limit = active + 1 + closed;
        if limit < start {
            start = limit;
        }
        if start <= head || (closed == 0 && active == head) {
            return (head, start, false, noop::ACTIVE_TURN_BOUNDARY);
        }
    }
    if start <= head {
        return (head, start, false, noop::NO_FOLDABLE_REGION);
    }
    (head, start, true, "")
}

/// Go: `splittable` — a turn's calls can be told apart by id. Without
/// distinct ids a result cannot be matched to its call, so the turn stays
/// whole.
fn splittable(m: &Message) -> bool {
    let mut seen = HashSet::with_capacity(m.tool_calls.len());
    for tc in &m.tool_calls {
        if tc.id.is_empty() || !seen.insert(tc.id.clone()) {
            return false;
        }
    }
    true
}

fn tool_call_ids(m: &Message) -> HashSet<String> {
    m.tool_calls.iter().map(|tc| tc.id.clone()).collect()
}

/// Retains a calling turn with every result it received (Go:
/// `keepToolCallGroup`).
fn keep_tool_call_group(region: &[Message], keep: &mut [bool], assistant_index: usize) {
    if assistant_index >= region.len() {
        return;
    }
    let m = &region[assistant_index];
    if m.role != Role::ASSISTANT || m.tool_calls.is_empty() {
        return;
    }
    keep[assistant_index] = true;
    let ids = tool_call_ids(m);
    let mut j = assistant_index + 1;
    while j < region.len() && region[j].role == Role::TOOL {
        if ids.contains(&region[j].tool_call_id) {
            keep[j] = true;
        }
        j += 1;
    }
}

/// Retains the turn that issued the call a kept result answers (Go:
/// `keepAnsweredCall`). Its other results stay free to fold when the calls
/// can be told apart; `split_kept_turn` then hands each side only its own
/// calls.
fn keep_answered_call(region: &[Message], keep: &mut [bool], assistant_index: i64) {
    if !(0..region.len() as i64).contains(&assistant_index) {
        return;
    }
    let idx = assistant_index as usize;
    if splittable(&region[idx]) {
        keep[idx] = true;
        return;
    }
    keep_tool_call_group(region, keep, idx);
}

fn find_tool_caller(region: &[Message], tool_index: usize, id: &str) -> i64 {
    for i in (0..tool_index).rev() {
        if region[i].role != Role::ASSISTANT {
            continue;
        }
        if region[i].tool_calls.iter().any(|tc| tc.id == id) {
            return i as i64;
        }
    }
    -1
}

/// Splits a kept assistant turn whose results are not all kept (Go:
/// `splitKeptTurn`): the retained copy issues only the calls whose results
/// stay, and the folded copy carries the rest. Neither side issues a call
/// the other side answers.
fn split_kept_turn(region: &[Message], keep: &[bool], i: usize) -> (Message, Option<Message>) {
    let mut m = region[i].clone();
    if m.tool_calls.is_empty() || !splittable(&m) {
        return (m, None);
    }
    let mut answered = HashSet::new();
    let mut j = i + 1;
    while j < region.len() && region[j].role == Role::TOOL {
        if keep[j] {
            answered.insert(region[j].tool_call_id.clone());
        }
        j += 1;
    }
    let mut stay = Vec::new();
    let mut fold = Vec::new();
    for tc in &m.tool_calls {
        if answered.contains(&tc.id) {
            stay.push(tc.clone());
        } else {
            fold.push(tc.clone());
        }
    }
    if fold.is_empty() {
        return (m, None);
    }
    m.tool_calls = stay;
    let folded = Message {
        role: Role::ASSISTANT,
        created_at: m.created_at,
        tool_calls: fold,
        ..Message::default()
    };
    (m, Some(folded))
}

/// What a fold may not archive (Go: `keepIndexes`): the request that began
/// the running turn and the tool-call groups it is still executing.
pub(crate) fn keep_indexes(window: &Window, region: &[Message]) -> Vec<bool> {
    let mut keep = vec![false; region.len()];
    let active_turn = window.active_turn_created_at();
    for (i, m) in region.iter().enumerate() {
        if active_turn != 0 && m.role == Role::USER && m.created_at == active_turn {
            keep[i] = true;
        }
    }
    let pinned = keep.clone();
    for (i, m) in region.iter().enumerate() {
        if !pinned[i] {
            continue;
        }
        if m.role == Role::TOOL {
            let j = find_tool_caller(region, i, &m.tool_call_id);
            if j >= 0 {
                keep_answered_call(region, &mut keep, j);
            }
        } else if m.role == Role::ASSISTANT {
            keep_tool_call_group(region, &mut keep, i);
        }
    }
    keep
}

fn carries_standing_state(m: &Message) -> bool {
    m.role == Role::USER && !m.local_only && !is_compaction_summary(m)
}

/// Walks the host-injected block chain that opens content and drops the
/// ones `drop` reports (Go: `filterLeadingBlocks`). A block the host did
/// not inject is the user's own prose, and a one-time block ahead of a
/// standing one must not hide it.
fn filter_leading_blocks(content: &str, drop: &mut impl FnMut(&str, &str) -> bool) -> String {
    let mut rest = content;
    let mut kept = String::new();
    while let Some((tag, opening, end)) =
        reasonix_state::sessionstore::preview::leading_transient_block(rest)
    {
        let end = end.min(rest.len());
        let (head, tail) = rest.split_at(end);
        if !drop(&tag, &opening) {
            kept.push_str(head);
        }
        rest = tail;
    }
    let out = format!("{kept}{rest}");
    if out.trim().is_empty() {
        // A turn that was nothing but standing state still has to stay a
        // turn: an empty user message is a worse projection than a
        // redundant one.
        return content.to_owned();
    }
    out.trim_start_matches([' ', '\t', '\r', '\n']).to_owned()
}

/// Drops each standing-state block a later turn — kept or in the verbatim
/// tail — repeats (Go: `supersedeStandingState`). Only a newer block with
/// the same tag AND the same attributes supersedes: a short reminder must
/// never displace the fuller statement it points at. Newest-first, so the
/// newest of each variant survives.
fn supersede_standing_state(kept: Vec<Message>, tail: &[Message]) -> Vec<Message> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut drop = |tag: &str, opening: &str| -> bool {
        let superseded = reasonix_state::sessionstore::preview::SUPERSEDED_USER_BLOCK_TAGS
            .as_slice()
            .contains(&tag);
        if !superseded || seen.contains(opening) {
            return superseded;
        }
        seen.insert(opening.to_owned());
        false
    };
    for m in tail {
        if carries_standing_state(m) {
            // The tail's blocks only mark the variant seen; the live turn
            // stays whole.
            let _ = filter_leading_blocks(&m.content, &mut drop);
        }
    }
    let mut kept = kept;
    for i in (0..kept.len()).rev() {
        if !carries_standing_state(&kept[i]) {
            continue;
        }
        kept[i].content = filter_leading_blocks(&kept[i].content, &mut drop);
    }
    kept
}

/// Splits the region into what stays and what is archived (Go:
/// `partitionFold`).
pub(crate) fn partition_fold(
    window: &Window,
    region: &[Message],
    tail: &[Message],
) -> (Vec<Message>, Vec<Message>) {
    let keep = keep_indexes(window, region);
    let mut kept = Vec::new();
    let mut fold = Vec::new();
    for (i, m) in region.iter().enumerate() {
        if m.local_only {
            // Display-only output never reaches a provider.
        } else if is_compaction_summary(m) {
            fold.push(m.clone());
        } else if keep[i] && m.role == Role::ASSISTANT {
            let (stay, rest) = split_kept_turn(region, &keep, i);
            kept.push(stay);
            if let Some(rest) = rest {
                fold.push(rest);
            }
        } else if keep[i] {
            kept.push(m.clone());
        } else {
            fold.push(m.clone());
        }
    }
    (supersede_standing_state(kept, tail), fold)
}
/// The planned region's bounds (Go: the `head`/`start` pair planFoldRegion
/// returns and its callers pass on).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FoldSpan {
    pub head: usize,
    pub start: usize,
}

/// One fold's landing point once expressed canonically (Go: `foldBoundary`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FoldBoundary {
    /// The canonical prefix the new projection claims. It never moves
    /// backwards: history an archive already folded stays folded.
    pub covered: i64,
    /// Where the frozen body remainder the new projection must carry
    /// forward starts, as `body[body_suffix_from..]`. Equals the body
    /// length when nothing survives.
    pub body_suffix_from: usize,
}

/// Translates a working-view boundary into canonical coordinates (Go:
/// `mapFoldBoundary`). `projected` decides: false means the view is the
/// transcript and the body arguments are ignored, so a stale pair cannot
/// contradict it. At or inside the body, coverage stays at `prior_covered`
/// — those indices name no canonical message — and the remainder the
/// archive left behind stays frozen.
pub(crate) fn map_fold_boundary(
    start: usize,
    body_len: usize,
    prior_covered: i64,
    projected: bool,
) -> FoldBoundary {
    if !projected {
        return FoldBoundary {
            covered: start as i64,
            body_suffix_from: 0,
        };
    }
    let prior_covered = prior_covered.max(0);
    if (start as i64) < body_len as i64 {
        FoldBoundary {
            covered: prior_covered,
            body_suffix_from: start,
        }
    } else {
        FoldBoundary {
            covered: prior_covered + (start - body_len) as i64,
            body_suffix_from: body_len,
        }
    }
}

/// Builds the body a fold freezes: stable head, digest, kept history, and
/// the remainder of an older body the new digest did not consume (Go:
/// `checkpointProjectionMessages`). The recent tail is not here — it is
/// canonical[covered..], spliced live, and freezing a copy of it is what
/// made a fold claim the whole transcript.
pub(crate) fn checkpoint_projection_messages(
    msgs: &[Message],
    head: usize,
    kept: &[Message],
    body_suffix: &[Message],
    summary: &str,
) -> Vec<Message> {
    let mut proj = Vec::with_capacity(head + 1 + kept.len() + body_suffix.len());
    proj.extend_from_slice(&msgs[..head]);
    proj.push(format_summary_message(summary));
    proj.extend_from_slice(kept);
    proj.extend_from_slice(body_suffix);
    projection_messages(&proj).into_owned()
}

/// The arguments of one summary-projection commit (Go:
/// `summaryProjectionCommit`). `covered` is the canonical prefix this fold
/// claims, decided by the caller that planned the fold: persistence
/// records it and never re-derives one.
pub(crate) struct SummaryProjectionCommit<'a> {
    pub canonical: &'a [Message],
    pub covered: i64,
    pub projected: Vec<Message>,
    pub transcript_version: u64,
    pub projection_version: u64,
    pub generation: u64,
    pub trigger: String,
    pub summary: String,
    pub input_hash: String,
    pub output_hash: String,
    pub source_tokens: i64,
    pub projection_tokens: i64,
}

/// Builds the next compaction state from a commit (Go:
/// `summaryProjectionState`). LastReceipt is authoritative; the mirrored
/// `last_*`/`blocked_*` writer fields are stripped again on save.
fn summary_projection_state(
    window: &Window,
    commit: &SummaryProjectionCommit<'_>,
) -> (CompactionState, ContextMaintenanceReceipt) {
    let projection_version = commit.projection_version + 1;
    let now = reasonix_state::sessionstore::eventlog::CreatedAt::now();
    let summary_hash = summary_content_hash(&commit.summary);
    let covered_hash = covered_prefix_hash(commit.canonical, commit.covered);
    let receipt = ContextMaintenanceReceipt {
        operation_id: format!("summary-{projection_version}-{}", commit.output_hash),
        status: "applied".to_owned(),
        action: "summary".to_owned(),
        trigger: commit.trigger.clone(),
        source_projection: commit.projection_version,
        projection_version,
        covered_count: commit.covered,
        covered_prefix_hash: covered_hash.clone(),
        input_hash: commit.input_hash.clone(),
        output_hash: commit.output_hash.clone(),
        input_tokens: commit.source_tokens,
        result_tokens: commit.projection_tokens,
        saved_tokens: (commit.source_tokens - commit.projection_tokens).max(0),
        summary_hash: summary_hash.clone(),
        cache_break: true,
        created_at: now,
        ..ContextMaintenanceReceipt::default()
    };
    let state = CompactionState {
        schema_version: COMPACTION_STATE_SCHEMA_CURRENT,
        transcript_version: commit.transcript_version,
        generation: commit.generation + 1,
        prompt_cache_key: window.prompt_cache_key(),
        projection: ContextProjection {
            messages: commit.projected.clone(),
            transcript_version: commit.transcript_version,
            projection_version,
            covered_count: commit.covered,
            covered_prefix_hash: covered_hash,
            summary_hash,
            source_tokens: commit.source_tokens,
            projection_tokens: commit.projection_tokens,
            view_input_hash: commit.input_hash.clone(),
            view_output_hash: commit.output_hash.clone(),
            created_at: now,
        },
        last_receipt: Some(receipt.clone()),
        updated_at: now,
        ..CompactionState::default()
    };
    (state, receipt)
}

impl Window {
    /// CAS-installs one checkpoint under the compaction guard (Go:
    /// `commitSummaryProjection`): transcript version/hash, projection
    /// version, and generation must still match. The maintenance event is
    /// emitted only after the lock is released so a sink that re-enters
    /// the snapshot cannot deadlock.
    pub(crate) fn commit_summary_projection(
        &self,
        commit: SummaryProjectionCommit<'_>,
    ) -> Result<ContextMaintenanceReceipt, FoldError> {
        if commit.covered <= 0 || commit.covered > commit.canonical.len() as i64 {
            return Err(FoldError::Failed(format!(
                "compaction commit: covered {} outside canonical length {}",
                commit.covered,
                commit.canonical.len()
            )));
        }
        let (state, receipt) = summary_projection_state(self, &commit);
        let key = state.prompt_cache_key.clone();
        let result = self.with_core(|core| {
            let (current, current_version) = self.session().snapshot_messages_version();
            if current_version != commit.transcript_version
                || current.len() != commit.canonical.len()
                || covered_prefix_hash(&current, current.len() as i64)
                    != covered_prefix_hash(commit.canonical, commit.canonical.len() as i64)
                || core.compaction_state.projection.projection_version != commit.projection_version
                || core.compaction_state.generation != commit.generation
            {
                return Err(FoldError::StaleContext);
            }
            let previous = core.compaction_state.clone();
            self.install_projection_core(
                core,
                &key,
                commit.transcript_version,
                state.projection.clone(),
            );
            core.compaction_state.last_receipt = state.last_receipt.clone();
            core.compaction_state.updated_at = state.updated_at;
            // Go replaces the whole state here, which resets the recall
            // ledger the recall wave owns; the reset travels with it.
            core.compaction_state.recall = Default::default();
            if self
                .persist_compaction_state(&core.compaction_state)
                .is_err()
            {
                core.compaction_state = previous;
                return Err(FoldError::PersistFailed);
            }
            Ok(receipt)
        });
        if let Ok(receipt) = &result {
            self.emit_context_maintenance(receipt);
        }
        result
    }

    /// The view a fold operates on, and whether it came from the installed
    /// projection (Go: `visibleInputForFold`). The second answer tells a
    /// fold reaching new history from one re-folding what a checkpoint
    /// already holds.
    pub(crate) fn visible_input_for_fold(
        &self,
        state: &CompactionState,
        canonical: &[Message],
    ) -> (Vec<Message>, bool) {
        let key = self.prompt_cache_key();
        if projection_valid(state, canonical, &key, None) {
            let projected = model_visible_from_projection(&state.projection, canonical);
            if !projected.is_empty() {
                return (projected, true);
            }
        }
        (canonical.to_vec(), false)
    }

    /// Returns the body this fold freezes and the canonical boundary it
    /// claims (Go: `foldedProjection`). A boundary inside an older body
    /// carries that body's remainder forward: those messages have no
    /// canonical counterpart to splice them back from.
    pub(crate) fn folded_projection(
        &self,
        state: &CompactionState,
        projected: bool,
        msgs: &[Message],
        kept: &[Message],
        span: FoldSpan,
        summary: &str,
    ) -> (Vec<Message>, FoldBoundary) {
        let body_len = state.projection.messages.len();
        let boundary = map_fold_boundary(
            span.start,
            body_len,
            state.projection.covered_count,
            projected,
        );
        let suffix: &[Message] = if projected && boundary.body_suffix_from < body_len {
            &state.projection.messages[boundary.body_suffix_from..]
        } else {
            &[]
        };
        (
            checkpoint_projection_messages(msgs, span.head, kept, suffix, summary),
            boundary,
        )
    }

    /// Puts the card up for a fold of `n` messages out of `msgs` and
    /// returns the size it announced (Go: `announceCompaction`), which the
    /// done frame reports against.
    pub(crate) fn announce_compaction(&self, trigger: &str, n: usize, msgs: &[Message]) -> i64 {
        let source_tokens = estimate::estimated_prompt_tokens(self, msgs);
        self.sink().emit(Event {
            kind: crate::sink::kind::COMPACTION_STARTED.to_owned(),
            compaction: Some(self.compaction_frame(Compaction {
                trigger: trigger.to_owned(),
                messages: n as i64,
                source_tokens,
                ..Compaction::default()
            })),
            ..Event::default()
        });
        source_tokens
    }

    /// Closes the card of a fold that installed nothing (Go:
    /// `emitCompactionAborted`). `code` is the class the host settled on.
    pub(crate) fn emit_compaction_aborted(&self, trigger: &str, code: CompactionNoopReason) {
        self.sink().emit(Event {
            kind: crate::sink::kind::COMPACTION_DONE.to_owned(),
            compaction: Some(self.compaction_frame(Compaction {
                trigger: trigger.to_owned(),
                code: code.to_owned(),
                ..Compaction::default()
            })),
            ..Event::default()
        });
    }

    /// Stamps the boundary that sent a fold onto one frame (Go:
    /// `compactionFrame`). Every emitter carries it.
    pub(crate) fn compaction_frame(&self, mut c: Compaction) -> Compaction {
        c.boundary = self.compact_boundary().to_owned();
        c.trigger_tokens = self.compact_trigger();
        c
    }

    /// Runs one archive fold (Go: `foldArchived`): the write is the
    /// summary transaction; its failure is the fold's failure.
    pub(crate) fn fold_archived(
        &self,
        fold: &[Message],
        instructions: &str,
    ) -> Result<FoldSummary, FoldError> {
        fold_to_archive(self, fold, instructions)
    }

    /// Installs one content-driven summary checkpoint (Go:
    /// `compactToProjection`): stable prefix + one digest + recent verbatim
    /// tail. The canonical transcript is never rewritten. `must_free` marks
    /// the fold the caller cannot proceed without; a noop there is the
    /// caller's hard failure.
    pub(crate) fn compact_to_projection(
        &self,
        request: &FoldRequest,
    ) -> Result<FoldResult, FoldError> {
        let trigger = request.trigger.clone();
        let scope = request.scope;
        let instructions = request.instructions.clone();
        let verdict = |reason: CompactionNoopReason| {
            Ok(FoldResult {
                installed: false,
                noop_reason: Some(reason),
            })
        };

        let (canonical, transcript_version) = self.session().snapshot_messages_version();
        let (state_snapshot, start_projection_version, start_generation) = self.with_core(|core| {
            (
                core.compaction_state.clone(),
                core.compaction_state.projection.projection_version,
                core.compaction_state.generation,
            )
        });
        let (msgs, from_projection) = self.visible_input_for_fold(&state_snapshot, &canonical);
        let view_input_hash = provider_visible_fingerprint(&model_messages(&msgs));
        if !scope.ignore_economics
            && state_snapshot.last_receipt.as_ref().is_some_and(|r| {
                r.status == "applied" && r.action == "summary" && r.input_hash == view_input_hash
            })
        {
            return verdict(noop::INPUT_UNCHANGED);
        }
        let (head, start, ok, plan_reason) = plan_fold_region(self, &msgs);
        if !ok {
            return verdict(plan_reason);
        }
        // A checkpoint already holds everything up to its own length;
        // folding only inside that buys a second digest of one digest.
        if from_projection && !scope.ignore_economics {
            let held = state_snapshot.projection.messages.len();
            if start <= held {
                return verdict(noop::NO_NEW_CLOSED_PREFIX);
            }
            // Every checkpoint costs the whole prefix cache, so a second
            // one waits for a tail's worth of new closed history.
            if !scope.ignore_amount
                && estimate::estimated_prompt_tokens(self, &msgs[held..start])
                    < self.refold_min_tokens()
            {
                return verdict(noop::FOLD_BELOW_ECONOMICS);
            }
        }
        // The annotation rides the projection, not the canonical
        // transcript: the original stays whole for resume and rewind.
        let region = msgs[head..start].to_vec();
        let (kept, fold) = partition_fold(self, &region, &msgs[start..]);
        if fold.is_empty()
            || (!scope.ignore_economics && !scope.ignore_amount && !self.fold_economics(&fold))
        {
            return verdict(noop::FOLD_BELOW_ECONOMICS);
        }
        fold_cannot_fit(self, &trigger, &msgs, head, start)?;

        let source_tokens = self.announce_compaction(&trigger, fold.len(), &msgs);
        // The extension PreCompact/interceptCompactionPrepare interceptors
        // run here in Go; the extension wave owns that machine and
        // FoldError::HookRefused stays producer-less until it lands.

        let res = self
            .fold_archived(&fold, &instructions)
            .inspect_err(|err| {
                self.emit_compaction_aborted(&trigger, failure_code(err).1);
            })?;
        let summary = res.text;
        // interceptCompactionComplete: extension machine, named deferred.

        let (proj_msgs, boundary) = self.folded_projection(
            &state_snapshot,
            from_projection,
            &msgs,
            &kept,
            FoldSpan { head, start },
            &summary,
        );
        let candidate = model_visible_from_projection(
            &ContextProjection {
                messages: proj_msgs.clone(),
                covered_count: boundary.covered,
                ..ContextProjection::default()
            },
            &canonical,
        );
        let proj_tokens = estimate::estimated_prompt_tokens(self, &candidate);
        let fixed_prefix_tokens = estimate::estimated_prompt_tokens(self, &msgs[..head]);
        if let Err(err) = accept_checkpoint_candidate(
            self,
            &trigger,
            &scope,
            source_tokens,
            proj_tokens,
            fixed_prefix_tokens,
        ) {
            let err = attribute(
                err,
                candidate_holds(self, proj_tokens, fixed_prefix_tokens, &kept, &summary),
            );
            self.emit_compaction_aborted(&trigger, failure_code(&err).1);
            return Err(err);
        }
        let view_output_hash = provider_visible_fingerprint(&model_messages(&candidate));
        self.commit_summary_projection(SummaryProjectionCommit {
            canonical: &canonical,
            covered: boundary.covered,
            projected: proj_msgs,
            transcript_version,
            projection_version: start_projection_version,
            generation: start_generation,
            trigger: trigger.clone(),
            summary: summary.clone(),
            input_hash: view_input_hash,
            output_hash: view_output_hash,
            source_tokens,
            projection_tokens: proj_tokens,
        })
        .inspect_err(|err| {
            self.emit_compaction_aborted(&trigger, failure_code(err).1);
        })?;
        self.sink().emit(Event {
            kind: crate::sink::kind::COMPACTION_DONE.to_owned(),
            compaction: Some(self.compaction_frame(Compaction {
                trigger,
                messages: fold.len() as i64,
                summary,
                source_tokens,
                projection_tokens: proj_tokens,
                ..Compaction::default()
            })),
            ..Event::default()
        });
        // Only once the checkpoint is committed: a rejected candidate
        // folded nothing.
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}

/// The fold machine (Go: `compactToProjection`'s owner): the archive fold
/// wired to its window. The reference is weak because the window holds the
/// executor strongly — a strong cycle would outlive both.
pub struct ArchiveFold {
    window: Weak<Window>,
}

impl ArchiveFold {
    pub fn new(window: Weak<Window>) -> Self {
        ArchiveFold { window }
    }

    /// Builds and installs the archive fold for `window` (Go: the `New`
    /// wiring of the compaction family).
    pub fn install(window: &Arc<Window>) -> Arc<dyn FoldExecutor> {
        let executor: Arc<dyn FoldExecutor> = Arc::new(ArchiveFold::new(Arc::downgrade(window)));
        window.set_fold_executor(executor.clone());
        executor
    }
}

impl FoldExecutor for ArchiveFold {
    fn fold(&self, request: &FoldRequest) -> Result<FoldResult, FoldError> {
        let window = self
            .window
            .upgrade()
            .ok_or_else(|| FoldError::Failed("window dropped".to_owned()))?;
        let _run = hold_fold_run(&window);
        window.compact_to_projection(request)
    }
}

/// The one-fold-at-a-time lock (Go: `compactionRunMu`).
pub(crate) fn hold_fold_run(window: &Window) -> MutexGuard<'_, ()> {
    window.fold_run.lock().expect("fold run lock")
}

#[cfg(test)]
#[path = "fold_tests.rs"]
mod tests;
