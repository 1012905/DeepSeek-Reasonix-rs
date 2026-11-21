//! The fold oracles (Go: compact_test.go, compact_manual_*_test.go,
//! compact_threshold_test.go, compaction_budgets_test.go,
//! compact_failure_code_test.go, compact_plan_test.go's region half,
//! compact_archive_test.go, compact_raw_output_test.go,
//! projection_valid_test.go's install half, compact_group.go).

use std::sync::Arc;

use reasonix_contract::provider::{Message, Role, ToolCall};
use reasonix_state::sessionstore::Session;
use reasonix_state::sessionstore::compaction_state::load_compaction_state;

use crate::seams::fold_executor::{FoldError, FoldRequest, FoldResult, FoldScope};
use crate::sink::Discard;
use crate::window::WindowConfig;
use crate::window::fold::{
    closed_prefix_end, map_fold_boundary, partition_fold, pinned_prefix_len, plan_fold_region,
};
use crate::window::fold_archive::{
    archive_pointer, fold_to_archive, format_summary_message, prompt_cache_key, render_transcript,
    render_transcript_verbatim, summarize_tool_args,
};
use crate::window::maintenance::{ContextPreparePolicy, noop};
use crate::window::trigger_label;
use crate::window::{Window, is_compaction_summary, projection_valid};

fn msg(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_owned(),
        ..Message::default()
    }
}

/// A unique scratch root per test (Go: testenv.TempDir).
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "reasonix-fold-test-{}-{}-{}",
        name,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// A window bound to a temp archive dir, so folds write inside the test's
/// own tree (Go: Options.ArchiveDir + testenv.TempDir).
fn archive_window(
    context_window: i64,
    compact_ratio: f64,
    name: &str,
) -> (Arc<Window>, std::path::PathBuf) {
    let dir = scratch(name);
    let config = WindowConfig {
        context_window,
        compact_ratio,
        workspace_id: "ws".to_owned(),
        model_ref: "p/m".to_owned(),
        archive_dir: dir.to_string_lossy().into_owned(),
        ..WindowConfig::default()
    };
    let window = Arc::new(Window::new(
        config,
        Arc::new(Session::new("sys")),
        Arc::new(reasonix_tools::Registry::new()),
        Arc::new(Discard),
    ));
    (window, dir)
}

fn manual_request(instructions: &str, ignore_economics: bool) -> FoldRequest {
    FoldRequest {
        trigger: trigger_label::MANUAL.to_owned(),
        instructions: instructions.to_owned(),
        scope: FoldScope {
            ignore_threshold: true,
            ignore_economics,
            ignore_amount: true,
        },
        must_free: false,
    }
}

/// Go: foldableSessionOverForce — bulk assistant text Prepare must fold.
fn foldable_transcript(turns: usize) -> Vec<Message> {
    let big = "word ".repeat(400);
    let mut msgs = vec![
        msg(Role::SYSTEM, "sys"),
        msg(
            Role::USER,
            "standing constraint: never change the public API",
        ),
    ];
    for i in 0..turns {
        msgs.push(msg(Role::ASSISTANT, &big));
        msgs.push(msg(Role::USER, &format!("continue {i}")));
    }
    msgs
}

fn fold_once(window: &Arc<Window>, request: &FoldRequest) -> Result<FoldResult, FoldError> {
    window.compact_to_projection(request)
}

fn archived_folds(window: &Window) -> usize {
    let out = crate::window::fold_archive::outputs_dir(window);
    std::fs::read_dir(&out)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("fold-"))
                .count()
        })
        .unwrap_or(0)
}

fn failure_code_of(err: &FoldError) -> &'static str {
    crate::window::maintenance::failure_code(err).1
}

