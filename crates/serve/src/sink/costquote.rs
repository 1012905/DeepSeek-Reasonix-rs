//! Fills occurrence-time cost quotes before events reach a frontend
//! (docs/contracts/sse.md §5; Go `costquote_sink.go` + `usage_quote.go`).
//! Frontends consume the quote a usage event carries and never reprice.
//!
//! The Go sink builds its quote through `pricing.BuildQuote`, whose engine
//! (fixed-point money, the official vendor rate tables, peak windows) has not
//! migrated yet. Until it does, [`ensure_cost_quote`] answers the no-price
//! fallback for every usage event without an attached quote — estimated,
//! incomplete, display unavailable — and never invents a number.

use std::sync::{Arc, Mutex};

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::pricing::CostQuote;

use super::Sink;
use super::capabilities::AuditForwarder;

/// How a caller selected the requested presentation currency (Go
/// `pricing.DisplayRequest`). The source is runtime context only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayRequest {
    pub currency: String,
    pub source: String,
}

pub const DISPLAY_SOURCE_EXPLICIT: &str = "explicit";
pub const DISPLAY_SOURCE_AUTO: &str = "auto";

/// Supplies the explicit display request for the quoting middleware. Empty
/// means auto and keeps the price-book currency (Go `QuoteContext`).
#[derive(Default)]
pub struct QuoteContext {
    display: Mutex<DisplayState>,
}

#[derive(Default)]
struct DisplayState {
    currency: String,
    request: Option<DisplayRequest>,
}

impl QuoteContext {
    /// Updates the resolved display currency used for the selected amount.
    pub fn set_display(&self, currency: &str) {
        let currency = normalize_currency(currency);
        let mut state = self.display.lock().unwrap_or_else(|e| e.into_inner());
        state.currency = currency.clone();
        state.request = Some(DisplayRequest {
            currency,
            source: DISPLAY_SOURCE_EXPLICIT.to_owned(),
        });
    }

    pub fn set_display_request(&self, request: DisplayRequest) {
        let currency = normalize_currency(&request.currency);
        let source = if request.source.is_empty() {
            DISPLAY_SOURCE_AUTO
        } else {
            request.source.as_str()
        };
        let mut state = self.display.lock().unwrap_or_else(|e| e.into_inner());
        state.request = Some(DisplayRequest {
            currency: currency.clone(),
            source: source.to_owned(),
        });
        state.currency = currency;
    }

    /// The display request in force, defaulted to auto (Go `snapshot`). The
    /// quote engine consumes it when it migrates; the display-currency notice
    /// path only writes.
    pub fn snapshot(&self) -> DisplayRequest {
        let state = self.display.lock().unwrap_or_else(|e| e.into_inner());
        let mut request = state.request.clone().unwrap_or_default();
        if request.currency.is_empty() && !state.currency.is_empty() {
            request.currency = state.currency.clone();
        }
        if request.source.is_empty() {
            request.source = if request.currency.is_empty() {
                DISPLAY_SOURCE_AUTO.to_owned()
            } else {
                DISPLAY_SOURCE_EXPLICIT.to_owned()
            };
        }
        request
    }
}

/// Maps symbols and aliases to ISO-4217 codes when known; unknown
/// three-letter codes pass through uppercased, empty stays empty. Mirrors Go
/// `pricing.NormalizeCurrency` until the pricing wave lands it in
/// reasonix-contract.
pub fn normalize_currency(currency: &str) -> String {
    let value = currency.trim();
    if value.is_empty() {
        return String::new();
    }
    match value.to_ascii_uppercase().as_str() {
        "CNY" | "RMB" | "CNH" | "YUAN" | "RENMINBI" => return "CNY".to_owned(),
        "USD" | "US$" | "DOLLAR" | "DOLLARS" => return "USD".to_owned(),
        "EUR" | "EURO" | "EUROS" => return "EUR".to_owned(),
        "GBP" | "POUND" | "POUNDS" | "STERLING" => return "GBP".to_owned(),
        "JPY" | "YEN" => return "JPY".to_owned(),
        _ => {}
    }
    match value {
        "¥" | "￥" => return "CNY".to_owned(),
        "$" => return "USD".to_owned(),
        "€" => return "EUR".to_owned(),
        "£" => return "GBP".to_owned(),
        _ => {}
    }
    if value.len() == 3 && value.chars().all(|c| c.is_ascii_alphabetic()) {
        return value.to_ascii_uppercase();
    }
    value.to_owned()
}

