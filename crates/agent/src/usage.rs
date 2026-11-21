//! Sampling-usage accounting (Go: `run_usage.go` — `mergeSamplingUsage`,
//! `finalizeSamplingUsage`, `emitTurnUsage`).
//!
//! One model round spans body attempts; billable counters accumulate across
//! them while the context gauges stay the latest attempt's. A failed
//! attempt carries only what its stream produced — estimated token
//! reconstruction is the usage wave's machine, so a failed attempt without
//! terminal usage keeps `None` rather than an invented billable record.

use reasonix_contract::eventwire::usage::Usage as WireUsage;
use reasonix_provider::Usage;

use crate::sink::Sink;
use crate::sink::kind;

fn usage_total_tokens(u: &Usage) -> i64 {
    if u.total_tokens > 0 {
        u.total_tokens
    } else {
        u.prompt_tokens + u.completion_tokens
    }
}

fn usage_request_count(u: Option<&Usage>) -> i64 {
    match u {
        Some(u) if u.request_count > 0 => u.request_count,
        Some(_) => 1,
        None => 0,
    }
}

/// Billable input for one attempt: the cache split when the provider sent
/// one, the prompt otherwise (Go: `billableHitMiss`/`billablePrompt`).
fn billable_hit_miss(u: &Usage) -> (i64, i64) {
    if u.cache_hit_tokens + u.cache_miss_tokens > 0 {
        (u.cache_hit_tokens, u.cache_miss_tokens)
    } else {
        (0, u.prompt_tokens)
    }
}

fn billable_prompt(hit: i64, miss: i64, prompt: i64) -> i64 {
    if hit + miss > 0 { hit + miss } else { prompt }
}

/// Accumulates billable counters across body attempts (Go:
/// `mergeSamplingUsage`).
pub fn merge_sampling_usage(acc: Option<Usage>, attempt: Option<&Usage>) -> Option<Usage> {
    let attempt = attempt?;
    let mut merged = match acc {
        None => {
            let mut merged = attempt.clone();
            if merged.request_count <= 0 {
                merged.request_count = 1;
            }
            let (hit, miss) = billable_hit_miss(attempt);
            merged.cache_hit_tokens = hit;
            merged.cache_miss_tokens = miss;
            merged.prompt_tokens = billable_prompt(hit, miss, attempt.prompt_tokens);
            return Some(merged);
        }
        Some(acc) => acc,
    };
    let (ah, am) = if merged.cache_hit_tokens + merged.cache_miss_tokens > 0 {
        (merged.cache_hit_tokens, merged.cache_miss_tokens)
    } else {
        billable_hit_miss(&merged)
    };
    let (bh, bm) = billable_hit_miss(attempt);
    let acc_prompt = merged.prompt_tokens;
    merged.cache_hit_tokens = ah + bh;
    merged.cache_miss_tokens = am + bm;
    merged.cache_write_tokens += attempt.cache_write_tokens;
    merged.cache_write_billed_tokens += attempt.cache_write_billed_tokens;
    merged.prompt_tokens = billable_prompt(merged.cache_hit_tokens, merged.cache_miss_tokens, 0);
    if merged.prompt_tokens == 0 {
        merged.prompt_tokens = acc_prompt + attempt.prompt_tokens;
    }
    merged.completion_tokens += attempt.completion_tokens;
    merged.reasoning_tokens += attempt.reasoning_tokens;
    merged.total_tokens += usage_total_tokens(attempt);
    merged.request_count = usage_request_count(Some(&merged)) + usage_request_count(Some(attempt));
    if attempt.estimated {
        merged.estimated = true;
    }
    if !attempt.finish_reason.is_empty() {
        merged.finish_reason = attempt.finish_reason.clone();
    }
    Some(merged)
}

/// Builds the coherent billable record consumers expect: billable totals
/// from the aggregate, context gauges and finish reason from the latest
/// attempt (Go: `finalizeSamplingUsage`).
pub fn finalize_sampling_usage(billable: Option<&Usage>, latest: Option<&Usage>) -> Option<Usage> {
    let mut out = match (billable, latest) {
        (None, None) => return None,
        (None, Some(latest)) => latest.clone(),
        (Some(billable), latest) => {
            let mut out = billable.clone();
            if let Some(latest) = latest {
                out.finish_reason = latest.finish_reason.clone();
            }
            out
        }
    };
    let hit_miss = out.cache_hit_tokens + out.cache_miss_tokens;
    if hit_miss > 0 {
        out.prompt_tokens = hit_miss;
    }
    if out.total_tokens < out.prompt_tokens + out.completion_tokens {
        out.total_tokens = out.prompt_tokens + out.completion_tokens;
    }
    Some(out)
}

/// The eventwire projection of one usage record.
pub fn to_wire_usage(u: &Usage) -> WireUsage {
    WireUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        cache_hit_tokens: u.cache_hit_tokens,
        cache_miss_tokens: u.cache_miss_tokens,
        reasoning_tokens: u.reasoning_tokens,
        estimated: u.estimated,
        ..WireUsage::default()
    }
}

/// Emits the turn's Usage event (Go: `emitTurnUsage`). A record with no
/// shape emits nothing: an event that reads as "the model cost nothing"
/// teaches the wrong lesson.
pub fn emit_turn_usage(sink: &dyn Sink, usage: &Usage, model_ref: &str, source: &str) {
    if usage.total_tokens <= 0 && usage.request_count <= 0 && usage.prompt_tokens <= 0 {
        return;
    }
    let mut wire = to_wire_usage(usage);
    if !source.is_empty() {
        wire.source = source.to_owned();
    }
    sink.emit(reasonix_contract::eventwire::event::Event {
        kind: kind::USAGE.to_owned(),
        model_ref: model_ref.to_owned(),
        usage: Some(wire),
        ..reasonix_contract::eventwire::event::Event::default()
    });
}