// Go: TestPinnedPrefixLen — the system prompt and nothing else is pinned;
// summaries and a large first turn stay foldable.
#[test]
fn pins_only_the_system_prompt() {
    let sys = msg(Role::SYSTEM, "");
    let small = msg(Role::USER, "do X with token T");
    let big = msg(Role::USER, &"x".repeat(100_000));
    let sum = msg(
        Role::USER,
        "<compaction-summary>\ndigest\n</compaction-summary>",
    );
    let asst = msg(Role::ASSISTANT, "a");
    let cases: [(&str, Vec<Message>, usize); 5] = [
        (
            "pins-only-the-system-prompt",
            vec![sys.clone(), small.clone(), asst.clone(), asst.clone()],
            1,
        ),
        (
            "summaries-are-not-pinned",
            vec![
                sys.clone(),
                small.clone(),
                sum.clone(),
                sum.clone(),
                asst.clone(),
            ],
            1,
        ),
        (
            "large-first-turn-stays-foldable",
            vec![sys.clone(), big, asst.clone(), asst.clone()],
            1,
        ),
        (
            "no-system-prompt-pins-nothing",
            vec![small.clone(), asst.clone()],
            0,
        ),
        ("tiny-window-pins-the-same", vec![sys, small, asst], 1),
    ];
    for (name, msgs, want) in cases {
        assert_eq!(pinned_prefix_len(&msgs), want, "{name}");
    }
}

// Go: TestRenderTranscriptRedactsToolCallArgs — a digest cannot reproduce a
// long argument.
#[test]
fn render_transcript_redacts_tool_call_args() {
    let msgs = vec![
        msg(Role::USER, "Find me popular GitHub MCP projects"),
        Message {
            role: Role::ASSISTANT,
            content: "I'll research that.".to_owned(),
            tool_calls: vec![ToolCall {
                id: "1".to_owned(),
                name: "research".to_owned(),
                arguments:
                    r#"{"task":"Search for recently popular GitHub projects that let AI use/control any software through MCP..."}"#
                        .to_owned(),
                ..ToolCall::default()
            }],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            name: "research".to_owned(),
            content: "Found 5 projects.".to_owned(),
            ..Message::default()
        },
    ];
    let out = render_transcript(&msgs);
    assert!(!out.contains("Search for recently popular"), "{out}");
    assert!(out.contains("[assistant calls research]"), "{out}");
    assert!(out.contains("task"), "{out}");
}

// Go: TestSummarizeToolArgs.
#[test]
fn summarize_tool_args_table() {
    let cases = [
        (
            "redacts long task prompt",
            r#"{"task":"Search for recently popular"}"#,
            "task",
        ),
        ("empty args", "", "no arguments"),
        ("invalid json", "not json", "bytes"),
        (
            "multiple keys sorted",
            r#"{"prompt":"do something","model":"gpt-4"}"#,
            "model, prompt",
        ),
    ];
    for (name, args, want) in cases {
        assert!(summarize_tool_args(args).contains(want), "{name}");
    }
}

// Go: TestRecallStillAnswersFromTheFullOutput — the verbatim render keeps
// the full output the bounded model copy dropped.
#[test]
fn verbatim_render_keeps_the_full_output() {
    const SENTINEL: &str = "RAW-ONLY-SENTINEL";
    let msgs = vec![
        Message {
            role: Role::ASSISTANT,
            tool_calls: vec![ToolCall {
                id: "1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"big.log"}"#.to_owned(),
                ..ToolCall::default()
            }],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            tool_call_id: "1".to_owned(),
            name: "read_file".to_owned(),
            content: "bounded head\n[output truncated]\n".to_owned(),
            raw_content: format!("bounded head\n{SENTINEL} filler line\n"),
            ..Message::default()
        },
    ];
    assert!(
        render_transcript_verbatim(&msgs).contains(SENTINEL),
        "the verbatim render lost the full output it exists to return"
    );
}

