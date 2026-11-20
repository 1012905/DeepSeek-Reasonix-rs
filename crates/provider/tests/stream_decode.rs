//! SSE stream decoding oracles (Go: `openai/openai_test.go`,
//! `empty_stream_test.go`, `stream_payload_error_test.go`, `think_test.go`,
//! `usage_capture_test.go`).

use reasonix_provider::WireUsage;
use reasonix_provider::error::{
    ProviderError, STREAM_INTERRUPT_IDLE_TIMEOUT, STREAM_INTERRUPT_PREMATURE_EOF,
    STREAM_INTERRUPT_UPSTREAM_ERROR,
};
use reasonix_provider::stream::{Chunk, StreamDecoder};
use reasonix_provider::think::ThinkSplitter;
use reasonix_provider::usage::normalize_usage;

/// Feeds frame text to a fresh decoder and drains to the end.
fn run(frames: &[&str]) -> (Vec<Chunk>, Option<ProviderError>) {
    let mut decoder = StreamDecoder::new("relay");
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.feed(format!("{frame}\n\n").as_bytes()));
    }
    out.extend(decoder.finish());
    let outcome = decoder.outcome();
    (out, outcome)
}

fn text_of(chunks: &[Chunk]) -> String {
    chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn reasoning_of(chunks: &[Chunk]) -> String {
    chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Reasoning(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

// Go TestStreamReadsReasoningFallbackField.
#[test]
fn reads_reasoning_fallback_field() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"reasoning":"vllm thinking","content":"answer"}}]}"#,
        "data: [DONE]",
    ]);
    assert!(err.is_none());
    assert_eq!(reasoning_of(&chunks), "vllm thinking");
    assert_eq!(text_of(&chunks), "answer");
    assert!(matches!(chunks.last(), Some(Chunk::Done)));
}

// Go TestStreamReasoningContentTakesPrecedenceOverFallback.
#[test]
fn reasoning_content_takes_precedence_over_fallback() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"reasoning_content":"standard","reasoning":"fallback"}}]}"#,
        "data: [DONE]",
    ]);
    assert!(err.is_none());
    assert_eq!(reasoning_of(&chunks), "standard");
}

// Go TestStreamSynthesizesMissingToolCallIDs.
#[test]
fn synthesizes_missing_tool_call_ids() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\"p\":\"a\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"name":"read_file","arguments":"{\"p\":\"b\"}"}}]}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#,
        "data: [DONE]",
    ]);
    assert!(err.is_none());
    let ids: Vec<String> = chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::ToolCall(tc) => Some(tc.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2, "want 2 tool calls: {chunks:?}");
    assert!(
        !ids[0].is_empty() && !ids[1].is_empty(),
        "empty id: {ids:?}"
    );
    assert_ne!(ids[0], ids[1], "synthesized ids must be distinct");
    // The start chunks fired the moment each name was known.
    let starts = chunks
        .iter()
        .filter(|c| matches!(c, Chunk::ToolCallStart { .. }))
        .count();
    assert_eq!(starts, 2);
    // Usage carries the finish reason it saw.
    let usage = chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Usage(u) => Some(u.clone()),
            _ => None,
        })
        .next()
        .expect("usage chunk");
    assert_eq!(usage.finish_reason, "tool_calls");
    assert_eq!(usage.prompt_tokens, 5);
}

