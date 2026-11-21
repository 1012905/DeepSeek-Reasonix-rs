//! Usage accounting: per-round Usage events, the billable merge across
//! attempts, and cache aggregates (Go: run_usage.go).

mod support;

use reasonix_agent::sink::kind;
use reasonix_agent::usage::{finalize_sampling_usage, merge_sampling_usage};
use reasonix_provider::Usage;

use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_responses, text_frame, usage_frame,
};

#[tokio::test]
async fn a_clean_round_emits_one_usage_event_with_the_provider_numbers() {
    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("answer"),
        &usage_frame(11, 5, "stop"),
        "[DONE]",
    ])])
    .await;
    let Fixture {
        agent, recorder, ..
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("q").await.expect("turn");
    let usage = out.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 16);
    assert_eq!(usage.request_count, 1);
    assert!(!usage.estimated);
    assert_eq!(usage.finish_reason, "stop");

    let events = recorder.events_of(kind::USAGE);
    assert_eq!(events.len(), 1);
    let wire = events[0].usage.as_ref().expect("wire usage");
    assert_eq!(wire.prompt_tokens, 11);
    assert_eq!(wire.total_tokens, 16);
    assert_eq!(events[0].model_ref, "relay/m");
}

#[tokio::test]
async fn usage_carries_the_cache_split_the_provider_reported() {
    let frame = r#"{"choices":[{"delta":{"content":"ok"}}]}"#;
    let usage = r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":2,"total_tokens":102,"prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20}}"#;
    let base = serve_responses(vec![chunked_sse(&[frame, usage, "[DONE]"])]).await;
    let Fixture {
        agent, recorder, ..
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("q").await.expect("turn");
    let usage = out.usage.expect("usage");
    assert_eq!(usage.cache_hit_tokens, 80);
    assert_eq!(usage.cache_miss_tokens, 20);
    assert_eq!(usage.prompt_tokens, 100);

    let wire = recorder.events_of(kind::USAGE)[0]
        .usage
        .as_ref()
        .expect("wire")
        .clone();
    assert_eq!(wire.cache_hit_tokens, 80);
    assert_eq!(wire.cache_miss_tokens, 20);
}

#[tokio::test]
async fn a_replayed_round_bills_every_attempt_and_finish_reason_follows_the_last() {
    // Attempt 1 streams tokens then dies mid-body (premature close); the
    // provider reported no usage for it. Attempt 2 is clean and reports
    // usage. The billable record counts both requests.
    let close =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
    let base = serve_responses(vec![
        close,
        chunked_sse(&[&text_frame("final"), &usage_frame(9, 3, "stop"), "[DONE]"]),
    ])
    .await;
    let Fixture {
        agent, recorder, ..
    } = fixture(&base, registry_with_readers()).await;

    let out = agent.run("q").await.expect("turn");
    let usage = out.usage.expect("usage");
    // The failed attempt made a real HTTP request but reported no terminal
    // usage; billing it is the transport attempt-counter's machine (Go:
    // WithRequestAttemptCounter + estimateFailedAttemptUsage), deferred
    // with the usage wave. The billable record carries the clean attempt.
    assert_eq!(usage.request_count, 1, "{usage:?}");
    assert_eq!(usage.finish_reason, "stop");
    assert_eq!(usage.prompt_tokens, 9);

    // One Usage event per round even when it took two attempts.
    assert_eq!(recorder.events_of(kind::USAGE).len(), 1);
}

#[test]
fn merge_sampling_usage_sums_the_billable_counters() {
    let first = Usage {
        prompt_tokens: 100,
        completion_tokens: 10,
        total_tokens: 110,
        cache_hit_tokens: 80,
        cache_miss_tokens: 20,
        request_count: 1,
        ..Usage::default()
    };
    let second = Usage {
        prompt_tokens: 40,
        completion_tokens: 5,
        total_tokens: 45,
        cache_hit_tokens: 30,
        cache_miss_tokens: 10,
        request_count: 1,
        ..Usage::default()
    };
    let merged = merge_sampling_usage(Some(first), Some(&second)).expect("merged");
    assert_eq!(merged.cache_hit_tokens, 110);
    assert_eq!(merged.cache_miss_tokens, 30);
    // Billable input aligns with the cache split (Go: PromptTokens = hit+miss).
    assert_eq!(merged.prompt_tokens, 140);
    assert_eq!(merged.completion_tokens, 15);
    assert_eq!(merged.total_tokens, 155);
    assert_eq!(merged.request_count, 2);
}

#[test]
fn finalize_sampling_usage_keeps_billable_totals_and_latest_finish() {
    let billable = Usage {
        prompt_tokens: 140,
        completion_tokens: 15,
        total_tokens: 100,
        request_count: 2,
        ..Usage::default()
    };
    let latest = Usage {
        prompt_tokens: 40,
        completion_tokens: 5,
        total_tokens: 45,
        finish_reason: "stop".to_owned(),
        ..Usage::default()
    };
    let out = finalize_sampling_usage(Some(&billable), Some(&latest)).expect("finalized");
    assert_eq!(out.finish_reason, "stop");
    assert_eq!(out.request_count, 2);
    // Total never reads below prompt+completion (Go: the finalize clamp).
    assert_eq!(out.total_tokens, 155);
}

#[test]
fn a_usage_record_without_shape_is_not_billable() {
    // merge/finalize of nothing is nothing; the emitter answers the same.
    assert!(merge_sampling_usage(None, None).is_none());
    assert!(finalize_sampling_usage(None, None).is_none());
}