// Go: TestAcceptCheckpointCandidateRules + TestAManualFoldAboveTheTrigger...
// + TestAnAutomaticFoldAboveTheTrigger... + TestCheckpointRejectionNamesTheRule
// — a candidate the host refuses names which rule refused it.
#[test]
fn accept_checkpoint_candidate_rules() {
    let (window, _dir) = archive_window(1_000_000, 0.85, "accept");
    let scope = FoldScope::default();
    let pressure = trigger_label::PRESSURE;
    let manual = trigger_label::MANUAL;

    // 20% candidate under the normal path: accept.
    super::accept_checkpoint_candidate(&window, pressure, &scope, 850_000, 180_000, 50_000)
        .expect("20% candidate");
    // 51% candidate: reject.
    let err =
        super::accept_checkpoint_candidate(&window, pressure, &scope, 850_000, 510_000, 50_000)
            .expect_err("51% candidate");
    assert_eq!(failure_code_of(&err), noop::CANDIDATE_ABOVE_CEILING);
    // Fixed prefix > 50% with enough savings: accept.
    super::accept_checkpoint_candidate(&window, pressure, &scope, 900_000, 600_000, 520_000)
        .expect("fixed-prefix exception");
    // Fixed prefix > 50% without 25% savings: reject.
    let err =
        super::accept_checkpoint_candidate(&window, pressure, &scope, 600_000, 550_000, 520_000)
            .expect_err("fixed-prefix without savings");
    assert_eq!(failure_code_of(&err), noop::SAVINGS_BELOW_MINIMUM);
    // Manual below trigger: accept any reduction.
    super::accept_checkpoint_candidate(&window, manual, &scope, 100_000, 80_000, 10_000)
        .expect("manual below trigger");

    let trigger = window.compact_trigger();
    // A manual fold above the trigger that frees anything is accepted.
    super::accept_checkpoint_candidate(
        &window,
        manual,
        &scope,
        trigger + 20_000,
        trigger + 5_000,
        1_000,
    )
    .expect("manual fold above the trigger");
    // A manual fold that frees nothing is still refused.
    let err = super::accept_checkpoint_candidate(
        &window,
        manual,
        &scope,
        trigger + 20_000,
        trigger + 20_000,
        1_000,
    )
    .expect_err("manual fold that frees nothing");
    assert_eq!(failure_code_of(&err), noop::CANDIDATE_NOT_SMALLER);
    // An automatic fold that stays above the trigger buys the next fold.
    super::accept_checkpoint_candidate(
        &window,
        pressure,
        &scope,
        trigger + 20_000,
        trigger + 5_000,
        1_000,
    )
    .expect_err("automatic fold above the trigger");

    // The rules keep their identities (Go's failureFixture: window 5000,
    // ratio 0.5, where the ceiling and the trigger coincide).
    let (tight, _dir) = archive_window(5000, 0.5, "accept-tight");
    let tight_trigger = tight.compact_trigger();
    let err = super::accept_checkpoint_candidate(&tight, pressure, &scope, 1000, 1000, 10)
        .expect_err("not smaller");
    assert_eq!(failure_code_of(&err), noop::CANDIDATE_NOT_SMALLER);
    let err = super::accept_checkpoint_candidate(
        &tight,
        pressure,
        &scope,
        tight_trigger * 4,
        tight_trigger,
        10,
    )
    .expect_err("still at trigger");
    let _ = &err;
    assert_eq!(failure_code_of(&err), noop::CANDIDATE_ABOVE_TRIGGER);
}

