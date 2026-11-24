//! ACP-018: reconnect / disconnect / submitForm result shapes, state machine,
//! validation boundary, and the injectable controller seam.

use std::cell::RefCell;
use std::collections::BTreeMap;

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::mcp_control::{
    MCP_APPROVAL_OWED, McpControl, ReasonixExtensionFormResult, ReasonixMcpDisconnect,
    ReasonixMcpReconnect, ReconnectOutcome, ReconnectState, SessionExtensionFormParams,
    control_failure, name_required, unknown_session,
};
use serde_json::Value;

type SubmittedRecord = (String, String, BTreeMap<String, Value>);

struct DoubleControl {
    outcome: ReconnectOutcome,
    disconnect_live: bool,
    submitted: RefCell<Vec<SubmittedRecord>>,
}

impl McpControl for DoubleControl {
    fn reconnect(&self, _name: &str) -> ReconnectOutcome {
        self.outcome.clone()
    }

    fn disconnect(&self, _name: &str) -> bool {
        self.disconnect_live
    }

    fn submit_form(
        &self,
        plugin_id: &str,
        surface_id: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<(), String> {
        self.submitted.borrow_mut().push((
            plugin_id.to_owned(),
            surface_id.to_owned(),
            values.clone(),
        ));
        Ok(())
    }
}

#[test]
fn reconnect_ready_result_carries_tool_count() {
    let result =
        ReasonixMcpReconnect::from_outcome("s1", "fs", ReconnectOutcome::Ready { tools: 4 });
    assert_eq!(result.state, ReconnectState::Ready.wire());
    assert_eq!(result.tools, Some(4));
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.starts_with(
        "{\"schemaVersion\":1,\"sessionId\":\"s1\",\"name\":\"fs\",\"state\":\"ready\",\"tools\":4}"
    ));
}

#[test]
fn reconnect_approval_owed_echoes_code_and_launch() {
    let result = ReasonixMcpReconnect::from_outcome(
        "s1",
        "fs",
        ReconnectOutcome::ApprovalOwed {
            launch: "npx fs".to_owned(),
        },
    );
    assert_eq!(result.state, "approval_owed");
    assert_eq!(result.code.as_deref(), Some(MCP_APPROVAL_OWED));
    assert_eq!(result.launch.as_deref(), Some("npx fs"));
    assert!(result.error.is_none());
}

#[test]
fn reconnect_failed_result_carries_error() {
    let result = ReasonixMcpReconnect::from_outcome(
        "s1",
        "fs",
        ReconnectOutcome::Failed {
            error: "boom".to_owned(),
        },
    );
    assert_eq!(result.state, "failed");
    assert_eq!(result.error.as_deref(), Some("boom"));
}

#[test]
fn disconnect_result_serializes_disconnected_flag() {
    let result = ReasonixMcpDisconnect {
        schema_version: 1,
        session_id: "s1".to_owned(),
        name: "fs".to_owned(),
        disconnected: true,
    };
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        "{\"schemaVersion\":1,\"sessionId\":\"s1\",\"name\":\"fs\",\"disconnected\":true}"
    );
}

#[test]
fn submit_form_records_through_control_seam() {
    let double = DoubleControl {
        outcome: ReconnectOutcome::Ready { tools: 0 },
        disconnect_live: true,
        submitted: RefCell::new(Vec::new()),
    };
    let params = SessionExtensionFormParams {
        session_id: "s1".to_owned(),
        plugin_id: "p".to_owned(),
        surface_id: "s".to_owned(),
        values: BTreeMap::from([("choice".to_owned(), Value::String("a".to_owned()))]),
    };
    double
        .submit_form(&params.plugin_id, &params.surface_id, &params.values)
        .unwrap();
    let recorded = double.submitted.borrow();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "p");
    assert_eq!(recorded[0].1, "s");

    let result = ReasonixExtensionFormResult {
        schema_version: 1,
        session_id: "s1".to_owned(),
        accepted: true,
    };
    assert!(result.accepted);
}

#[test]
fn validation_boundary_errors_are_typed_and_verbatim() {
    let blank = name_required("_reasonix.io/session/mcp/reconnect");
    assert_eq!(blank.code, RpcErrorCode::InvalidParams);
    assert_eq!(
        blank.message,
        "_reasonix.io/session/mcp/reconnect: name is required"
    );

    let unknown = unknown_session("_reasonix.io/session/mcpServers", "s9");
    assert_eq!(unknown.code, RpcErrorCode::InvalidParams);
    assert_eq!(
        unknown.message,
        "_reasonix.io/session/mcpServers: unknown session s9"
    );

    let failure = control_failure("_reasonix.io/session/extension/submitForm", "no hub");
    assert_eq!(failure.code, RpcErrorCode::Internal);
    assert_eq!(failure.code.code(), -32603);
    assert_eq!(
        failure.message,
        "_reasonix.io/session/extension/submitForm: no hub"
    );
}
