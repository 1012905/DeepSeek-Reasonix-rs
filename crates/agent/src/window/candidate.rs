//! The checkpoint candidate's verdict (Go: `candidate_holds.go`,
//! `checkpoint_rejected.go`, and `acceptCheckpointCandidate` /
//! `foldCannotFit` from `candidate_holds.go`).
//!
//! A candidate the host refuses names which rule refused it: four rules
//! share one sentinel on the Go side, and a reader that cannot tell them
//! apart reports all as the first. The code is the identity; the sentence
//! is a diagnostic.

use reasonix_contract::provider::{Message, Role};

use crate::seams::fold_executor::FoldError;
use crate::window::maintenance::{CompactionNoopReason, noop};

use super::Window;
use super::estimate;
use super::fold::partition_fold;
use super::trigger_label;

/// Splits a rejected candidate's size by what it is made of (Go:
/// `candidateHolds`), so a fold that cannot get under the line says what
/// keeps it there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CandidateHolds {
    prefix: i64,
    kept: i64,
    summary: i64,
    tail: i64,
}

impl std::fmt::Display for CandidateHolds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fixed prefix {} + protected messages {} + summary {} + recent tail {} tokens",
            self.prefix, self.kept, self.summary, self.tail
        )
    }
}

pub(crate) fn candidate_holds(
    window: &Window,
    total: i64,
    prefix: i64,
    kept: &[Message],
    summary: &str,
) -> CandidateHolds {
    let mut h = CandidateHolds {
        prefix,
        kept: estimate::estimated_prompt_tokens(window, kept),
        summary: estimate::estimated_prompt_tokens(
            window,
            &[Message {
                role: Role::USER,
                content: summary.to_owned(),
                ..Message::default()
            }],
        ),
        tail: 0,
    };
    h.tail = (total - h.prefix - h.kept - h.summary).max(0);
    h
}

/// Names what made a size-based rejection (Go: `attribute`); any other
/// refusal is left as it was.
pub(crate) fn attribute(err: FoldError, h: CandidateHolds) -> FoldError {
    match err {
        FoldError::CheckpointRejected { code, reason }
            if code == noop::CANDIDATE_ABOVE_TRIGGER
                || code == noop::CANDIDATE_ABOVE_CEILING
                || code == noop::CANDIDATE_ABOVE_PHYSICAL =>
        {
            FoldError::CheckpointRejected {
                code,
                reason: format!("{reason} ({h})"),
            }
        }
        other => other,
    }
}

/// Go: `rejectCheckpoint` — the verdict with the rule that refused it and
/// the figures behind that rule.
pub(crate) fn reject_checkpoint(code: CompactionNoopReason, reason: String) -> FoldError {
    FoldError::CheckpointRejected { code, reason }
}

/// Acceptance rules for one checkpoint candidate (Go:
/// `acceptCheckpointCandidate`): 50% + smaller for auto; waiving economics
/// may exceed 50% only if still below trigger; manual below trigger
/// accepts any savings, since below the trigger the ceiling has nothing to
/// protect.
pub(crate) fn accept_checkpoint_candidate(
    window: &Window,
    trigger: &str,
    scope: &crate::seams::fold_executor::FoldScope,
    source_tokens: i64,
    candidate_tokens: i64,
    fixed_prefix_tokens: i64,
) -> Result<(), FoldError> {
    if candidate_tokens >= source_tokens {
        return Err(reject_checkpoint(
            noop::CANDIDATE_NOT_SMALLER,
            format!("candidate would not reduce tokens ({candidate_tokens} >= {source_tokens})"),
        ));
    }
    let trigger_tokens = window.compact_trigger();
    let ceiling = window.checkpoint_ceiling();
    let hard = window.hard_input_ceiling();
    if trigger == trigger_label::MANUAL {
        return Ok(());
    }
    if fixed_prefix_tokens > ceiling {
        // Exceptional path: fixed prefix alone already exceeds 50%.
        let savings = source_tokens - candidate_tokens;
        if savings < window.exceptional_minimum_savings() {
            return Err(reject_checkpoint(
                noop::SAVINGS_BELOW_MINIMUM,
                format!(
                    "fixed-prefix exception requires >={} token savings, got {}",
                    window.exceptional_minimum_savings(),
                    savings
                ),
            ));
        }
        if trigger_tokens > 0 && candidate_tokens >= trigger_tokens {
            return Err(reject_checkpoint(
                noop::CANDIDATE_ABOVE_TRIGGER,
                format!("candidate {candidate_tokens} still at or above trigger {trigger_tokens}"),
            ));
        }
        if hard > 0 && candidate_tokens >= hard {
            return Err(reject_checkpoint(
                noop::CANDIDATE_ABOVE_PHYSICAL,
                format!("candidate {candidate_tokens} still at or above physical ceiling {hard}"),
            ));
        }
        return Ok(());
    }
    if ceiling > 0 && candidate_tokens > ceiling && !scope.ignore_economics {
        // Protected content made the candidate large; this is not a
        // fixed-prefix exception. Force/overflow may still land a strictly
        // smaller view below the trigger when the ceiling cannot.
        return Err(reject_checkpoint(
            noop::CANDIDATE_ABOVE_CEILING,
            format!(
                "candidate {candidate_tokens} exceeds checkpoint ceiling {ceiling} (protected content too large)"
            ),
        ));
    }
    // Not an exception anyone may waive: a checkpoint that lands back at
    // the trigger has bought the next fold rather than avoided it.
    if trigger_tokens > 0 && candidate_tokens >= trigger_tokens {
        return Err(reject_checkpoint(
            noop::CANDIDATE_ABOVE_TRIGGER,
            format!("candidate {candidate_tokens} still at or above trigger {trigger_tokens}"),
        ));
    }
    Ok(())
}

/// Refuses, before any summary is paid for, a fold whose candidate cannot
/// get under the trigger whatever the archive says (Go: `foldCannotFit`):
/// the fixed prefix, the messages the keep policy protects and the recent
/// tail already reach it.
pub(crate) fn fold_cannot_fit(
    window: &Window,
    trigger: &str,
    msgs: &[Message],
    head: usize,
    start: usize,
) -> Result<(), FoldError> {
    let prefix = estimate::estimated_prompt_tokens(window, &msgs[..head]);
    let limit = window.compact_trigger();
    if window.effective_context_window() <= 0 || limit <= 0 {
        return Ok(());
    }
    if prefix >= limit {
        return Err(reject_checkpoint(
            noop::FIXED_PREFIX_ABOVE_TRIGGER,
            format!("fixed prefix ({prefix} tokens) already exceeds trigger ({limit})"),
        ));
    }
    if trigger == trigger_label::MANUAL && estimate::estimated_prompt_tokens(window, msgs) < limit {
        return Ok(());
    }
    let (kept, _) = partition_fold(window, &msgs[head..start], &msgs[start..]);
    let held = CandidateHolds {
        prefix,
        kept: estimate::estimated_prompt_tokens(window, &kept),
        summary: 0,
        tail: estimate::estimated_prompt_tokens(window, &msgs[start..]),
    };
    if held.prefix + held.kept + held.tail < limit {
        return Ok(());
    }
    Err(attribute(
        reject_checkpoint(
            noop::CANDIDATE_ABOVE_TRIGGER,
            format!("candidate cannot get under trigger {limit} even with an empty summary"),
        ),
        held,
    ))
}