// Go: TestMaybeCompactFoldsSingleLargeMessageAtThreshold — one large
// assistant turn folds at the observed pressure, and the canonical
// transcript is untouched.
#[test]
fn a_single_large_message_folds_at_threshold() {
    let (window, _dir) = archive_window(10_000, 0.8, "single");
    let big = "large prompt chunk ".repeat(500);
    window.session().replace(vec![
        msg(Role::SYSTEM, "sys"),
        msg(Role::USER, "task"),
        msg(Role::ASSISTANT, &big),
        msg(Role::USER, "next"),
        msg(Role::ASSISTANT, "ok"),
    ]);
    crate::window::fold::ArchiveFold::install(&window);
    let canonical_len = window.session().len();
    let outcome = window
        .prepare(ContextPreparePolicy {
            observed_input_tokens: 8_500,
            ..ContextPreparePolicy::default()
        })
        .expect("prepare");
    assert!(
        outcome.maintenance.installed,
        "no fold ran: {:?}",
        outcome.maintenance
    );
    assert_eq!(window.session().len(), canonical_len);
    let visible = window.model_visible_messages();
    assert!(
        visible.iter().any(is_compaction_summary),
        "the fold did not install a digest"
    );
}

// Go: TestLargeFixedPrefixDoesNotRefuseCompaction — the fixed prefix is
// compared in tokens, not characters.
#[test]
fn large_fixed_prefix_does_not_refuse_compaction() {
    let prefix = "standing project instruction. ".repeat(4_000);
    let (window, _dir) = archive_window(128_000, 0.85, "prefix");
    let mut msgs = vec![msg(Role::SYSTEM, &prefix)];
    let big = "word ".repeat(400);
    for i in 0..40 {
        msgs.push(msg(Role::ASSISTANT, &big));
        msgs.push(msg(Role::USER, &format!("continue {i}")));
    }
    window.session().replace(msgs);
    assert!(
        prefix.len() as i64 > window.compact_trigger(),
        "fixture prefix cannot show the bug"
    );
    let request = FoldRequest {
        trigger: trigger_label::MANUAL.to_owned(),
        instructions: String::new(),
        scope: FoldScope {
            ignore_threshold: true,
            ignore_economics: true,
            ignore_amount: true,
        },
        must_free: false,
    };
    assert!(
        fold_once(&window, &request).expect("fold").installed,
        "compaction refused a prefix that fits"
    );
}

// Go: TestManualCompactRunsBelowTheAutomaticTrigger — asking waives the
// automatic threshold.
#[test]
fn manual_compact_runs_below_the_automatic_trigger() {
    let (window, _dir) = archive_window(200_000, 0.8, "manual-below");
    window.session().replace(foldable_transcript(60));
    let est =
        crate::window::estimated_visible_request_tokens(&window, &window.model_visible_messages());
    assert!(
        est < window.compact_trigger(),
        "fixture is not below the trigger: {est} >= {}",
        window.compact_trigger()
    );
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("fold")
            .installed
    );
    assert_eq!(archived_folds(&window), 1);
}

// Go: TestManualCompactOnAnUnchangedContextBuysNothing — requesting waives
// the threshold, never the new-history rule.
#[test]
fn manual_compact_on_an_unchanged_context_buys_nothing() {
    let (window, _dir) = archive_window(200_000, 0.8, "manual-unchanged");
    window.session().replace(foldable_transcript(60));
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("first fold")
            .installed
    );
    let first = archived_folds(&window);

    let result = fold_once(&window, &manual_request("", false)).expect("second fold");
    assert!(!result.installed, "the second request folded again");
    assert!(
        result.noop_reason == Some(noop::NO_NEW_CLOSED_PREFIX)
            || result.noop_reason == Some(noop::INPUT_UNCHANGED),
        "verdict = {:?}, want a reason a frontend can show",
        result.noop_reason
    );
    assert_eq!(archived_folds(&window), first);
}

// Go: TestManualCompactFoldsAfterALittleNewHistory.
#[test]
fn manual_compact_folds_after_a_little_new_history() {
    let (window, _dir) = archive_window(200_000, 0.8, "manual-little");
    window.session().replace(foldable_transcript(60));
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("first fold")
            .installed
    );
    let first = archived_folds(&window);

    for i in 0..12 {
        window
            .session()
            .add(msg(Role::ASSISTANT, &"word ".repeat(120)));
        window
            .session()
            .add(msg(Role::USER, &format!("continue {i}")));
    }
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("second fold")
            .installed
    );
    assert_eq!(archived_folds(&window) - first, 1);
}