// Go TestStreamReadsGeminiThoughtSignature (both shapes).
#[test]
fn reads_gemini_thought_signature() {
    for frame in [
        r#"{"index":0,"id":"call_abc123","type":"function","extra_content":{"google":{"thought_signature":"gemini_sig_xyz789"}},"function":{"name":"write_file"}}"#,
        r#"{"index":0,"id":"call_abc123","type":"function","function":{"name":"write_file","thought_signature":"gemini_sig_xyz789"}}"#,
    ] {
        let (chunks, err) = run(&[
            &format!(r#"data: {{"choices":[{{"delta":{{"tool_calls":[{frame}]}}}}]}}"#),
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"test.txt\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ]);
        assert!(err.is_none());
        let tc = chunks
            .iter()
            .find_map(|c| match c {
                Chunk::ToolCall(tc) => Some(tc.clone()),
                _ => None,
            })
            .expect("ChunkToolCall");
        assert_eq!(tc.thought_signature, "gemini_sig_xyz789");
        assert_eq!(tc.arguments, r#"{"path":"test.txt"}"#);
    }
}

// Go TestAnEmptyStreamIsAnUpstreamFailureNotAnEmptyAnswer: a heartbeat and a
// clean end is the far side failing, not the model answering with nothing.
#[test]
fn empty_stream_is_an_upstream_failure_not_an_empty_answer() {
    let (chunks, err) = run(&[": OPENROUTER PROCESSING", "data: [DONE]"]);
    assert!(
        chunks
            .iter()
            .all(|c| !matches!(c, Chunk::Text(_) | Chunk::Usage(_) | Chunk::Reasoning(_))),
        "nothing meaningful may be carried: {chunks:?}"
    );
    let Some(err) = err else {
        panic!("a stream that carried nothing ended silently");
    };
    assert!(
        err.is_stream_interrupted(),
        "want a replayable interruption: {err:?}"
    );
    assert_eq!(
        err.stream_interrupt_reason(),
        STREAM_INTERRUPT_UPSTREAM_ERROR
    );
    assert_eq!(err.to_string(), "relay: the upstream sent an empty stream");
}

// Go TestAUsageOnlyStreamIsStillTheModelAnswering.
#[test]
fn usage_only_stream_is_still_the_model_answering() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":0}}"#,
        "data: [DONE]",
    ]);
    assert!(
        err.is_none(),
        "usage-bearing stream reported as failure: {err:?}"
    );
    assert!(chunks.iter().any(|c| matches!(c, Chunk::Usage(_))));
    assert!(matches!(chunks.last(), Some(Chunk::Done)));
}

const UPSTREAM_DOWN: &str = r#"{"error":{"message":"Upstream service temporarily unavailable"}}"#;

// Go TestUpstreamErrorBeforeAnyOutputIsReplayable: the gateway's own sentence
// is kept, the failure is replayable.
#[test]
fn upstream_error_before_any_output_is_replayable() {
    let (chunks, err) = run(&[&format!("data: {UPSTREAM_DOWN}")]);
    assert!(
        chunks.iter().all(|c| !matches!(c, Chunk::Text(_))),
        "emitted text before the error"
    );
    let Some(err) = err else {
        panic!("no failure surfaced")
    };
    assert!(err.is_stream_interrupted(), "{err:?}");
    assert_eq!(
        err.stream_interrupt_reason(),
        STREAM_INTERRUPT_UPSTREAM_ERROR
    );
    assert_eq!(
        err.to_string(),
        "relay: Upstream service temporarily unavailable"
    );
}

// Go TestUpstreamErrorAfterOutputStaysTerminal.
#[test]
fn upstream_error_after_output_stays_terminal() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"content":"partial "}}]}"#,
        &format!("data: {UPSTREAM_DOWN}"),
    ]);
    assert_eq!(text_of(&chunks), "partial ");
    let err = err.expect("terminal failure");
    assert!(
        !err.is_stream_interrupted(),
        "a failure after output was replayed: {err:?}"
    );
    assert!(matches!(err, ProviderError::StreamPayload { .. }));
    assert_eq!(
        err.to_string(),
        "relay: Upstream service temporarily unavailable"
    );
}

// A clean close before any terminal the protocol accepts (Go: the #3953
// premature-cut check) is a premature_eof interruption.
#[test]
fn premature_cut_without_done_or_finish_reason() {
    let (chunks, err) = run(&[r#"data: {"choices":[{"delta":{"content":"half"}}]}"#]);
    assert_eq!(text_of(&chunks), "half");
    let err = err.expect("premature cut");
    assert_eq!(
        err.stream_interrupt_reason(),
        STREAM_INTERRUPT_PREMATURE_EOF
    );
    assert!(err.to_string().contains("stream ended before completion"));
}

// A legal finish_reason is a complete terminal even without [DONE].
#[test]
fn finish_reason_is_a_legal_terminal_without_done() {
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
    ]);
    assert!(err.is_none());
    assert!(matches!(chunks.last(), Some(Chunk::Done)));
}

// A malformed data frame is a typed decode failure with a bounded excerpt.
#[test]
fn malformed_frame_is_a_typed_decode_failure() {
    let (chunks, err) = run(&["data: {\"choices\": [not-json"]);
    assert!(
        chunks.iter().all(|c| !matches!(c, Chunk::Done)),
        "no Done after a decode failure"
    );
    let err = err.expect("decode failure");
    match err {
        ProviderError::StreamDecode {
            provider, payload, ..
        } => {
            assert_eq!(provider, "relay");
            assert!(payload.contains("choices"), "excerpt quoted: {payload}");
        }
        other => panic!("want StreamDecode, got {other:?}"),
    }
}

