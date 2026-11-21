//! Token estimation (Go: `output_budget.go` estimation half, `compact.go`
//! `msgChars`/`fixedTokenEstimate`, `tool_image_aging.go`
//! `requestImageTokens`).
//!
//! Same-session usage calibrates the ~4-chars-per-token cold shape; the
//! provider's own count outranks the estimate wherever both exist. Image
//! bytes are priced without decoding: the Rust tree has no image-size
//! reader yet, so every image is priced at the largest a request may carry
//! (Go: `visionimage.MaxDim`), which errs high and folds early rather than
//! overflowing. The vision slice owns the real header read.

use std::sync::Mutex;

use reasonix_contract::provider::{Message, Role};
use reasonix_provider::{ChatCompletionRequest, Usage};

use super::SharedWindowPolicy;

/// ~4 chars/token, used before any usage is available to calibrate (Go:
/// `fallbackTokPerChar`).
pub const FALLBACK_TOK_PER_CHAR: f64 = 0.25;

/// The margin kept between the estimated prompt and the window, floor
/// (Go: `outputBudgetReserve`).
pub const OUTPUT_BUDGET_RESERVE: i64 = 8 * 1024;

/// The largest image a request may carry, in pixels per side (Go:
/// `visionimage.MaxDim`).
const IMAGE_MAX_DIM: i64 = 1568;

/// One image priced at the fallback dimensions (Go: `imageTokenEstimate`
/// with an unreadable header).
pub const IMAGE_TOKEN_ESTIMATE_FALLBACK: i64 = (IMAGE_MAX_DIM * IMAGE_MAX_DIM + 749) / 750;

/// The margin kept between the estimated prompt and the window. The
/// estimate is our own arithmetic, so its error grows with the prompt: a
/// fixed margin that covers a 200K window leaves a 1M one to a 400 every
/// turn (Go: `outputBudgetReserveFor`).
pub fn output_budget_reserve_for(window: i64) -> i64 {
    OUTPUT_BUDGET_RESERVE.max(window / 32)
}

/// The conservative provider-visible text and CJK composition used for
/// overflow protection (Go: `requestCalibrationShape`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestCalibrationShape {
    pub request_chars: i64,
    pub compact_chars: i64,
    pub cjk_runes: i64,
    pub cjk_bytes: i64,
    /// Estimated, never learned from (Go: `imageTokens`).
    pub image_tokens: i64,
}

impl RequestCalibrationShape {
    /// Accumulates one wire string's length and CJK composition (Go:
    /// `addText`).
    fn add_text(&mut self, s: &str) {
        self.request_chars += s.len() as i64;
        for r in s.chars() {
            if is_cjk_rune(r) {
                self.cjk_runes += 1;
                self.cjk_bytes += r.len_utf8() as i64;
            }
        }
    }

    fn plus(&self, o: &RequestCalibrationShape) -> RequestCalibrationShape {
        RequestCalibrationShape {
            request_chars: self.request_chars + o.request_chars,
            compact_chars: self.compact_chars + o.compact_chars,
            cjk_runes: self.cjk_runes + o.cjk_runes,
            cjk_bytes: self.cjk_bytes + o.cjk_bytes,
            image_tokens: self.image_tokens + o.image_tokens,
        }
    }
}

/// Go: `isCJKRune` — the scripts whose per-character cost differs enough
/// from the aggregate ratio to need its own accounting.
pub fn is_cjk_rune(r: char) -> bool {
    let c = r as u32;
    (0x4E00..=0x9FFF).contains(&c)
        || (0x3400..=0x4DBF).contains(&c)
        || (0x3040..=0x30FF).contains(&c)
        || (0xAC00..=0xD7AF).contains(&c)
}

/// Counts the characters that ride to the provider for one message —
/// content plus tool-call names and arguments, but not reasoning (stripped
/// on send) (Go: `msgChars`).
pub fn msg_chars(m: &Message) -> i64 {
    if m.local_only {
        return 0;
    }
    let mut n = m.content.len() as i64;
    for tc in &m.tool_calls {
        n += tc.name.len() as i64 + tc.arguments.len() as i64;
    }
    n
}