// Go: TestForcedCompactWaivesEconomicsOnly — the escape hatch waives
// economics and nothing else: a candidate that is not smaller is refused.
#[test]
fn forced_compact_waives_economics_only() {
    let (window, _dir) = archive_window(200_000, 0.8, "forced");
    window.session().replace(foldable_transcript(60));
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("first fold")
            .installed
    );
    let first = archived_folds(&window);

    let result = fold_once(&window, &manual_request("", true));
    assert!(
        archived_folds(&window) > first,
        "a forced request over the same context made no fold"
    );
    if let Ok(result) = result {
        assert!(result.installed, "forced fold reported no install");
        let receipt = window
            .context_maintenance_snapshot()
            .last_receipt
            .expect("receipt");
        assert!(
            receipt.result_tokens < receipt.input_tokens,
            "forced fold installed a projection that is not smaller: {receipt:?}"
        );
    }
}

// Go: TestCompactInstallsCoveredPrefixHash — a fresh projection validates
// against its own transcript and carries the lineage key.
#[test]
fn compact_installs_covered_prefix_hash() {
    let (window, dir) = archive_window(2_000, 0.9, "install-hash");
    window.session().replace(foldable_transcript(8));
    let path = dir.join("s.jsonl");
    window.set_session_path(&path.to_string_lossy());
    assert!(
        fold_once(&window, &manual_request("", false))
            .expect("fold")
            .installed
    );

    let st = window.with_core(|core| core.compaction_state.clone());
    assert!(
        !st.projection.covered_prefix_hash.is_empty(),
        "hash not set"
    );
    assert_eq!(st.prompt_cache_key, prompt_cache_key("ws", "", "p/m"));
    let msgs = window.session().snapshot();
    assert!(
        projection_valid(&st, &msgs, &st.prompt_cache_key, None),
        "fresh projection should validate"
    );
    let on_disk = load_compaction_state(&path)
        .expect("load")
        .expect("sidecar");
    assert_eq!(on_disk.projection.projection_version, 1);
    assert_eq!(on_disk.last_receipt.expect("receipt").status, "applied");
}

// Go: TestManualCompactWithNothingToFoldIsAVerdictNotAnOverflow — a
// hand-typed compact on a short transcript is answered, never claimed as a
// provider-limit failure.
#[test]
fn manual_compact_with_nothing_to_fold_is_a_verdict() {
    let shapes: [(&str, Vec<Message>); 3] = [
        (
            "one exchange",
            vec![
                msg(Role::SYSTEM, "sys"),
                msg(Role::USER, "hello"),
                msg(Role::ASSISTANT, "hi"),
            ],
        ),
        (
            "nothing answered yet",
            vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "hello")],
        ),
        ("system only", vec![msg(Role::SYSTEM, "sys")]),
    ];
    for (name, msgs) in shapes {
        let (window, _dir) = archive_window(200_000, 0.8, "verdict");
        window.session().replace(msgs);
        match fold_once(&window, &manual_request("", false)) {
            Err(err) => {
                assert!(
                    matches!(
                        err,
                        FoldError::CheckpointRejected { .. } | FoldError::Failed(_)
                    ),
                    "{name}: {err}, want a declined verdict"
                );
                assert!(
                    !err.to_string().contains("context exceeds provider limit"),
                    "{name}: {err} claims the provider limit"
                );
            }
            Ok(result) => {
                if result.installed {
                    continue; // folding a short transcript may succeed
                }
                assert!(
                    result.noop_reason.is_some(),
                    "{name}: a declined request carries no reason"
                );
            }
        }
    }
}