/// Wraps `inner` with quoting (Go `NewCostQuoteSink`). A `None` context still
/// quotes: the fallback keeps the no-price shape.
pub fn cost_quote_sink(inner: Arc<dyn Sink>, ctx: Option<Arc<QuoteContext>>) -> Arc<CostQuoteSink> {
    Arc::new(CostQuoteSink {
        forwarder: AuditForwarder::new(inner.clone()),
        inner,
        ctx: ctx.unwrap_or_default(),
    })
}

/// Fills cost quotes on usage and guardian-assessment events before
/// forwarding. A nil sink still forwards nothing; the quote rides inside the
/// event's usage payload.
pub struct CostQuoteSink {
    forwarder: AuditForwarder,
    inner: Arc<dyn Sink>,
    ctx: Arc<QuoteContext>,
}

impl Sink for CostQuoteSink {
    fn emit(&self, mut e: Event) {
        if e.kind == "notice" && e.code == "display_currency" {
            self.ctx.set_display(&e.detail);
        }
        if e.kind == "usage" && e.usage.as_ref().is_some_and(|u| u.cost_quote.is_none()) {
            let quote = ensure_cost_quote(&e, Some(&self.ctx));
            if let Some(u) = e.usage.as_mut() {
                u.cost_quote = quote;
            }
        }
        if e.kind == "guardian_assessment" {
            let needs = e
                .guardian
                .as_ref()
                .and_then(|g| g.usage.as_ref())
                .is_some_and(|u| u.cost_quote.is_none());
            if needs {
                let quote = ensure_cost_quote(&guardian_usage_event(&e), Some(&self.ctx));
                if let Some(u) = e.guardian.as_mut().and_then(|g| g.usage.as_mut()) {
                    u.cost_quote = quote;
                }
            }
        }
        self.inner.emit(e);
    }

    fn record_turn_completion(&self) {
        self.forwarder.record_turn_completion();
    }

    fn record_memory_recall(&self, a: super::capabilities::MemoryRecallAudit) {
        self.forwarder.record_memory_recall(a);
    }

    fn record_delegation_audit(&self, a: super::capabilities::DelegationAudit) {
        self.forwarder.record_delegation_audit(a);
    }

    fn record_workspace_mutation(&self, m: super::capabilities::WorkspaceMutation) {
        self.forwarder.record_workspace_mutation(m);
    }

    fn record_run_budget(&self, s: super::capabilities::RunBudgetSample) {
        self.forwarder.record_run_budget(s);
    }

    fn quote_usage(&self, e: &Event) -> Option<CostQuote> {
        if let Some(existing) = e.usage.as_ref().and_then(|u| u.cost_quote.clone()) {
            return Some(existing);
        }
        ensure_cost_quote(e, Some(&self.ctx))
    }

    fn capture_usage_scope(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.forwarder.capture_usage_scope()
    }
}

/// Spells a review's usage as the usage event it is, so the review and the
/// turn's own calls are priced by one code path (Go `guardianUsageEvent`).
fn guardian_usage_event(e: &Event) -> Event {
    Event {
        kind: "usage".to_owned(),
        model_ref: e.model_ref.clone(),
        source: e.source.clone(),
        usage: e.guardian.as_ref().and_then(|g| g.usage.clone()),
        ..Event::default()
    }
}

