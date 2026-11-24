//! Paid-search / paid-fetch approval seam (tools.md §1.20; Go
//! `internal/contract/tool/paidsearch.go`).
//!
//! A web_search or web_fetch call is about to use a backend the vendor
//! charges for. Only a person agrees to be charged: no approval mode
//! (YOLO included) answers this, and a nil approver fails closed.
//!
//! What lives elsewhere, not in this seam: the free-backend fallback order,
//! the config whitelist, the session grant, the persisted "always allow"
//! write-back, the price book, and the billing record (controller / boot
//! chain / pricing slices). This crate owns the request shape, the service
//! split, and the per-call context binding.

#![forbid(unsafe_code)]

use std::sync::Arc;

use crate::abi::CallContext;

/// Approval decision tool id (Go `permission.PaidSearch`,
/// `controller.PaidSearchApprovalTool`): the tool+subject namespace a
/// session grant books under.
pub const PAID_SEARCH_APPROVAL_TOOL: &str = "paid_search";

/// The fetch service spelling, set only by the fetch chain (Go
/// `tool.PaidServiceFetch`).
pub const PAID_SERVICE_FETCH: &str = "fetch";
/// The search service spelling recorded in the contract (Go
/// `tool.PaidServiceSearch`). Note the source deviation: the search chain
/// leaves [`PaidSearchRequest::service`] at its zero value (`""`) rather than
/// filling this constant; an empty service reads as search at the decision
/// site. The constant stays for explicit construction and the fetch/split
/// comparison.
pub const PAID_SERVICE_SEARCH: &str = "search";

/// A web_search / web_fetch call about to use a backend the vendor charges
/// for. [`query`] is what would be sent — a search term or a fetch address —
/// shown to the user so they see what leaves the machine. [`price`] is the
/// per-1000-successful-request quote string; an unpublished price carries
/// the fallback supplied by the chain layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaidSearchRequest {
    /// What is being bought. Empty reads as search; [`PAID_SERVICE_FETCH`]
    /// reads as fetch. Agreeing to one is not agreeing to the other.
    pub service: String,
    pub backend: String,
    pub query: String,
    pub price: String,
}

impl PaidSearchRequest {
    /// Builds a search request. Per the source chain the service stays at its
    /// zero value; the controller classifies an empty service as search.
    pub fn search(
        backend: impl Into<String>,
        query: impl Into<String>,
        price: impl Into<String>,
    ) -> Self {
        Self {
            service: String::new(),
            backend: backend.into(),
            query: query.into(),
            price: price.into(),
        }
    }

    /// Builds a fetch request; its approval is booked apart from search.
    pub fn fetch(
        backend: impl Into<String>,
        query: impl Into<String>,
        price: impl Into<String>,
    ) -> Self {
        Self {
            service: PAID_SERVICE_FETCH.to_owned(),
            backend: backend.into(),
            query: query.into(),
            price: price.into(),
        }
    }

    /// Whether this request buys a fetch rather than a search.
    pub fn is_fetch(&self) -> bool {
        self.service == PAID_SERVICE_FETCH
    }
}

/// Asks the user whether one backend may be charged (Go
/// `tool.PaidSearchApprover`). A fresh human decision: approval modes must
/// not answer it; a missing approver fails closed at the call site.
pub trait PaidSearchApprover: Send + Sync {
    fn approve_paid_search(
        &self,
        req: &PaidSearchRequest,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
}

/// The approver bound to one call, when the host installed one (Go
/// `tool.PaidSearchApproverFrom`). A [`None`] return is the fail-closed
/// condition: the consenting chain must treat it as not allowed.
pub fn paid_search_approver_from(ctx: &CallContext) -> Option<Arc<dyn PaidSearchApprover>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn PaidSearchApprover>>()
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::CallContext;

    struct RecordingApprover {
        decisions: std::sync::Mutex<Vec<PaidSearchRequest>>,
        allow: bool,
    }

    impl PaidSearchApprover for RecordingApprover {
        fn approve_paid_search(
            &self,
            req: &PaidSearchRequest,
        ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
            self.decisions
                .lock()
                .expect("decisions lock")
                .push(req.clone());
            Ok(self.allow)
        }
    }

    #[test]
    fn search_requests_keep_the_zero_service_and_fetch_is_explicit() {
        let search = PaidSearchRequest::search("deepseek_api", "golang cancel", "$0.75 / 1k");
        let fetch =
            PaidSearchRequest::fetch("deepseek_api", "https://example.com/page", "$0.75 / 1k");
        assert_eq!(
            search.service, "",
            "source search chain leaves Service empty"
        );
        assert!(!search.is_fetch());
        assert_eq!(fetch.service, PAID_SERVICE_FETCH);
        assert!(fetch.is_fetch());
        // Same backend, different service: the search grant must not ride fetch.
        assert_ne!(search.service, fetch.service);
        assert_eq!(PAID_SERVICE_SEARCH, "search");
    }

    #[test]
    fn the_request_carries_backend_query_and_price_to_the_card() {
        let req = PaidSearchRequest {
            service: PAID_SERVICE_FETCH.to_owned(),
            backend: "deepseek_api".to_owned(),
            query: "https://example.com/page".to_owned(),
            price: "$0.75 per 1,000 successful requests".to_owned(),
        };
        assert_eq!(req.backend, "deepseek_api");
        assert_eq!(req.query, "https://example.com/page");
        assert_eq!(req.price, "$0.75 per 1,000 successful requests");
    }

    #[test]
    fn the_approver_binding_round_trips_and_a_headless_ctx_fails_closed() {
        let stub = Arc::new(RecordingApprover {
            decisions: std::sync::Mutex::new(Vec::new()),
            allow: true,
        });
        let port: Arc<dyn PaidSearchApprover> = stub.clone();
        let ctx = CallContext::new().with_value(Arc::new(port));
        let bound = paid_search_approver_from(&ctx).expect("approver installed");
        let req = PaidSearchRequest::search("deepseek_api", "q", "price");
        assert!(bound.approve_paid_search(&req).expect("decision"));
        let seen = stub.decisions.lock().expect("decisions lock").clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].backend, "deepseek_api");

        assert!(
            paid_search_approver_from(&CallContext::new()).is_none(),
            "no approver installed means fail closed"
        );
    }

    #[test]
    fn a_declined_decision_returns_false_not_an_error() {
        let stub = Arc::new(RecordingApprover {
            decisions: std::sync::Mutex::new(Vec::new()),
            allow: false,
        });
        let bound: Arc<dyn PaidSearchApprover> = stub;
        let req = PaidSearchRequest::fetch("b", "https://example.com", "price");
        let allow = bound.approve_paid_search(&req).expect("no transport error");
        assert!(!allow);
    }

    #[test]
    fn the_approval_tool_id_is_paid_search() {
        assert_eq!(PAID_SEARCH_APPROVAL_TOOL, "paid_search");
    }
}
