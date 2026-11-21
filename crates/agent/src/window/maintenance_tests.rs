//! The Prepare/fold-decision oracles (Go: context_block_release_test.go,
//! context_held_notice_test.go, and the context_manager_test.go decision
//! cases that do not need extensions).

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use reasonix_state::sessionstore::compaction_state::ContextProjection;

use super::*;
use crate::seams::fold_executor::{FoldExecutor, FoldRequest, FoldResult};
use crate::window::testing::*;

// Go: newPressureAgent — a window of 20000 at ratio 0.5 folds at 10000
// tokens, so a transient failure releases after 1250 more tokens of
// input.
fn pressure_window() -> Arc<Window> {
    Arc::new(window_over(
        config(20_000, 0.5),
        vec![
            system("system"),
            user("task"),
            assistant(&"old work output. ".repeat(3_200)),
            user("current"),
            assistant("tail"),
        ],
    ))
}

struct FailingExecutor {
    calls: AtomicI64,
}

impl FoldExecutor for FailingExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(FoldError::Failed("summary unavailable".to_owned()))
    }
}

struct CountingExecutor {
    calls: AtomicI64,
}

impl FoldExecutor for CountingExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}

// Go: TestDeterministicFailureStaysBlockedDespiteGrowth.
#[test]
fn deterministic_failure_stays_blocked_despite_growth() {
    let window = pressure_window();
    let exec = Arc::new(FailingExecutor {
        calls: AtomicI64::new(0),
    });
    window.set_fold_executor(exec.clone());
    window.record_context_maintenance_outcome(
        "",
        trigger_label::PRESSURE,
        "summary",
        "blocked",
        noop::FIXED_PREFIX_ABOVE_TRIGGER,
        "too large",
    );
    window
        .session()
        .add(user(&"more input words. ".repeat(2_000)));
    let prepared = window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
    assert!(!prepared.messages.is_empty());
}

// Go: TestRepeatedOverflowOnUnchangedInputIsBounded. One pressure
// attempt, then the unclassified failure holds every overflow attempt.
#[test]
fn repeated_overflow_on_unchanged_input_is_bounded() {
    let window = pressure_window();
    let exec = Arc::new(FailingExecutor {
        calls: AtomicI64::new(0),
    });
    window.set_fold_executor(exec.clone());
    window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("pressure prepare");
    for _ in 0..20 {
        let err = window.prepare(ContextPreparePolicy {
            trigger: trigger_label::OVERFLOW.to_owned(),
            ..ContextPreparePolicy::default()
        });
        assert!(err.is_err(), "overflow on a held generation must refuse");
    }
    assert!(
        exec.calls.load(Ordering::SeqCst) <= 3,
        "summary requests = {}",
        exec.calls.load(Ordering::SeqCst)
    );
}

// Go: TestHoldBelowTheTriggerIsNotAnnounced — a hold with nothing due
// says nothing.
#[test]
fn a_hold_below_the_trigger_is_not_announced() {
    let sink = Arc::new(RecordingSink::default());
    let window = Window::new(
        config(20_000, 0.5),
        Arc::new(reasonix_state::sessionstore::Session::new("")),
        Arc::new(reasonix_tools::Registry::new()),
        sink.clone(),
    );
    window.record_context_maintenance_outcome(
        "",
        trigger_label::PRESSURE,
        "summary",
        "blocked",
        noop::FIXED_PREFIX_ABOVE_TRIGGER,
        "too large",
    );
    window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert!(
        sink.notices_with_code(crate::sink::notice_code::COMPACT_HELD)
            .is_empty(),
        "a hold with nothing due was announced"
    );
}

// A hold at the due threshold says so once per turn and cause (Go: the
// lastReported scoping of noteMaintenanceHeld).
#[test]
fn a_hold_at_the_threshold_is_announced_once_per_turn() {
    let sink = Arc::new(RecordingSink::default());
    let transcript = Arc::new(reasonix_state::sessionstore::Session::new(""));
    transcript.replace(vec![
        system("system"),
        user("task"),
        assistant(&"old work output. ".repeat(3_200)),
    ]);
    let window = Arc::new(Window::new(
        config(20_000, 0.5),
        transcript,
        Arc::new(reasonix_tools::Registry::new()),
        sink.clone(),
    ));
    let exec = Arc::new(FailingExecutor {
        calls: AtomicI64::new(0),
    });
    window.set_fold_executor(exec);
    window.record_context_maintenance_outcome(
        "",
        trigger_label::PRESSURE,
        "summary",
        "blocked",
        noop::FIXED_PREFIX_ABOVE_TRIGGER,
        "too large",
    );
    for _ in 0..2 {
        window
            .prepare(ContextPreparePolicy {
                trigger: trigger_label::PRESSURE.to_owned(),
                ..ContextPreparePolicy::default()
            })
            .expect("prepare");
    }
    let held = sink.notices_with_code(crate::sink::notice_code::COMPACT_HELD);
    assert_eq!(held.len(), 1, "the hold must speak once per turn and cause");
}

