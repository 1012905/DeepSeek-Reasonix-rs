//! TOOL-015 paid-search / paid-fetch approval seam (Go
//! `internal/contract/tool/paidsearch.go`, docs/contracts/tools.md §1.20):
//! the request shape, the search/fetch split, the approver port, and the
//! fail-closed binding. The fresh decision, session grant, persist-to-user-
//! config flow, whitelist, and price book live in the controller/boot slices.

#![forbid(unsafe_code)]

use std::sync::Arc;

use reasonix_tools::abi::CallContext;
use reasonix_tools::builtin::{
    PAID_SEARCH_APPROVAL_TOOL, PAID_SERVICE_FETCH, PAID_SERVICE_SEARCH, PaidSearchApprover,
    PaidSearchRequest, paid_search_approver_from,
};

struct RecordingApprover {
    seen: std::sync::Mutex<Vec<PaidSearchRequest>>,
    allow: bool,
}

impl PaidSearchApprover for RecordingApprover {
    fn approve_paid_search(
        &self,
        req: &PaidSearchRequest,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        self.seen.lock().expect("seen lock").push(req.clone());
        Ok(self.allow)
    }
}

fn bound(approver: Arc<RecordingApprover>) -> CallContext {
    let port: Arc<dyn PaidSearchApprover> = approver;
    CallContext::new().with_value(Arc::new(port))
}

#[test]
fn search_keeps_the_zero_service_and_fetch_is_explicitly_booked() {
    let search = PaidSearchRequest::search("deepseek_api", "golang cancel", "$0.75 / 1k");
    let fetch = PaidSearchRequest::fetch("deepseek_api", "https://example.com/page", "$0.75 / 1k");
    assert_eq!(
        search.service, "",
        "source search chain leaves Service empty"
    );
    assert!(!search.is_fetch());
    assert_eq!(fetch.service, PAID_SERVICE_FETCH);
    assert!(fetch.is_fetch());
    assert_ne!(
        search.service, fetch.service,
        "agreeing to one is not agreeing to the other"
    );
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
fn the_approver_binding_routes_the_request_and_a_decline_returns_false() {
    let approver = Arc::new(RecordingApprover {
        seen: std::sync::Mutex::new(Vec::new()),
        allow: true,
    });
    let ctx = bound(approver.clone());
    let port = paid_search_approver_from(&ctx).expect("approver installed");
    let req = PaidSearchRequest::search("deepseek_api", "q", "price");
    assert!(port.approve_paid_search(&req).expect("no transport error"));
    let seen = approver.seen.lock().expect("seen lock").clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].backend, "deepseek_api");
    assert_eq!(seen[0].query, "q");

    let declined = Arc::new(RecordingApprover {
        seen: std::sync::Mutex::new(Vec::new()),
        allow: false,
    });
    let port: Arc<dyn PaidSearchApprover> = declined;
    let fetch = PaidSearchRequest::fetch("b", "https://example.com", "price");
    assert!(
        !port.approve_paid_search(&fetch).expect("decision lands"),
        "a decline answers false, not an error"
    );
}

#[test]
fn a_headless_context_has_no_approver_and_fails_closed() {
    assert!(
        paid_search_approver_from(&CallContext::new()).is_none(),
        "nil approver: the consenting chain treats this as not allowed"
    );
}

#[test]
fn the_approval_tool_id_is_paid_search() {
    assert_eq!(PAID_SEARCH_APPROVAL_TOOL, "paid_search");
}