/// What every verbatim-retention decision measures with (Go:
/// `fixedTokenEstimate`). Calibrated usage must not be used here: a
/// threshold that moves with the last turn's ratio would keep a turn at one
/// checkpoint and fold it at the next.
pub fn fixed_token_estimate(m: &Message) -> i64 {
    (msg_chars(m) as f64 * FALLBACK_TOK_PER_CHAR) as i64
}

/// Estimates what the images a request built from msgs would carry cost in
/// the window, after aging (Go: `requestImageTokens`). Tool-image aging is
/// the aging machine's concern (a later wave); until it lands every stored
/// image is priced, which again errs high.
pub fn request_image_tokens(msgs: &[Message]) -> i64 {
    let mut tokens = 0i64;
    for m in msgs {
        if m.local_only {
            continue;
        }
        tokens += m.images.len() as i64 * IMAGE_TOKEN_ESTIMATE_FALLBACK;
    }
    tokens
}

/// Counts one message's shared-window text plus only the replay fields the
/// active provider declares, keeping omitted bytes out of the ratio (Go:
/// `messageCalibrationShape`).
pub fn message_calibration_shape(
    msg: &Message,
    policy: &SharedWindowPolicy,
) -> RequestCalibrationShape {
    let mut shape = RequestCalibrationShape::default();
    if msg.local_only {
        return shape;
    }
    shape.compact_chars = msg_chars(msg);
    shape.request_chars = 4;
    shape.add_text(msg.role.as_str());
    shape.add_text(&msg.content);
    if msg.role == Role::ASSISTANT
        && (!msg.tool_calls.is_empty() || policy.replays_ordinary_reasoning)
    {
        shape.add_text(&msg.reasoning_content);
    }
    shape.add_text(&msg.name);
    shape.add_text(&msg.tool_call_id);
    for call in &msg.tool_calls {
        shape.request_chars += 8;
        shape.add_text(&call.id);
        shape.add_text(&call.name);
        shape.add_text(&call.arguments);
    }
    if policy.replays_responses_items {
        for item in &msg.responses_items {
            shape.add_text(item.get().to_string().as_str());
        }
    }
    shape
}

/// The shared-window shape of one request (Go:
/// `requestCalibrationShapeWithPolicy` over the request copy).
pub fn request_calibration_shape(
    req: &ChatCompletionRequest,
    policy: SharedWindowPolicy,
) -> RequestCalibrationShape {
    let mut shape = RequestCalibrationShape::default();
    for msg in &req.messages {
        shape = shape.plus(&message_calibration_shape(msg, &policy));
    }
    shape.image_tokens = request_image_tokens(&req.messages);
    for schema in &req.tools {
        shape.request_chars += 8;
        shape.add_text(&schema.name);
        shape.add_text(&schema.description);
        shape.add_text(&schema.parameters);
    }
    shape
}

/// The last request's shape, cached for calibration reads (Go:
/// `activeReqShape`).
#[derive(Debug, Default)]
pub struct OutputEstimateState {
    /// The latest provider telemetry for per-turn readouts (Go:
    /// `lastUsage`).
    pub last_usage: Option<Usage>,
    pub active_shape: Option<RequestCalibrationShape>,
    /// The learned tokenizer ratio (Go: `promptCalibration`).
    pub prompt_calibration: Option<PromptTokenCalibration>,
    /// The anchor tying one provider count to the view that request carried
    /// (Go: `usageAnchor`).
    pub usage_anchor: Option<super::usage::UsageAnchor>,
    /// The gauge's memoised prompt size (Go: `contextUsage`).
    pub context_usage: Option<super::usage::ContextUsageMemo>,
    /// The schemas the last request carried (Go: `sess.lastProviderSchemas`).
    pub last_provider_schemas: Vec<reasonix_provider::ToolSchema>,
    /// The provider's declared default output budget (Go:
    /// `sess.output.outputBudget`).
    pub output_budget: i64,
}

/// The learned tokenizer ratio (Go: `promptTokenCalibration`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PromptTokenCalibration {
    pub prompt_tokens: i64,
    pub request_chars: i64,
    pub compact_chars: i64,
    pub cjk_runes: i64,
    pub cjk_bytes: i64,
}

impl OutputEstimateState {
    pub(crate) fn lock(
        m: &Mutex<OutputEstimateState>,
    ) -> std::sync::MutexGuard<'_, OutputEstimateState> {
        m.lock().expect("window lock")
    }
}

