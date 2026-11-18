//! Message normalization (Go: 'provider.go' normalization entry points,
//! 'tool_args.go'). Behavior frozen in docs/contracts/provider-message.md §7.
//!
//! Normalization is total: every input yields a well-formed, model-acceptable
//! message list. It is the gate between stored history (any shape the log
//! preserved) and the two consumers: the provider wire (strict pairing,
//! orphan-free) and session display (lossless, receipts folded). Healthy
//! histories pass through without allocating: entry points return
//! [Cow::Borrowed] over the caller's slice, the Rust shape of Go's
//! same-backing-array fast path.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use super::{INTERRUPTED_TOOL_RESULT, Message, Role, ToolCall};

/// Provider wire-safe normalization (Go: 'NormalizeMessages'; alias
/// 'SanitizeToolPairing'). Every assistant tool call is answered, orphan tool
/// messages are dropped, truncated call arguments are closed (Go:
/// provider.go:255).
pub fn normalize_messages(msgs: &[Message]) -> Cow<'_, [Message]> {
    normalize_impl(Cow::Borrowed(msgs), true)
}

/// Alias of [normalize_messages] kept so call sites read as defensive wire
/// prep (Go: 'SanitizeToolPairing', provider.go:235).
pub fn sanitize_tool_pairing(msgs: &[Message]) -> Cow<'_, [Message]> {
    normalize_messages(msgs)
}

/// Session-view normalization (Go: 'NormalizeSessionMessages',
/// provider.go:264): shares the assistant-turn repairs but preserves stored
/// tool messages so save/load stays a lossless round trip.
pub fn normalize_session_messages(msgs: &[Message]) -> Cow<'_, [Message]> {
    normalize_impl(attach_standalone_decision_receipts(msgs), false)
}

/// Folds the short-lived receipt encoding that stored a LocalOnly message
/// after an assistant message into that assistant's receipts (Go:
/// 'attachStandaloneDecisionReceipts', provider.go:273). Healthy histories
/// return the input unchanged.
pub fn attach_standalone_decision_receipts(msgs: &[Message]) -> Cow<'_, [Message]> {
    let mut needs_migration = false;
    {
        let mut target: Option<usize> = None;
        for m in msgs {
            if m.role == Role::USER && !m.local_only {
                target = None;
            } else if m.role == Role::ASSISTANT && !m.local_only {
                target = Some(0);
            } else if target.is_some() && m.local_only && m.decision_receipt.is_some() {
                needs_migration = true;
                break;
            }
        }
    }
    if !needs_migration {
        return Cow::Borrowed(msgs);
    }
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len());
    let mut target: Option<usize> = None;
    for m in msgs {
        if m.role == Role::USER && !m.local_only {
            target = None;
        } else if m.role == Role::ASSISTANT && !m.local_only {
            out.push(m.clone());
            target = Some(out.len() - 1);
            continue;
        } else if let Some(t) = target
            && m.local_only
            && m.decision_receipt.is_some()
        {
            out[t].decision_receipts.push(m.decision_receipt.clone());
            continue;
        }
        out.push(m.clone());
    }
    Cow::Owned(out)
}

fn normalize_impl<'a>(msgs: Cow<'a, [Message]>, drop_orphan_tools: bool) -> Cow<'a, [Message]> {
    if try_normalize_fast_path(&msgs, drop_orphan_tools) {
        return msgs;
    }
    Cow::Owned(normalize_slow(&msgs, drop_orphan_tools))
}

/// Reports whether msgs needs no repair (Go: 'tryNormalizeFastPath',
/// provider.go:365): no LocalOnly message while dropping orphans, every
/// assistant-with-calls turn well formed with intact arguments, no bare tool
/// message while dropping orphans.
fn try_normalize_fast_path(msgs: &[Message], drop_orphan_tools: bool) -> bool {
    let mut i = 0;
    while i < msgs.len() {
        let m = &msgs[i];
        if m.local_only {
            if drop_orphan_tools {
                return false;
            }
            i += 1;
            continue;
        }
        if m.role == Role::ASSISTANT && !m.tool_calls.is_empty() {
            let mut j = i + 1;
            while j < msgs.len() && msgs[j].role == Role::TOOL && !msgs[j].local_only {
                j += 1;
            }
            if !tool_turn_well_formed(&m.tool_calls, &msgs[i + 1..j])
                || needs_tool_call_arg_repair(&m.tool_calls, drop_orphan_tools)
            {
                return false;
            }
            i = j;
            continue;
        }
        if m.role == Role::TOOL && drop_orphan_tools {
            return false;
        }
        i += 1;
    }
    true
}

