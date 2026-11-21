//! The response boundary (Go: the `classifyResponseBoundary` cluster in
//! `run_loop.go`/`boundary.go`): what a limit-terminal cut, what still ran,
//! and the host fact that tells the model.

use reasonix_contract::provider::ToolCall;
use reasonix_provider::Usage;

/// Go: `finishReasonMessage` — the notice a limit-terminal deserves.
pub(crate) fn finish_reason_message(usage: Option<&Usage>) -> Option<String> {
    let u = usage?;
    match u.finish_reason.as_str() {
        "length" => Some("response truncated: hit max output tokens".to_owned()),
        "content_filter" => Some("response blocked by content filter".to_owned()),
        "repetition_truncation" => Some("response truncated: model repetition detected".to_owned()),
        _ => None,
    }
}

/// Go: `truncatedTerminal` — the finish reasons that end a response before
/// the model chose to stop. A content filter refused what was written
/// rather than leaving it unfinished, so it is not one.
pub(crate) fn truncated_terminal(usage: Option<&Usage>) -> bool {
    usage.is_some_and(|u| u.finish_reason == "length" || u.finish_reason == "repetition_truncation")
}

/// Go: `truncationCause` — the reason is the identity; this is its wording.
pub(crate) fn truncation_cause(reason: &str) -> &'static str {
    if reason == "repetition_truncation" {
        "was stopped by the host's repetition guard"
    } else {
        "hit the model's output-token limit"
    }
}

/// One sampling terminal classified against the tool calls it carried (Go:
/// `responseBoundary`). Truncation can only have cut the last call, so an
/// unparseable tail is a host fact there and the model's own error anywhere
/// else.
pub(crate) fn classify_response_boundary(
    usage: Option<&Usage>,
    calls: &[ToolCall],
) -> ResponseBoundary {
    if calls.is_empty() {
        return ResponseBoundary::default();
    }
    let last = calls.len() - 1;
    let reported = truncated_terminal(usage);
    let cut = arguments_cut_off(&calls[last].arguments);
    if !reported && !cut {
        return ResponseBoundary {
            committed: calls.to_vec(),
            ..ResponseBoundary::default()
        };
    }
    let reason = usage.map(|u| u.finish_reason.as_str()).unwrap_or("");
    if !cut && serde_json::from_str::<serde_json::Value>(&calls[last].arguments).is_ok() {
        return ResponseBoundary {
            committed: calls.to_vec(),
            fact: truncation_fact(reason, calls.len(), ""),
            ..ResponseBoundary::default()
        };
    }
    ResponseBoundary {
        committed: calls[..last].to_vec(),
        dropped: calls[last].name.clone(),
        fact: truncation_fact(reason, last, &calls[last].name),
    }
}

/// Go: `responseBoundary`.
#[derive(Default)]
pub(crate) struct ResponseBoundary {
    pub committed: Vec<ToolCall>,
    dropped: String,
    /// Host attribution owed to the model, empty on a normal terminal.
    fact: String,
}

impl ResponseBoundary {
    /// Go: `responseBoundary.noticeDetail`.
    pub(crate) fn notice_detail(&self) -> String {
        if self.dropped.is_empty() {
            String::new()
        } else {
            format!(
                "trailing tool call {:?} was incomplete and was not executed",
                self.dropped
            )
        }
    }

    /// Hands the model the host's reading of a response that ended at a
    /// limit; reports whether one was owed (Go: `recordTruncationFact`).
    pub(crate) fn record(&self, add_host_message: impl FnOnce(&str)) -> bool {
        if self.fact.is_empty() {
            return false;
        }
        add_host_message(&self.fact);
        true
    }
}

/// Reports arguments that stop mid-value (Go: `argumentsCutOff`). Empty
/// arguments are a valid call shape, not a cut. serde_json names no
/// unexpected-EOF identity at the top level, so a truncated document is
/// detected by the only observable it leaves: closing the last open
/// delimiter parses.
fn arguments_cut_off(arguments: &str) -> bool {
    if arguments.trim().is_empty() {
        return false;
    }
    if serde_json::from_str::<serde_json::Value>(arguments).is_ok() {
        return false;
    }
    let mut closed = arguments.to_owned();
    closed.push('}');
    if serde_json::from_str::<serde_json::Value>(&closed).is_ok() {
        return true;
    }
    closed.pop();
    closed.push(']');
    serde_json::from_str::<serde_json::Value>(&closed).is_ok()
}

/// What only the host knows: the response ended at a limit (Go:
/// `truncationFact`).
fn truncation_fact(reason: &str, executed: usize, dropped: &str) -> String {
    let mut fact = format!(
        "Host fact about your previous response, not a mistake in your calls: it {} and was cut off.",
        truncation_cause(reason)
    );
    if !dropped.is_empty() {
        fact.push_str(&format!(
            "\n\nThe trailing tool call {dropped:?} was still being written when the cut landed, so \
             the host dropped it: it did not run, its half-written arguments were not repaired or \
             guessed at, and it is not in the conversation. Any error you would expect from it does \
             not exist."
        ));
    }
    match executed {
        0 => fact.push_str("\n\nNothing from that response ran."),
        1 => fact.push_str(
            "\n\nThe 1 tool call that was already complete ran normally; its result is above.",
        ),
        n => fact.push_str(&format!(
            "\n\nThe {n} tool calls that were already complete ran normally; their results are above."
        )),
    }
    fact.push_str(
        "\n\nRe-emit only the work that did not run. Batching independent calls in one response is \
         correct and stays correct — this was a size limit, not a batching failure — but keep this \
         response within it: fewer or shorter arguments per response, and split a long write across \
         calls.",
    );
    fact
}
