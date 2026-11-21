use super::*;

// Go: TestContextWindowProbeBoundsWhatWasRejectedAndAccepted.
#[test]
fn the_probe_bounds_what_was_rejected_and_accepted() {
    let probe = ContextWindowProbe::default();
    assert_eq!(probe.window(), 0, "window before any rejection");
    probe.note_accepted(40_000);
    assert_eq!(probe.window(), 0, "an accepted request alone says nothing");
    probe.note_rejected(100_000);
    probe.note_rejected(80_000);
    probe.note_rejected(120_000);
    assert_eq!(probe.window(), 40_000, "the largest accepted size wins");
    probe.note_accepted(90_000); // above the smallest refusal
    assert_eq!(
        probe.window(),
        80_000,
        "the smallest refused size stands in"
    );
}

// Go: TestContextWindowProbeIgnoresARefusalTooSmallToFold.
#[test]
fn a_refusal_too_small_to_fold_is_ignored() {
    let probe = ContextWindowProbe::default();
    probe.note_rejected(MIN_FOLD_SPAN_TOKENS - 1);
    assert_eq!(probe.window(), 0, "a refusal no fold could answer");
    probe.note_rejected(0);
    probe.note_accepted(-5);
    assert_eq!(probe.window(), 0, "meaningless sizes are not a bound");
}

// Go: the compact.go threshold table — a ratio past the window retires
// maintenance, an unmeasured window leaves the boundary unset.
#[test]
fn trigger_boundaries_follow_the_window_and_ratio() {
    let window = crate::window::testing::window(0, 0.9);
    assert_eq!(window.compact_trigger(), 0);
    let window = crate::window::testing::window(100_000, 0.5);
    assert_eq!(window.compact_trigger(), 50_000);
    let window = crate::window::testing::window(100_000, 0.0);
    assert_eq!(
        window.compact_trigger(),
        ((100_000f64 * crate::window::trigger::DEFAULT_COMPACT_RATIO) as i64).max(1)
    );
    // compact_ratio 2 puts the boundary past the window, which is how
    // maintenance is turned off: the threshold is unreachable before the
    // hard ceiling, and the boundary names itself to no frontend.
    let window = crate::window::testing::window(100_000, 2.0);
    assert_eq!(window.compact_trigger(), 200_000);
    assert_eq!(window.compact_boundary(), "");
}

// Go: refoldMinTokens — clamped to the window's share and to half the
// trigger, because a tail reaching the trigger leaves nothing to fold.
#[test]
fn the_refold_floor_tracks_the_trigger() {
    let window = crate::window::testing::window(100_000, 0.5);
    // 10% = 10_000, lifted to the 32K floor, then halved by the
    // trigger's own half (25_000) — a tail reaching the trigger leaves
    // nothing to fold.
    assert_eq!(window.refold_min_tokens(), 25_000);
    let window = crate::window::testing::window(200_000, 0.9);
    // 10% = 20_000 -> 32K floor; trigger/2 = 90_000 -> 32_768 stands.
    assert_eq!(window.refold_min_tokens(), 32_768);
    let window = crate::window::testing::window(400_000, 0.5);
    // 10% = 40_000; trigger/2 = 100_000; max 96_000 -> 40_000 stands.
    assert_eq!(window.refold_min_tokens(), 40_000);
    let window = crate::window::testing::window(800_000, 0.9);
    // 10% = 80_000; trigger/2 = 360_000; max 96_000 -> 80_000 stands.
    assert_eq!(window.refold_min_tokens(), 80_000);
    let window = crate::window::testing::window(0, 0.0);
    assert_eq!(window.refold_min_tokens(), MIN_REFOLD_TOKENS);
}
