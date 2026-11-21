use std::sync::Arc;

use reasonix_provider::{ChatCompletionRequest, ProviderError};
use reasonix_state::sessionstore::compaction_state::ContextProjection;

use super::*;
use crate::seams::fold_executor::{FoldError, FoldExecutor, FoldRequest, FoldResult};
use crate::window::testing::*;

fn overflow_rejection() -> ProviderError {
    ProviderError::Api {
        provider: "mock".to_owned(),
        status: 400,
        body: r#"{"error":{"message":"this model's maximum context length is 32768 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#.to_owned(),
        trace_id: String::new(),
        tool_context: String::new(),
    }
}

fn codeless_rejection() -> ProviderError {
    ProviderError::Api {
        provider: "mock".to_owned(),
        status: 400,
        body: r#"{"error":{"message":"maximum context length is 40000 tokens, you requested more","type":"invalid_request_error"}}"#.to_owned(),
        trace_id: String::new(),
        tool_context: String::new(),
    }
}

fn frozen_request(window: &Window) -> ChatCompletionRequest {
    window
        .prepare_request(Arc::new(crate::compose::DefaultComposer::default()))
        .expect("prepare")
}

fn request_chars(req: &ChatCompletionRequest) -> i64 {
    req.messages.iter().map(|m| m.content.len() as i64).sum()
}

// Go: foldableSessionOverForce + agentOverForceWindow — the session sits
// BELOW the pressure trigger (so the first prepare folds nothing) and the
// overflow trigger waives the threshold.
fn turn_session(window: &Window, turns: usize) {
    let big = "word ".repeat(400);
    let mut msgs = vec![
        system("sys"),
        user("standing constraint: never change the public API"),
    ];
    for _ in 0..turns {
        msgs.push(assistant(&big));
        msgs.push(user("continue"));
    }
    window.session().replace(msgs);
}

fn window_at(window_tokens: i64, ratio: f64) -> Arc<Window> {
    Arc::new(window_over(config(window_tokens, ratio), vec![]))
}

struct InstallingExecutor {
    window: Arc<Window>,
    digest_chars: usize,
}

impl FoldExecutor for InstallingExecutor {
    fn fold(&self, _request: &FoldRequest) -> Result<FoldResult, FoldError> {
        let canonical = self.window.session().snapshot();
        let covered = canonical.len() as i64;
        let digest = user(&format!(
            "<compaction-summary>\n{}\n</compaction-summary>",
            "d".repeat(self.digest_chars)
        ));
        self.window.install_projection(ContextProjection {
            messages: vec![digest],
            covered_count: covered,
            covered_prefix_hash: reasonix_state::sessionstore::compaction_state::covered_prefix_hash(&canonical, covered),
            projection_version: 3,
            ..ContextProjection::default()
        });
        Ok(FoldResult {
            installed: true,
            noop_reason: None,
        })
    }
}

// Go: TestContextOverflowRejectionFoldsAndReplays — the whole point: a
// provider that refuses the request as too long gets a smaller one, not a
// failed turn.
#[test]
fn an_overflow_rejection_folds_and_replays() {
    let window = window_at(40_000, 0.5);
    turn_session(&window, 20);
    window.set_fold_executor(Arc::new(InstallingExecutor {
        window: window.clone(),
        digest_chars: 2_000,
    }));
    let mut frozen = frozen_request(&window);
    let refused_chars = request_chars(&frozen);
    assert!(refused_chars > 20_000, "fixture must be a real request");

    let mut one_shot = false;
    let outcome = window.recover_context_overflow(&mut frozen, &mut one_shot, &overflow_rejection());
    assert!(matches!(outcome, OverflowRecovery::Recovered), "{outcome:?}");
    let replayed_chars = request_chars(&frozen);
    assert!(
        replayed_chars < refused_chars,
        "replay carried {replayed_chars} against the refused {refused_chars}; the fold freed nothing"
    );
}

// Go: TestContextOverflowProbesAnUndeclaredWindow — the refusal is the only
// size information there is, and it is enough to fold against.
#[test]
fn an_undeclared_window_is_probed_by_the_refusal() {
    let window = window_at(0, 0.0);
    turn_session(&window, 20);
    window.set_fold_executor(Arc::new(InstallingExecutor {
        window: window.clone(),
        digest_chars: 2_000,
    }));
    assert_eq!(
        window.effective_context_window(),
        0,
        "window before the refusal"
    );
    let mut frozen = frozen_request(&window);
    let refused_chars = request_chars(&frozen);
    let mut one_shot = false;
    let outcome = window.recover_context_overflow(&mut frozen, &mut one_shot, &overflow_rejection());
    assert!(matches!(outcome, OverflowRecovery::Recovered), "{outcome:?}");
    assert!(
        window.effective_context_window() > 0,
        "the refusal taught the agent nothing about the window"
    );
    assert!(
        request_chars(&frozen) < refused_chars,
        "the replay must carry less than the refused request"
    );
}