/// Records the learned ratio. A request that carried images was billed for
/// pixels at a rate the host never sees, and a ratio learned from it would
/// price every later character as part of a picture (Go:
/// `setPromptTokenCalibration`).
pub fn set_prompt_token_calibration(
    window: &super::Window,
    prompt_tokens: i64,
    shape: &RequestCalibrationShape,
) {
    if prompt_tokens <= 0 || shape.request_chars <= 0 || shape.image_tokens > 0 {
        return;
    }
    let mut out = OutputEstimateState::lock(window.output_state());
    out.prompt_calibration = Some(PromptTokenCalibration {
        prompt_tokens,
        request_chars: shape.request_chars,
        compact_chars: shape.compact_chars,
        cjk_runes: shape.cjk_runes,
        cjk_bytes: shape.cjk_bytes,
    });
}

/// Calibrates from the request the loop actually sent (Go:
/// `setPromptTokenCalibrationFromActive`).
pub fn set_prompt_token_calibration_from_active(window: &super::Window, prompt_tokens: i64) {
    let shape = {
        let out = OutputEstimateState::lock(window.output_state());
        out.active_shape.clone()
    };
    if let Some(shape) = shape {
        set_prompt_token_calibration(window, prompt_tokens, &shape);
    }
}

/// Trusts provider telemetry only, and only about our own request: a turn
/// whose provider ran its own tools bills pages we never sent, and a ratio
/// learned from it inflates every later estimate (Go:
/// `setPromptTokenCalibrationFromUsage`). Divergence: Go also refuses
/// usage carrying `ServerToolRequests`; that field's adapter is a later
/// provider slice, so the guard reads zero here.
pub fn set_prompt_token_calibration_from_usage(window: &super::Window, usage: &Usage) {
    if usage.estimated {
        return;
    }
    set_prompt_token_calibration_from_active(window, usage.prompt_tokens.max(0));
}

/// The calibrated token count for one shape, or None where no trusted ratio
/// exists (Go: `calibratedPromptTokens`).
fn calibrated_prompt_tokens(
    calibration: Option<&PromptTokenCalibration>,
    shape: &RequestCalibrationShape,
) -> Option<i64> {
    if shape.request_chars <= 0 {
        return None;
    }
    let cal = calibration?;
    if cal.request_chars <= 0 {
        return None;
    }
    let ratio = cal.prompt_tokens as f64 / cal.request_chars as f64;
    if !(ratio > 0.05 && ratio < 2.0) {
        return None;
    }
    let mut trusted_chars = shape.request_chars;
    let mut excess_cjk_bytes = 0i64;
    // A higher CJK share cannot safely reuse the aggregate ratio. Scale its
    // represented share and price only the excess at the cold rate,
    // preserving exact calibration for stable CJK sessions.
    if shape.cjk_runes * cal.request_chars > cal.cjk_runes * shape.request_chars {
        let scaled = cal.cjk_bytes * shape.request_chars / cal.request_chars;
        let trusted_cjk_bytes = scaled.min(shape.cjk_bytes);
        excess_cjk_bytes = shape.cjk_bytes - trusted_cjk_bytes;
        trusted_chars -= excess_cjk_bytes;
    }
    let cold = (excess_cjk_bytes as f64 * FALLBACK_TOK_PER_CHAR).ceil();
    Some((trusted_chars as f64 * ratio).ceil() as i64 + cold as i64)
}

/// Sizes one shape in real tokens against a calibration (Go:
/// `estimatedShapeTokens`). Pure: the window method reads the learned ratio
/// and calls this.
pub fn estimated_shape_tokens_with(
    calibration: Option<&PromptTokenCalibration>,
    shape: &RequestCalibrationShape,
) -> i64 {
    if shape.request_chars <= 0 {
        return 0;
    }
    if let Some(calibrated) = calibrated_prompt_tokens(calibration, shape) {
        return calibrated + shape.image_tokens;
    }
    (shape.request_chars as f64 * FALLBACK_TOK_PER_CHAR) as i64 + shape.image_tokens
}

/// Sizes the provider-visible messages in real tokens (Go:
/// `estimatedPromptTokens`).
pub fn estimated_prompt_tokens(window: &super::Window, msgs: &[Message]) -> i64 {
    let req = ChatCompletionRequest {
        messages: msgs.to_vec(),
        ..ChatCompletionRequest::default()
    };
    estimated_request_tokens(window, &req)
}