fn normalize_slow(msgs: &[Message], drop_orphan_tools: bool) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len());
    let mut i = 0;
    while i < msgs.len() {
        let m = &msgs[i];
        if m.local_only {
            if !drop_orphan_tools {
                out.push(m.clone());
            }
            i += 1;
            continue;
        }
        if m.role == Role::ASSISTANT && !m.tool_calls.is_empty() {
            let mut j = i + 1;
            while j < msgs.len() && msgs[j].role == Role::TOOL && !msgs[j].local_only {
                j += 1;
            }
            let avail = &msgs[i + 1..j];
            let mut assistant = m.clone();
            let calls: Vec<ToolCall> = match backfill_tool_call_names(&assistant.tool_calls, avail)
            {
                Cow::Borrowed(c) => c.to_vec(),
                Cow::Owned(c) => c,
            };
            assistant.tool_calls = calls.clone();
            out.push(repair_tool_call_args(assistant, drop_orphan_tools));
            if drop_orphan_tools {
                out.extend(pair_tool_results(&calls, avail));
            } else {
                out.extend(session_tool_results(&calls, avail));
            }
            i = j;
            continue;
        }
        if m.role == Role::TOOL {
            if !drop_orphan_tools {
                out.push(m.clone());
            }
            i += 1;
            continue;
        }
        out.push(m.clone());
        i += 1;
    }
    out
}

/// Whether calls and results already satisfy the wire contract (Go:
/// 'toolTurnWellFormed', provider.go:394).
pub fn tool_turn_well_formed(calls: &[ToolCall], results: &[Message]) -> bool {
    if calls.len() != results.len() {
        return false;
    }
    for tc in calls {
        if tc.name.is_empty() {
            return false;
        }
    }
    for (k, tc) in calls.iter().enumerate() {
        if results[k].tool_call_id != tc.id {
            return false;
        }
        if results[k].name != tc.name {
            return false;
        }
    }
    true
}

/// Whether every call carries a non-empty id unique within the batch (Go:
/// 'idDistinct', provider.go:521).
pub fn id_distinct(calls: &[ToolCall]) -> bool {
    let mut seen: HashSet<&str> = HashSet::with_capacity(calls.len());
    for tc in calls {
        if tc.id.is_empty() {
            return false;
        }
        if !seen.insert(tc.id.as_str()) {
            return false;
        }
    }
    true
}

/// Placeholder standing in for a tool result that never landed (Go:
/// provider.go:227,432).
fn interrupted_placeholder(tc: &ToolCall) -> Message {
    Message {
        role: Role::TOOL,
        tool_call_id: tc.id.clone(),
        name: tc.name.clone(),
        content: INTERRUPTED_TOOL_RESULT.to_owned(),
        ..Message::default()
    }
}

/// Provider-mode pairing (Go: 'pairToolResults', provider.go:420): distinct
/// ids pair by id (reordered results re-sort to call order), missing calls get
/// a placeholder; non-distinct ids pair positionally with identity overwritten
/// from the call.
pub fn pair_tool_results(calls: &[ToolCall], avail: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::with_capacity(calls.len());
    if id_distinct(calls) {
        let mut by_id: HashMap<&str, &Message> = HashMap::with_capacity(avail.len());
        for r in avail {
            by_id.insert(r.tool_call_id.as_str(), r);
        }
        for tc in calls {
            match by_id.get(tc.id.as_str()) {
                Some(r) => {
                    let mut r = (*r).clone();
                    r.name = tc.name.clone();
                    out.push(r);
                }
                None => out.push(interrupted_placeholder(tc)),
            }
        }
        return out;
    }
    for (k, tc) in calls.iter().enumerate() {
        if k < avail.len() {
            let mut r = avail[k].clone();
            r.tool_call_id = tc.id.clone();
            r.name = tc.name.clone();
            out.push(r);
        } else {
            out.push(interrupted_placeholder(tc));
        }
    }
    out
}