// The active turn is not a transaction in flight: the fold ends where every
// call the turn issued has its result.
#[test]
fn plan_fold_region_trims_to_the_active_turns_closed_prefix() {
    let (window, _dir) = archive_window(20_000, 0.5, "plan-active");
    let call = |id: &str| ToolCall {
        id: id.to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
        ..ToolCall::default()
    };
    let mut active = msg(Role::USER, "current turn");
    active.created_at = 42;
    let msgs = vec![
        msg(Role::SYSTEM, "sys"),
        msg(Role::USER, "earlier"),
        msg(Role::ASSISTANT, "earlier answer"),
        active,
        Message {
            role: Role::ASSISTANT,
            tool_calls: vec![call("a"), call("b")],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            tool_call_id: "a".to_owned(),
            name: "bash".to_owned(),
            content: "done".to_owned(),
            ..Message::default()
        },
    ];
    window.note_active_turn_created_at(42);
    let (head, start, ok, reason) = plan_fold_region(&window, &msgs);
    assert!(ok, "{reason:?}");
    assert_eq!(head, 1);
    // Call b is unanswered, so the fold stops right after the turn's own
    // request; the partition keeps that request.
    assert_eq!(start, 4, "active turn boundary trimmed wrong ({reason:?})");

    // Nothing closed inside the turn and nothing precedes it: the whole
    // visible context is one transaction in flight.
    let only = vec![msg(Role::SYSTEM, "sys"), msgs[3].clone(), msgs[4].clone()];
    let (_, _, ok, reason) = plan_fold_region(&window, &only);
    assert!(!ok);
    assert_eq!(reason, noop::ACTIVE_TURN_BOUNDARY);
}

// The request that began the running turn stays; everything else in the
// region archives (Go: keepIndexes + partitionFold).
#[test]
fn partition_keeps_the_active_turns_request() {
    let (window, _dir) = archive_window(20_000, 0.5, "partition");
    let call = |id: &str| ToolCall {
        id: id.to_owned(),
        name: "edit".to_owned(),
        arguments: "{}".to_owned(),
        ..ToolCall::default()
    };
    let mut pinned = msg(Role::USER, "keep me");
    pinned.created_at = 7;
    let region = vec![
        msg(Role::USER, "old question"),
        msg(Role::ASSISTANT, "old answer"),
        pinned,
        Message {
            role: Role::ASSISTANT,
            tool_calls: vec![call("k"), call("f")],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            tool_call_id: "k".to_owned(),
            name: "edit".to_owned(),
            content: "kept result".to_owned(),
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            tool_call_id: "f".to_owned(),
            name: "edit".to_owned(),
            content: "folded result".to_owned(),
            ..Message::default()
        },
    ];
    window.note_active_turn_created_at(7);
    let (kept, fold) = partition_fold(&window, &region, &[]);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].content, "keep me");
    // The whole caller archives with both calls: nothing of it is kept.
    let folded_caller = fold
        .iter()
        .find(|m| !m.tool_calls.is_empty())
        .expect("folded caller");
    assert_eq!(folded_caller.tool_calls.len(), 2);
    assert_eq!(fold.len(), 5, "{fold:?}");
}