// A pressure Prepare above the trigger folds through the seam and
// returns the re-read view with the install verdict.
#[test]
fn prepare_above_the_trigger_folds_through_the_seam() {
    let window = pressure_window();
    let before = window.model_visible_messages().len();
    let exec: Arc<dyn FoldExecutor> = Arc::new(InstallingExecutor {
        window: window.clone(),
    });
    window.set_fold_executor(exec);
    let prepared = window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert!(prepared.maintenance.installed, "the fold must install");
    assert!(prepared.messages.len() < before);
}

// Go: ContextPreparePolicy.ObservedInputTokens pins the estimate so a
// test can trigger a fold at an exact size.
#[test]
fn observed_input_tokens_pin_the_fold_decision() {
    let window = pressure_window();
    let exec = Arc::new(CountingExecutor {
        calls: AtomicI64::new(0),
    });
    window.set_fold_executor(exec.clone());
    window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            observed_input_tokens: 15_000,
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert_eq!(exec.calls.load(Ordering::SeqCst), 1);
    // Below the pinned trigger the same transcript folds nothing.
    window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            observed_input_tokens: 5_000,
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert_eq!(exec.calls.load(Ordering::SeqCst), 1);
}

// A summary that stays above the trigger pauses maintenance for the
// turn (Go: the resultAboveTrigger arm) and later pressure rounds stop
// asking.
#[test]
fn a_result_above_the_trigger_marks_the_generation_stuck() {
    let window = pressure_window();
    let exec = Arc::new(StuckInstallingExecutor {
        window: window.clone(),
        calls: AtomicI64::new(0),
    });
    window.set_fold_executor(exec.clone());
    let prepared = window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert!(prepared.maintenance.installed);
    let calls_after_first = exec.calls.load(Ordering::SeqCst);
    let again = window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .expect("second prepare");
    assert!(!again.maintenance.installed);
    assert_eq!(
        exec.calls.load(Ordering::SeqCst),
        calls_after_first,
        "a stuck generation must not re-enter summary"
    );
}

// Go: the manual trigger waives the threshold (asking by hand waives
// only how much was added).
#[test]
fn a_manual_prepare_waives_the_threshold() {
    let window = Arc::new(window_over(
        config(20_000, 0.5),
        vec![user("tiny"), assistant("reply")],
    ));
    let exec: Arc<dyn FoldExecutor> = Arc::new(InstallingExecutor {
        window: window.clone(),
    });
    window.set_fold_executor(exec);
    let prepared = window
        .prepare(ContextPreparePolicy {
            trigger: trigger_label::MANUAL.to_owned(),
            ignore_economics: true,
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert!(prepared.maintenance.installed);
}

struct InstallingExecutor {
    window: Arc<Window>,
}

impl InstallingExecutor {
    fn digest_projection(&self) -> ContextProjection {
        let canonical = self.window.session().snapshot();
        let covered = canonical.len().saturating_sub(1) as i64;
        ContextProjection {
            messages: vec![user(
                "<compaction-summary>\nthe earlier work is summarized here\n</compaction-summary>",
            )],
            covered_count: covered,
            covered_prefix_hash:
                reasonix_state::sessionstore::compaction_state::covered_prefix_hash(
                    &canonical, covered,
                ),
            projection_version: 7,
            ..ContextProjection::default()
        }
    }
}

impl FoldExecutor for InstallingExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        self.window.install_projection(self.digest_projection());
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}

// Installs a projection whose digest alone stays above the fold trigger.
struct StuckInstallingExecutor {
    window: Arc<Window>,
    calls: AtomicI64,
}

impl FoldExecutor for StuckInstallingExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let canonical = self.window.session().snapshot();
        let covered = canonical.len() as i64;
        let digest = user(&format!(
            "<compaction-summary>\n{}\n</compaction-summary>",
            "d".repeat(48_000)
        ));
        self.window.install_projection(ContextProjection {
            messages: vec![digest],
            covered_count: covered,
            covered_prefix_hash:
                reasonix_state::sessionstore::compaction_state::covered_prefix_hash(
                    &canonical, covered,
                ),
            projection_version: 9,
            ..ContextProjection::default()
        });
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}
