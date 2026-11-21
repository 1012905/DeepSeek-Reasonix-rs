use std::sync::Arc;

use reasonix_state::sessionstore::compaction_state::ContextProjection;

use super::*;
use crate::window::testing::*;

fn install_digest(window: &Window, digest_chars: usize, covered: i64) {
    let canonical = window.session().snapshot();
    let digest = user(&format!(
        "<compaction-summary>\n{}\n</compaction-summary>",
        "d".repeat(digest_chars)
    ));
    window.install_projection(ContextProjection {
        messages: vec![digest],
        covered_count: covered,
        covered_prefix_hash: reasonix_state::sessionstore::compaction_state::covered_prefix_hash(
            &canonical, covered,
        ),
        projection_version: 11,
        ..ContextProjection::default()
    });
}

// Go: context_status.go — present composition separates from cumulative
// summary-call cost, and the UI checkpoint label needs a still-valid
// covered prefix.
#[test]
fn the_snapshot_separates_projection_from_canonical() {
    let window = Arc::new(window_over(
        config(20_000, 0.5),
        vec![
            system("system"),
            user("task"),
            assistant(&"old work output. ".repeat(3_200)),
        ],
    ));
    let canonical_tokens = window.context_maintenance_snapshot().canonical_tokens;
    install_digest(&window, 2_000, 3);
    let snap = window.context_maintenance_snapshot();
    assert!(
        snap.projected_tokens < canonical_tokens,
        "projected {} vs canonical {canonical_tokens}",
        snap.projected_tokens
    );
    assert!(snap.summary_tokens > 0, "the digest is priced as summary");
    assert_eq!(snap.checkpoint_state, "applied");
    assert_eq!(snap.projection_version, 11);
    assert_eq!(snap.fold_trigger, window.compact_trigger());
    assert_eq!(snap.trigger_tokens, snap.fold_trigger);
    assert_eq!(
        snap.headroom,
        (snap.hard_input_ceiling - snap.projected_tokens).max(0)
    );
}

// Go: the receipt path of the snapshot — a held blocked receipt marks the
// generation blocked, and an applied one reports what it saved.
#[test]
fn the_snapshot_reports_a_held_generation() {
    let window = Arc::new(window_over(
        config(20_000, 0.5),
        vec![
            system("system"),
            user("task"),
            assistant(&"old work output. ".repeat(3_200)),
        ],
    ));
    window.record_context_maintenance_outcome(
        "",
        crate::window::trigger_label::PRESSURE,
        "summary",
        "blocked",
        crate::window::maintenance::noop::FIXED_PREFIX_ABOVE_TRIGGER,
        "too large",
    );
    let snap = window.context_maintenance_snapshot();
    assert!(snap.blocked, "a deterministic block must hold");
    let receipt = snap.last_receipt.expect("receipt");
    assert_eq!(receipt.status, "blocked");
    assert_eq!(receipt.code, "fixed_prefix_above_trigger");
}

// Go: the empty-conversation zero snapshot.
#[test]
fn an_empty_conversation_reads_a_zero_snapshot() {
    let window = window_over(config(20_000, 0.5), vec![]);
    let snap = window.context_maintenance_snapshot();
    assert_eq!(snap.canonical_tokens, 0);
    assert_eq!(snap.projected_tokens, 0);
    assert!(!snap.blocked);
    assert!(snap.last_receipt.is_none());
}