/// Session-mode pairing (Go: 'sessionToolResults', provider.go:454): keeps
/// every stored result in order and appends placeholders only for calls with
/// no recorded answer.
pub fn session_tool_results(calls: &[ToolCall], avail: &[Message]) -> Vec<Message> {
    let mut out: Vec<Message> = avail.to_vec();
    if id_distinct(calls) {
        let answered: HashSet<&str> = avail.iter().map(|r| r.tool_call_id.as_str()).collect();
        for tc in calls {
            if !answered.contains(tc.id.as_str()) {
                out.push(interrupted_placeholder(tc));
            }
        }
        return out;
    }
    for tc in calls.iter().skip(avail.len()) {
        out.push(interrupted_placeholder(tc));
    }
    out
}

/// Backfills empty call names from the matching results (Go:
/// 'backfillToolCallNames', provider.go:481; #4727). No empty names returns
/// the input borrowed, without copying.
pub fn backfill_tool_call_names<'a>(
    calls: &'a [ToolCall],
    results: &[Message],
) -> Cow<'a, [ToolCall]> {
    if !calls.iter().any(|c| c.name.is_empty()) {
        return Cow::Borrowed(calls);
    }
    let mut out = calls.to_vec();
    if id_distinct(calls) {
        let mut by_id: HashMap<&str, &str> = HashMap::with_capacity(results.len());
        for r in results {
            if !r.name.is_empty() {
                by_id.insert(r.tool_call_id.as_str(), r.name.as_str());
            }
        }
        for call in &mut out {
            if call.name.is_empty()
                && let Some(name) = by_id.get(call.id.as_str())
            {
                call.name = (*name).to_owned();
            }
        }
        return Cow::Owned(out);
    }
    for (k, call) in out.iter_mut().enumerate() {
        if call.name.is_empty() && k < results.len() {
            call.name = results[k].name.clone();
        }
    }
    Cow::Owned(out)
}

/// Whether a recorded argument string would be refused by a strict gateway
/// (Go: 'argsNeedRepair', tool_args.go:19). Empty arguments are repaired only
/// on the provider path; non-empty invalid JSON is repaired in both modes.
pub fn args_need_repair(args: &str, for_provider: bool) -> bool {
    if args.trim().is_empty() {
        return for_provider;
    }
    !json_valid(args)
}

/// Whether any call in the batch needs argument repair (Go:
/// 'needsToolCallArgRepair', tool_args.go:28).
pub fn needs_tool_call_arg_repair(calls: &[ToolCall], for_provider: bool) -> bool {
    calls
        .iter()
        .any(|tc| args_need_repair(&tc.arguments, for_provider))
}

/// Copy-on-write argument repair (Go: 'repairToolCallArgs', tool_args.go:37):
/// empty arguments become an empty object on the provider path, non-empty
/// invalid JSON is closed. The message is taken by value so the caller's
/// history is never mutated.
pub fn repair_tool_call_args(mut m: Message, for_provider: bool) -> Message {
    if !needs_tool_call_arg_repair(&m.tool_calls, for_provider) {
        return m;
    }
    for call in &mut m.tool_calls {
        if !args_need_repair(&call.arguments, for_provider) {
            continue;
        }
        if call.arguments.trim().is_empty() {
            call.arguments = "{}".to_owned();
            continue;
        }
        call.arguments = close_truncated_json(&call.arguments);
    }
    m
}

/// Best-effort completes a JSON document cut off mid-stream (Go:
/// 'closeTruncatedJSON', tool_args.go:60): unterminated strings and open
/// containers are closed, a dangling comma is dropped, a dangling colon gets
/// null; anything still invalid degrades to an empty object.
pub fn close_truncated_json(s: &str) -> String {
    let mut stack: Vec<u8> = Vec::new();
    let mut in_str = false;
    let mut esc = false;
    for &c in s.as_bytes() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    let mut out = s.to_owned();
    if esc {
        out.pop();
    }
    if in_str {
        out.push('"');
    }
    let trimmed_len = out.trim_end_matches([' ', '\t', '\r', '\n']).len();
    let trimmed = &out[..trimmed_len];
    if trimmed.ends_with(',') {
        out.truncate(trimmed_len - 1);
    } else if trimmed.ends_with(':') {
        out.truncate(trimmed_len);
        out.push_str("null");
    }
    while let Some(closer) = stack.pop() {
        out.push(closer as char);
    }
    if !json_valid(&out) {
        return "{}".to_owned();
    }
    out
}

/// Go json.Valid equivalent for the argument corpus: one JSON value with
/// optional surrounding whitespace.
fn json_valid(s: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(s).is_ok()
}