/// Sizes one whole request (Go: `estimatedRequestTokens`).
pub fn estimated_request_tokens(window: &super::Window, req: &ChatCompletionRequest) -> i64 {
    let out = OutputEstimateState::lock(window.output_state());
    let shape = request_calibration_shape(req, window.shared_window());
    estimated_shape_tokens_with(out.prompt_calibration.as_ref(), &shape)
}

/// Sizes one shape with the window's learned ratio (Go:
/// `estimatedShapeTokens` as a window read).
pub fn estimated_shape_tokens(window: &super::Window, shape: &RequestCalibrationShape) -> i64 {
    let out = OutputEstimateState::lock(window.output_state());
    estimated_shape_tokens_with(out.prompt_calibration.as_ref(), shape)
}

/// The per-message flat estimate, sized from the learned compact ratio when
/// one exists (Go: `tokPerChar` + `textTokens`).
pub fn text_tokens(window: &super::Window, s: &str) -> i64 {
    let out = OutputEstimateState::lock(window.output_state());
    if let Some(cal) = &out.prompt_calibration
        && cal.compact_chars > 0
    {
        let r = cal.prompt_tokens as f64 / cal.compact_chars as f64;
        if r > 0.05 && r < 2.0 {
            return (s.len() as f64 * r) as i64;
        }
    }
    (s.len() as f64 * FALLBACK_TOK_PER_CHAR) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::Window;
    use crate::window::testing::*;

    fn shape_of(window: &Window, msgs: &[Message]) -> RequestCalibrationShape {
        let req = ChatCompletionRequest {
            messages: msgs.to_vec(),
            ..ChatCompletionRequest::default()
        };
        request_calibration_shape(&req, window.shared_window())
    }

    fn call_1() -> reasonix_contract::provider::ToolCall {
        reasonix_contract::provider::ToolCall {
            id: "call_1".to_owned(),
            name: "bash".to_owned(),
            arguments: "{}".to_owned(),
            ..reasonix_contract::provider::ToolCall::default()
        }
    }

    // Before any usage calibrates the session, the estimate compared against
    // the context window must still be tokens. It used to be characters,
    // which reads 3-4x high and compacted long before the configured ratio
    // (Go: TestEstimatedPromptTokensStayInTokenUnitBeforeCalibration).
    #[test]
    fn cold_estimates_stay_in_token_unit() {
        let window = window(1_000_000, 0.0);
        let cases = [
            ("chinese", "上下文压缩策略".repeat(8_000), 33_600, 50_000),
            (
                "english",
                "compact the context window ".repeat(8_000),
                54_000,
                70_000,
            ),
        ];
        for (name, text, realish, upper) in cases {
            let got = estimated_prompt_tokens(&window, &[user(&text)]);
            assert!(
                got >= realish / 2 && got <= upper,
                "{name}: cold estimate = {got} tokens, want between {} and {upper} (real ~{realish})",
                realish / 2
            );
        }
    }

    // Desktop rebinds sessions constantly — tab switches, forks, and the
    // snapshot conflict path that adopts the newer disk transcript. Each
    // rebind used to drop the calibration and put the next turn back on the
    // cold estimate (Go: TestSessionSwapKeepsPromptCalibration).
    #[test]
    fn a_conversation_rebind_keeps_the_prompt_calibration() {
        let window = window(200_000, 0.0);
        let msgs = vec![user(&"字".repeat(60_000))];
        let shape = shape_of(&window, &msgs);
        set_prompt_token_calibration(&window, 36_000, &shape);

        let before = estimated_prompt_tokens(&window, &msgs);
        window.reset();
        let after = estimated_prompt_tokens(&window, &msgs);

        assert_eq!(before, after, "estimate moved across a session swap");
        assert!(
            after <= 40_000,
            "estimate = {after}, want the calibrated ~36000 rather than a cold fallback"
        );
    }

    // Ordinary reasoning is not replayed by every provider, so a ratio
    // learned over it must not price a replayed tool-call turn's reasoning
    // as if the denominator had carried it (Go:
    // TestCalibrationIgnoresNonReplayableOrdinaryReasoning).
    #[test]
    fn calibration_ignores_non_replayable_ordinary_reasoning() {
        let window = window(0, 0.0);
        let previous = [user(&"x".repeat(300_000)), assistant("")];
        let mut hidden = previous[1].clone();
        hidden.reasoning_content = "hidden".repeat(150_000);
        let previous = vec![previous[0].clone(), hidden];
        set_prompt_token_calibration(&window, 75_000, &shape_of(&window, &previous));
        let current = vec![
            previous[0].clone(),
            previous[1].clone(),
            Message {
                role: Role::ASSISTANT,
                reasoning_content: "r".repeat(400_000),
                tool_calls: vec![call_1()],
                ..Message::default()
            },
        ];
        let got = estimated_prompt_tokens(&window, &current);
        assert!(
            got >= 160_000,
            "replayable reasoning estimate = {got}, want ordinary local reasoning excluded from \
             the calibration denominator"
        );
    }

    #[test]
    fn calibrated_responses_budget_includes_new_ordinary_reasoning() {
        let mut config = config(200_000, 0.0);
        config.shared_window = crate::window::SharedWindowPolicy {
            shares_context_window: true,
            output_budget: 128 * 1024,
            replays_ordinary_reasoning: true,
            replays_responses_items: false,
        };
        let window = window_over(config, vec![user(&"x".repeat(300_000))]);
        let shape = shape_of(&window, &[user(&"x".repeat(300_000))]);
        set_prompt_token_calibration(&window, 75_000, &shape);
        let current = ChatCompletionRequest {
            messages: vec![
                user(&"x".repeat(300_000)),
                Message {
                    role: Role::ASSISTANT,
                    reasoning_content: "r".repeat(400_000),
                    ..Message::default()
                },
            ],
            ..ChatCompletionRequest::default()
        };
        let got = estimated_request_tokens(&window, &current);
        assert!(
            got >= 174_000,
            "Responses ordinary reasoning estimate = {got}, want newly replayed reasoning included"
        );
        let budget = window
            .effective_output_budget(&current)
            .expect("budget resolves");
        assert!(
            matches!(budget, Some(b) if b <= 20_000),
            "replayed reasoning budget = {budget:?}, want a clipped budget <= 20000"
        );
    }

    #[test]
    fn calibrated_responses_budget_includes_new_replay_items() {
        let mut config = config(200_000, 0.0);
        config.shared_window = crate::window::SharedWindowPolicy {
            shares_context_window: true,
            output_budget: 128 * 1024,
            replays_ordinary_reasoning: false,
            replays_responses_items: true,
        };
        let window = window_over(config, vec![user(&"x".repeat(300_000))]);
        let shape = shape_of(&window, &[user(&"x".repeat(300_000))]);
        set_prompt_token_calibration(&window, 75_000, &shape);
        let item = format!(
            r#"{{"id":"ws_1","type":"web_search_call","status":"completed","action":{{"query":"{}"}}}}"#,
            "q".repeat(400_000)
        );
        let raw = serde_json::value::RawValue::from_string(item).expect("raw item");
        let current = ChatCompletionRequest {
            messages: vec![
                user(&"x".repeat(300_000)),
                Message {
                    role: Role::ASSISTANT,
                    responses_items: vec![raw],
                    ..Message::default()
                },
            ],
            ..ChatCompletionRequest::default()
        };
        let got = estimated_request_tokens(&window, &current);
        assert!(
            got >= 174_000,
            "Responses replay-item estimate = {got}, want newly replayed item included"
        );
        let budget = window
            .effective_output_budget(&current)
            .expect("budget resolves");
        assert!(
            matches!(budget, Some(b) if b < 128 * 1024),
            "Responses replay-item budget = {budget:?}, want a clipped budget"
        );
    }

    #[test]
    fn calibrated_output_budget_counts_tool_schemas_once() {
        let window = window(0, 0.0);
        let req = ChatCompletionRequest {
            messages: vec![user(&"x".repeat(100_000))],
            tools: vec![reasonix_provider::ToolSchema {
                name: "lookup".to_owned(),
                description: "y".repeat(100_000),
                parameters: r#"{"type":"object"}"#.to_owned(),
            }],
            ..ChatCompletionRequest::default()
        };
        let shape = request_calibration_shape(&req, window.shared_window());
        set_prompt_token_calibration(&window, 60_000, &shape);
        let got = estimated_request_tokens(&window, &req);
        assert_eq!(
            got, 60_000,
            "calibrated request tokens must count the tool schema once"
        );
    }
}