// Tool-call arguments accumulate across deltas and a progress tick fires per
// 2KB bucket once the call has started.
#[test]
fn tool_call_arguments_accumulate_with_progress_ticks() {
    let big = "x".repeat(2048);
    let (chunks, err) = run(&[
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"write_file","arguments":"{"}}]}}]}"#,
        &format!(
            r#"data: {{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":"{}"}}}}]}}}}]}}"#,
            big
        ),
        &format!(
            r#"data: {{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"function":{{"arguments":"{}"}}}}]}}}}]}}"#,
            "}"
        ),
        "data: [DONE]",
    ]);
    assert!(err.is_none());
    let deltas: Vec<usize> = chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::ToolCallArgsDelta { arg_chars, .. } => Some(*arg_chars),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec![2049],
        "one tick per 2KB bucket, cumulative: {deltas:?}"
    );
    let tc = chunks
        .iter()
        .find_map(|c| match c {
            Chunk::ToolCall(tc) => Some(tc.clone()),
            _ => None,
        })
        .expect("complete tool call");
    assert_eq!(tc.id, "c1");
    assert_eq!(tc.arguments.len(), 2050);
}

// Interruption identity: the stall reason is named by the transport, not by
// matching words (Go: stallCut attaches StreamInterruptIdleTimeout).
#[test]
fn stall_error_carries_idle_timeout_reason() {
    let err = reasonix_provider::error::ProviderError::StreamInterrupted {
        reason: STREAM_INTERRUPT_IDLE_TIMEOUT.to_owned(),
        message: "relay: stream stalled — no data for 5s".to_owned(),
    };
    assert_eq!(err.stream_interrupt_reason(), STREAM_INTERRUPT_IDLE_TIMEOUT);
}

// Go TestNormaliseUsageDeepSeekShape.
#[test]
fn normalize_usage_deepseek_shape() {
    let u = normalize_usage(&WireUsage {
        prompt_tokens: 1000,
        completion_tokens: 200,
        total_tokens: 1200,
        prompt_cache_hit_tokens: 900,
        prompt_cache_miss_tokens: 100,
        ..WireUsage::default()
    });
    assert_eq!(u.cache_hit_tokens, 900);
    assert_eq!(u.cache_miss_tokens, 100);
}

// Go TestNormaliseUsageMiMoShape.
#[test]
fn normalize_usage_mimo_shape() {
    let u = normalize_usage(&serde_json::from_str::<WireUsage>(
        r#"{"prompt_tokens":1000,"completion_tokens":500,"total_tokens":1500,"prompt_tokens_details":{"cached_tokens":600},"completion_tokens_details":{"reasoning_tokens":180}}"#,
    ).unwrap());
    assert_eq!(u.cache_hit_tokens, 600);
    assert_eq!(u.cache_miss_tokens, 400);
    assert_eq!(u.reasoning_tokens, 180);
}

// Go TestNormaliseUsageCacheSplitEdgeCases.
#[test]
fn normalize_usage_cache_split_edge_cases() {
    let cases: [(&str, &str, i64, i64); 4] = [
        (
            "explicit full miss",
            r#"{"prompt_tokens":1000,"completion_tokens":200,"prompt_tokens_details":{"cached_tokens":0}}"#,
            0,
            1000,
        ),
        (
            "no cache fields stays unknown",
            r#"{"prompt_tokens":1000,"completion_tokens":200}"#,
            0,
            0,
        ),
        (
            "over-reported hit clamps to prompt",
            r#"{"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":4000}}"#,
            1000,
            0,
        ),
        (
            "deepseek explicit full miss kept",
            r#"{"prompt_tokens":1000,"prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":1000}"#,
            0,
            1000,
        ),
    ];
    for (name, json, hit, miss) in cases {
        let u = normalize_usage(&serde_json::from_str::<WireUsage>(json).unwrap());
        assert_eq!(
            (u.cache_hit_tokens, u.cache_miss_tokens),
            (hit, miss),
            "cache split wrong for {name}"
        );
    }
}