// Go: compact_group.go — a kept caller whose results are not all kept
// splits: the retained copy issues only the calls whose results stay, and
// the folded copy carries the rest. Neither side issues a call the other
// side answers.
#[test]
fn kept_turn_splits_its_calls_by_answered_side() {
    let call = |id: &str| ToolCall {
        id: id.to_owned(),
        name: "edit".to_owned(),
        arguments: "{}".to_owned(),
        ..ToolCall::default()
    };
    let tool = |id: &str| Message {
        role: Role::TOOL,
        tool_call_id: id.to_owned(),
        name: "edit".to_owned(),
        content: "result".to_owned(),
        ..Message::default()
    };
    let caller = Message {
        role: Role::ASSISTANT,
        tool_calls: vec![call("k"), call("f")],
        ..Message::default()
    };
    let region = vec![caller.clone(), tool("k"), tool("f")];
    let mut keep = vec![false; 3];
    super::keep_tool_call_group(&region, &mut keep, 0);
    assert_eq!(keep, vec![true, true, true], "the whole group stays");

    // Only result k is kept: a splittable caller retains just its call.
    let mut keep = vec![false; 3];
    keep[0] = true;
    keep[1] = true;
    let (stay, folded) = super::split_kept_turn(&region, &keep, 0);
    assert_eq!(stay.tool_calls.len(), 1);
    assert_eq!(stay.tool_calls[0].id, "k");
    let folded = folded.expect("folded copy");
    assert_eq!(folded.tool_calls.len(), 1);
    assert_eq!(folded.tool_calls[0].id, "f");

    // An unsplittable caller (repeated ids) keeps its whole group instead.
    let mut unsplittable = caller.clone();
    unsplittable.tool_calls[1].id = "k".to_owned();
    let region2 = vec![unsplittable, tool("k"), tool("k")];
    let mut keep2 = vec![true, false, true];
    super::keep_answered_call(&region2, &mut keep2, 0);
    assert_eq!(keep2, vec![true, true, true], "the group stays whole");

    // A caller whose folded set is empty returns no folded copy.
    let keep3 = vec![true, true, true];
    let (_, none) = super::split_kept_turn(&region, &keep3, 0);
    assert!(none.is_none());
}

// A tool result a kept turn answers pulls its caller in; a splittable
// caller keeps only the call (Go: keepAnsweredCall).
#[test]
fn a_kept_result_pulls_its_splittable_caller() {
    let call = |id: &str| ToolCall {
        id: id.to_owned(),
        name: "edit".to_owned(),
        arguments: "{}".to_owned(),
        ..ToolCall::default()
    };
    let tool = |id: &str| Message {
        role: Role::TOOL,
        tool_call_id: id.to_owned(),
        name: "edit".to_owned(),
        content: "result".to_owned(),
        ..Message::default()
    };
    let caller = Message {
        role: Role::ASSISTANT,
        tool_calls: vec![call("k"), call("f")],
        ..Message::default()
    };
    let region = vec![caller, tool("k"), tool("f")];
    let mut keep = vec![false; 3];
    keep[1] = true;
    super::keep_answered_call(&region, &mut keep, 0);
    assert_eq!(keep, vec![true, true, false]);
}

// The digest a prior fold installed is archive input, never kept history;
// display-only output never reaches either side.
#[test]
fn partition_folds_the_previous_digest_and_drops_local_only() {
    let (window, _dir) = archive_window(20_000, 0.5, "partition-digest");
    let mut local = msg(Role::ASSISTANT, "display only");
    local.local_only = true;
    let region = vec![
        format_summary_message("older digest"),
        msg(Role::USER, "next question"),
        msg(Role::ASSISTANT, "next answer"),
        local,
    ];
    let (kept, fold) = partition_fold(&window, &region, &[]);
    assert!(kept.iter().all(|m| !m.local_only), "local output kept");
    assert!(kept.is_empty(), "nothing is pinned here: {kept:?}");
    assert_eq!(fold.len(), 3, "{fold:?}");
    assert!(is_compaction_summary(&fold[0]));
    assert!(!fold.iter().any(|m| m.local_only), "local output archived");
}