// Go: TestContextOverflowWithNothingToFoldFailsTheTurn — resending a
// request the provider already refused would spend a call to be told the
// same thing.
#[test]
fn nothing_to_fold_reports_a_failed_recovery() {
    let window = window_at(40_000, 0.5);
    turn_session(&window, 20);
    let mut frozen = frozen_request(&window);
    let refused = request_chars(&frozen);
    let mut one_shot = false;
    let outcome = window.recover_context_overflow(&mut frozen, &mut one_shot, &overflow_rejection());
    match outcome {
        OverflowRecovery::Failed { reason } => {
            assert!(!reason.is_empty());
        }
        other => panic!("want a failed recovery, got {other:?}"),
    }
    assert_eq!(request_chars(&frozen), refused, "the frozen request stands");
}

// Go: TestContextOverflowRecoveryIsOneShotPerRound — a fold that did not
// reach far enough must not become a replay loop.
#[test]
fn the_overflow_recovery_is_one_shot_per_round() {
    let window = window_at(40_000, 0.5);
    turn_session(&window, 20);
    window.set_fold_executor(Arc::new(InstallingExecutor {
        window: window.clone(),
        digest_chars: 8_000,
    }));
    let mut frozen = frozen_request(&window);
    let mut one_shot = false;
    let first = window.recover_context_overflow(&mut frozen, &mut one_shot, &overflow_rejection());
    assert!(matches!(first, OverflowRecovery::Recovered), "{first:?}");
    // A second rejection on the same round is answered by the latch, not by
    // another fold.
    let second = window.recover_context_overflow(&mut frozen, &mut one_shot, &overflow_rejection());
    assert!(
        matches!(second, OverflowRecovery::NotApplicable),
        "{second:?}"
    );
}

// Go: TestCodelessRejectionPastTheWindowWithItsCompletionFoldsAndReplays —
// a rejection that names no code is told apart by measurement: the prompt
// plus the completion it asked for no longer fits the window.
#[test]
fn a_codeless_rejection_past_the_window_with_its_completion_folds() {
    let window = window_at(12_000, 0.99);
    turn_session(&window, 19);
    window.set_fold_executor(Arc::new(InstallingExecutor {
        window: window.clone(),
        digest_chars: 1_000,
    }));
    let mut frozen = frozen_request(&window);
    let est = estimate::estimated_request_tokens(&window, &frozen);
    assert!(
        est < 12_000 && est + 4_000 > 12_000,
        "fixture prompt {est} must sit between window-completion and window"
    );
    let mut one_shot = false;
    let outcome = window.recover_context_overflow(&mut frozen, &mut one_shot, &codeless_rejection());
    assert!(matches!(outcome, OverflowRecovery::Recovered), "{outcome:?}");
}

// Go: TestCodelessRejectionOnASharedWindowFoldsAndReplays — the refusal is
// judged against the configured budget, not the clipped one.
#[test]
fn a_codeless_rejection_on_a_shared_window_judges_the_configured_budget() {
    let mut cfg = config(20_000, 0.99);
    cfg.max_output_tokens = 4_000;
    cfg.shared_window = crate::window::SharedWindowPolicy {
        shares_context_window: true,
        output_budget: 4_000,
        ..crate::window::SharedWindowPolicy::default()
    };
    let window = Arc::new(window_over(cfg, vec![]));
    turn_session(&window, 19);
    window.set_fold_executor(Arc::new(InstallingExecutor {
        window: window.clone(),
        digest_chars: 1_000,
    }));
    let mut frozen = frozen_request(&window);
    let est = estimate::estimated_request_tokens(&window, &frozen);
    assert!(
        est + 4_000 < 20_000
            && est + 4_000 + estimate::output_budget_reserve_for(20_000) > 20_000,
        "fixture prompt {est} must fit the window with its completion but not with the margin too"
    );
    let mut one_shot = false;
    let outcome = window.recover_context_overflow(&mut frozen, &mut one_shot, &codeless_rejection());
    assert!(matches!(outcome, OverflowRecovery::Recovered), "{outcome:?}");
}

// Go: context_overflow.go — a retryable status is a busy server whatever
// code rides along, and prose about limits is not a code.
#[test]
fn only_documented_codes_classify_as_overflow() {
    assert!(is_context_overflow(&overflow_rejection()));
    assert!(!is_context_overflow(&codeless_rejection()));
    let retryable = ProviderError::Api {
        provider: "mock".to_owned(),
        status: 429,
        body: r#"{"error":{"code":"context_length_exceeded"}}"#.to_owned(),
        trace_id: String::new(),
        tool_context: String::new(),
    };
    assert!(!is_context_overflow(&retryable));
    let prose = ProviderError::Api {
        provider: "mock".to_owned(),
        status: 400,
        body: r#"{"error":{"message":"the context window exceeded somewhere in the docs"}}"#.to_owned(),
        trace_id: String::new(),
        tool_context: String::new(),
    };
    assert!(!is_context_overflow(&prose));
    let interrupted = ProviderError::StreamInterrupted {
        reason: "premature_eof".to_owned(),
        message: String::new(),
    };
    assert!(!is_context_overflow(&interrupted));
}