// Go TestNormaliseUsageAnthropicStyleFallback — fixtures carried verbatim.
#[test]
fn normalize_usage_anthropic_style_fallback() {
    let cases: [(&str, &str, i64, i64, i64, i64, i64); 4] = [
        (
            "cache hit",
            r#"{"usage":{"input_tokens":21,"output_tokens":393,"cache_creation_input_tokens":0,"cache_read_input_tokens":188086}}"#,
            188107,
            393,
            188500,
            188086,
            21,
        ),
        (
            "cache creation",
            r#"{"usage":{"input_tokens":21,"output_tokens":393,"cache_creation_input_tokens":188086,"cache_read_input_tokens":0}}"#,
            188107,
            393,
            188500,
            0,
            188107,
        ),
        (
            "DeepSeek fields take priority",
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150,"prompt_cache_hit_tokens":30,"prompt_cache_miss_tokens":70,"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":888,"cache_read_input_tokens":999}}"#,
            100,
            50,
            150,
            30,
            70,
        ),
        (
            "OpenAI nested fields take priority",
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150,"prompt_tokens_details":{"cached_tokens":40},"input_tokens":1,"output_tokens":2,"cache_creation_input_tokens":888,"cache_read_input_tokens":999}}"#,
            100,
            50,
            150,
            40,
            60,
        ),
    ];
    for (name, json, prompt, completion, total, hit, miss) in cases {
        let outer: serde_json::Value = serde_json::from_str(json).unwrap();
        let usage = outer.get("usage").unwrap();
        let u = normalize_usage(&serde_json::from_value::<WireUsage>(usage.clone()).unwrap());
        assert_eq!(
            (
                u.prompt_tokens,
                u.completion_tokens,
                u.total_tokens,
                u.cache_hit_tokens,
                u.cache_miss_tokens
            ),
            (prompt, completion, total, hit, miss),
            "usage wrong for {name}"
        );
    }
}

// Go TestThinkSplitter — all eight cases.
#[test]
fn think_splitter() {
    let cases: [(&str, &[&str], &str, &str); 8] = [
        (
            "whole block in one delta",
            &["<think>reasoning here</think>the answer"],
            "reasoning here",
            "the answer",
        ),
        (
            "open tag split across deltas",
            &["<th", "ink>chain", " of thought</think>answer"],
            "chain of thought",
            "answer",
        ),
        (
            "close tag split across deltas",
            &["<think>thinking</thi", "nk>done"],
            "thinking",
            "done",
        ),
        (
            "leading whitespace before think is dropped",
            &["\n\n  <think>r</think>\n\nanswer"],
            "r",
            "answer",
        ),
        (
            "no think tag passes through as text",
            &["just a normal ", "answer"],
            "",
            "just a normal answer",
        ),
        (
            "think mentioned mid-answer is not hijacked",
            &["the model emits <think> tags around its reasoning"],
            "",
            "the model emits <think> tags around its reasoning",
        ),
        (
            "unterminated think block flushes as reasoning",
            &["<think>still thinking when the stream ended"],
            "still thinking when the stream ended",
            "",
        ),
        (
            "per-character streaming",
            &["<", "t", "h", "i", "n", "k", ">", "a", "</", "think>", "b"],
            "a",
            "b",
        ),
    ];
    for (name, deltas, want_reasoning, want_text) in cases {
        let mut t = ThinkSplitter::default();
        let mut reasoning = String::new();
        let mut text = String::new();
        for d in deltas {
            let (r, txt) = t.push(d);
            reasoning.push_str(&r);
            text.push_str(&txt);
        }
        let (r, txt) = t.flush();
        reasoning.push_str(&r);
        text.push_str(&txt);
        assert_eq!(reasoning, want_reasoning, "reasoning wrong for {name}");
        assert_eq!(text, want_text, "text wrong for {name}");
    }
}

// Non-streaming body decode: the standard one-shot shape (extension beyond
// the always-streaming Go adapter).
#[test]
fn decode_completion_body() {
    let body = r#"{"choices":[{"message":{"role":"assistant","content":"hi","reasoning_content":"thought"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    let out = reasonix_provider::decode_completion(body).unwrap();
    assert_eq!(out.text, "hi");
    assert_eq!(out.reasoning, "thought");
    assert_eq!(out.finish_reason, "stop");
    assert_eq!(out.usage.prompt_tokens, 3);
    assert_eq!(out.usage.completion_tokens, 2);
}