// A standing-state block a newer turn restates is dropped from the kept
// history; a one-time block survives (Go: supersedeStandingState).
#[test]
fn partition_supersedes_standing_state_blocks() {
    let (window, _dir) = archive_window(20_000, 0.5, "supersede");
    let mut old_goal = msg(
        Role::USER,
        "<active-goal id=\"1\">ship v1</active-goal>\nplease continue",
    );
    old_goal.created_at = 7;
    let new_goal = msg(
        Role::USER,
        "<active-goal id=\"1\">ship v2</active-goal>\nkeep going",
    );
    let region = vec![old_goal];
    window.note_active_turn_created_at(7);
    // The live turn restates the goal: the older copy of the block is
    // redundant in the kept history, the user's own words are not.
    let (kept, _fold) = partition_fold(&window, &region, &[new_goal]);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(
        !kept[0].content.contains("<active-goal"),
        "the older goal block survived: {:?}",
        kept[0].content
    );
    assert!(kept[0].content.contains("please continue"));

    // A one-time block in the tail never supersedes anything.
    let mut old_goal = msg(
        Role::USER,
        "<active-goal id=\"1\">ship v1</active-goal>\nplease continue",
    );
    old_goal.created_at = 7;
    let one_time = msg(Role::USER, "<memory-update>wrote a file</memory-update>");
    let (kept, _fold) = partition_fold(&window, &[old_goal], &[one_time]);
    assert!(
        kept[0].content.contains("<active-goal"),
        "a one-time block displaced a standing one: {:?}",
        kept[0].content
    );
}

// Go: fold_boundary.go — a working-view boundary expressed canonically.
#[test]
fn map_fold_boundary_cases() {
    // Not projected: the view is the transcript.
    assert_eq!(
        map_fold_boundary(4, 10, 3, false),
        super::FoldBoundary {
            covered: 4,
            body_suffix_from: 0
        }
    );
    // At or inside the body: coverage stays at the prior boundary and the
    // remainder stays frozen.
    assert_eq!(
        map_fold_boundary(5, 10, 3, true),
        super::FoldBoundary {
            covered: 3,
            body_suffix_from: 5
        }
    );
    // Past the body: coverage moves by the tail consumed.
    assert_eq!(
        map_fold_boundary(12, 10, 3, true),
        super::FoldBoundary {
            covered: 5,
            body_suffix_from: 10
        }
    );
}

// Go: compact_archive.go — the archive file is named by its content, so
// folding the same span twice finds the same file.
#[test]
fn fold_to_archive_names_the_file_by_its_content() {
    let (window, dir) = archive_window(20_000, 0.5, "archive-name");
    let fold = vec![msg(Role::USER, "one"), msg(Role::ASSISTANT, "two")];
    let first = fold_to_archive(&window, &fold, "").expect("write");
    let second = fold_to_archive(&window, &fold, "").expect("write again");
    assert_eq!(first.mode, crate::window::fold_archive::mode::ARCHIVED);
    assert_eq!(first.spans, 1);
    let out = crate::window::fold_archive::outputs_dir(&window);
    let files: Vec<_> = std::fs::read_dir(&out)
        .expect("outputs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "same span wrote twice: {files:?}");
    assert!(files[0].starts_with("fold-"));
    assert!(first.text.contains(&dir.to_string_lossy().to_string()));
    assert_eq!(first.text, second.text);
}

// Go: archivePointer — the pointer says what it is and how to read it, and
// carries the user's focus through.
#[test]
fn archive_pointer_carries_the_focus() {
    let text = archive_pointer("/tmp/fold-abc.md", 3, "keep the API stable");
    assert!(text.contains("3 earlier message(s)"), "{text}");
    assert!(text.contains("archived, not summarized"), "{text}");
    assert!(text.contains("Archive: /tmp/fold-abc.md"));
    assert!(text.contains("What the user asked you to keep in focus: keep the API stable"));
    let bare = archive_pointer("/tmp/fold-abc.md", 3, "  ");
    assert!(!bare.contains("keep in focus"), "{bare}");
}

// The receipt's summary hash fingerprints the digest; the projection
// wrapper stays a user turn the model can strip.
#[test]
fn summary_hashes_and_the_digest_wrapper_keep_their_shape() {
    assert_eq!(crate::window::fold_archive::summary_content_hash(""), "");
    let digest = format_summary_message("digest");
    assert!(digest.content.starts_with("<compaction-summary>\n"));
    assert!(digest.content.ends_with("</compaction-summary>"));
    assert_eq!(digest.role, Role::USER);
    let _ = closed_prefix_end(&[]);
}