/// Builds a cost quote for an event when missing (Go `EnsureCostQuote`). The
/// no-price fallback is the only path until the pricing engine migrates: no
/// rate card means estimated and incomplete, never a guessed amount. The
/// context rides along for signature parity — the Go no-price branch ignores
/// its snapshot too.
pub fn ensure_cost_quote(e: &Event, _ctx: Option<&QuoteContext>) -> Option<CostQuote> {
    let usage = e.usage.as_ref()?;
    Some(CostQuote {
        estimated: true,
        cost_complete: false,
        display_complete: false,
        complete: false,
        coverage: "incomplete".to_owned(),
        display_status: "unavailable".to_owned(),
        incomplete_reason: "no_price".to_owned(),
        model_ref: e.model_ref.clone(),
        usage_source: usage.source.clone(),
        valuations: Default::default(),
        ..CostQuote::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::FuncSink;
    use reasonix_contract::eventwire::guardian::Guardian;
    use reasonix_contract::eventwire::usage::Usage;
    use std::sync::Mutex as StdMutex;

    fn usage_event() -> Event {
        Event {
            kind: "usage".to_owned(),
            model_ref: "deepseek/deepseek-v4-flash".to_owned(),
            usage: Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 100,
                ..Usage::default()
            }),
            ..Event::default()
        }
    }

    // Ported: costquote_sink_test.go
    // TestEnsureCostQuoteDoesNotUseRuntimeFX, held at the no-price fallback
    // until the pricing engine migrates: the quote is estimated, incomplete,
    // carries no valuation of any basis — so nothing can be an FX basis — and
    // never claims completeness.
    #[test]
    fn quote_without_a_rate_card_never_invents_one() {
        let e = usage_event();
        let ctx = QuoteContext::default();
        ctx.set_display("USD");
        let q = ensure_cost_quote(&e, Some(&ctx)).expect("quote");
        assert!(q.estimated && !q.cost_complete && !q.complete);
        assert_eq!(q.display_status, "unavailable");
        assert_eq!(q.incomplete_reason, "no_price");
        assert!(q.valuations.is_empty(), "no valuation of any basis");
    }

    // The display_currency notice rebinds the context's display request
    // (broadcaster.go:145-147 consumes the same fact).
    #[test]
    fn display_currency_notice_rebinds_the_display_request() {
        let ctx = Arc::new(QuoteContext::default());
        ctx.set_display("usd");
        let request = ctx.snapshot();
        assert_eq!(request.currency, "USD");
        assert_eq!(request.source, DISPLAY_SOURCE_EXPLICIT);
        ctx.set_display("￥");
        assert_eq!(ctx.snapshot().currency, "CNY");
        ctx.set_display_request(DisplayRequest {
            currency: " eur ".to_owned(),
            source: String::new(),
        });
        let request = ctx.snapshot();
        assert_eq!(request.currency, "EUR");
        assert_eq!(request.source, DISPLAY_SOURCE_AUTO);
    }

    // normalize_currency mirrors Go pricing.NormalizeCurrency.
    #[test]
    fn normalize_currency_matches_the_go_table() {
        assert_eq!(normalize_currency("CNY"), "CNY");
        assert_eq!(normalize_currency(" rmb "), "CNY");
        assert_eq!(normalize_currency("yuan"), "CNY");
        assert_eq!(normalize_currency("US$"), "USD");
        assert_eq!(normalize_currency("dollar"), "USD");
        assert_eq!(normalize_currency("¥"), "CNY");
        assert_eq!(normalize_currency("￥"), "CNY");
        assert_eq!(normalize_currency("$"), "USD");
        assert_eq!(normalize_currency("€"), "EUR");
        assert_eq!(normalize_currency("£"), "GBP");
        assert_eq!(normalize_currency("gbp"), "GBP");
        assert_eq!(normalize_currency("yen"), "JPY");
        assert_eq!(normalize_currency("aud"), "AUD");
        assert_eq!(normalize_currency("AUD"), "AUD");
        assert_eq!(normalize_currency(""), "");
        assert_eq!(normalize_currency("nope"), "nope");
        assert_eq!(normalize_currency("toolong"), "toolong");
    }

    // Ported: the Emit seam — usage events without a quote leave with the
    // no-price quote; a quote already attached passes through untouched.
    #[test]
    fn emit_fills_missing_quotes_and_keeps_attached_ones() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let seen = got.clone();
        let sink = cost_quote_sink(
            Arc::new(FuncSink::new(move |e| {
                seen.lock().unwrap_or_else(|e| e.into_inner()).push(e);
            })),
            None,
        );
        sink.emit(usage_event());
        sink.emit(Event {
            kind: "usage".to_owned(),
            usage: Some(Usage {
                prompt_tokens: 5,
                cost_quote: Some(CostQuote {
                    complete: true,
                    ..CostQuote::default()
                }),
                ..Usage::default()
            }),
            ..Event::default()
        });
        let got = got.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(got.len(), 2);
        let first = got[0].usage.as_ref().unwrap().cost_quote.as_ref().unwrap();
        assert_eq!(first.incomplete_reason, "no_price");
        let second = got[1].usage.as_ref().unwrap().cost_quote.as_ref().unwrap();
        assert!(second.complete, "an attached quote must not be repriced");
    }

    // Ported: the guardian spelling — a review's usage is priced by the same
    // code path as the turn's own calls.
    #[test]
    fn guardian_usage_is_priced_by_the_same_path() {
        let got: Arc<StdMutex<Vec<Event>>> = Arc::default();
        let seen = got.clone();
        let sink = cost_quote_sink(
            Arc::new(FuncSink::new(move |e| {
                seen.lock().unwrap_or_else(|e| e.into_inner()).push(e);
            })),
            None,
        );
        sink.emit(Event {
            kind: "guardian_assessment".to_owned(),
            model_ref: "deepseek/deepseek-v4-flash".to_owned(),
            guardian: Some(Guardian {
                usage: Some(Usage {
                    prompt_tokens: 42,
                    ..Usage::default()
                }),
                ..Guardian::default()
            }),
            ..Event::default()
        });
        let got = got.lock().unwrap_or_else(|e| e.into_inner());
        let quote = got[0]
            .guardian
            .as_ref()
            .unwrap()
            .usage
            .as_ref()
            .unwrap()
            .cost_quote
            .as_ref()
            .expect("guardian usage must carry a quote");
        assert_eq!(quote.incomplete_reason, "no_price");
    }

    // The no-price quote carries the usage's raw source (Go sets
    // `UsageSource: e.UsageSource` before the BuildQuote path applies its
    // first-non-empty fallback).
    #[test]
    fn no_price_quote_carries_the_raw_usage_source() {
        let mut e = usage_event();
        if let Some(u) = e.usage.as_mut() {
            u.source = "subagent".to_owned();
        }
        let q = ensure_cost_quote(&e, None).expect("quote");
        assert_eq!(q.usage_source, "subagent");
        let bare = Event {
            kind: "usage".to_owned(),
            usage: Some(Usage::default()),
            ..Event::default()
        };
        assert_eq!(ensure_cost_quote(&bare, None).unwrap().usage_source, "");
    }

    // A quote_usage query through the sink preserves an attached quote.
    #[test]
    fn quote_usage_preserves_an_attached_quote() {
        let attached = CostQuote {
            complete: true,
            ..CostQuote::default()
        };
        let e = Event {
            kind: "usage".to_owned(),
            usage: Some(Usage {
                cost_quote: Some(attached.clone()),
                ..Usage::default()
            }),
            ..Event::default()
        };
        let sink = cost_quote_sink(Arc::new(DiscardShim), None);
        assert_eq!(
            crate::sink::capabilities::quote_usage(sink.as_ref(), &e).unwrap(),
            attached
        );
    }

    /// A sink with no quoter of its own, for the fallback probe.
    struct DiscardShim;
    impl Sink for DiscardShim {
        fn emit(&self, _: Event) {}
    }

    // A usage event with no usage payload quotes nothing.
    #[test]
    fn no_usage_quotes_nothing() {
        let e = Event {
            kind: "usage".to_owned(),
            ..Event::default()
        };
        assert!(ensure_cost_quote(&e, None).is_none());
    }
}
