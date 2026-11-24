//! ACP-016: usage accumulator turn/cumulative rules, source resolution, and the
//! finishTurn usage_update emit gates.

use reasonix_acp::update::{UsageCost, build_usage_update};
use reasonix_acp::usage::{UsageAccumulator, UsageTotals, resolve_source};

#[test]
fn turn_increment_resets_while_cumulative_accumulates() {
    let mut acc = UsageAccumulator::new();
    acc.add_event(&UsageTotals {
        prompt_tokens: 10,
        completion_tokens: 5,
        reasoning_tokens: 2,
        cache_hit_tokens: 0,
        cache_miss_tokens: 3,
        estimated: false,
        usage_source: "planner".to_owned(),
    });
    acc.add_event(&UsageTotals {
        prompt_tokens: 4,
        completion_tokens: 1,
        reasoning_tokens: 0,
        cache_hit_tokens: 1,
        cache_miss_tokens: 0,
        estimated: true,
        usage_source: "planner".to_owned(),
    });
    assert_eq!(acc.turn.prompt_tokens, 14);
    assert_eq!(acc.cumulative.prompt_tokens, 14);
    assert!(acc.cumulative.estimated);

    acc.reset_turn();
    assert_eq!(acc.turn.prompt_tokens, 0);
    assert_eq!(acc.cumulative.prompt_tokens, 14);
}

#[test]
fn source_resolution_falls_through_and_marks_mixed() {
    assert_eq!(resolve_source("", ""), "executor");
    assert_eq!(resolve_source("planner", ""), "planner");
    assert_eq!(resolve_source("", "executor"), "executor");
    assert_eq!(resolve_source("executor", "executor"), "executor");
    assert_eq!(resolve_source("planner", "executor"), "mixed");
}

#[test]
fn usage_update_gates_size_used_floor_and_currency() {
    assert!(build_usage_update(-5, 0, None).is_none());

    let plain = build_usage_update(1200, 8000, None).unwrap();
    let reasonix_acp::update::SessionUpdate::Usage { used, size, cost } = plain else {
        panic!("expected usage_update");
    };
    assert_eq!(used, 1200);
    assert_eq!(size, 8000);
    assert!(cost.is_none());

    let floored = build_usage_update(-9, 100, None).unwrap();
    let reasonix_acp::update::SessionUpdate::Usage { used, .. } = floored else {
        panic!()
    };
    assert_eq!(used, 0);

    let no_currency = build_usage_update(
        1,
        100,
        Some(UsageCost {
            amount: 0.03,
            currency: String::new(),
        }),
    )
    .unwrap();
    let reasonix_acp::update::SessionUpdate::Usage { cost, .. } = no_currency else {
        panic!()
    };
    assert!(cost.is_none());

    let priced = build_usage_update(
        1,
        100,
        Some(UsageCost {
            amount: 0.03,
            currency: "USD".to_owned(),
        }),
    )
    .unwrap();
    let reasonix_acp::update::SessionUpdate::Usage { cost, .. } = priced else {
        panic!()
    };
    assert_eq!(cost.unwrap().currency, "USD");
}
