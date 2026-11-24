//! ACP-024: `session/new` orchestration step order, failure cleanup, and the
//! afterResponse hook, exercised through in-memory doubles.

use std::cell::Cell;
use std::rc::Rc;

use reasonix_acp::lifecycle::LeaseRefusal;
use reasonix_acp::session_new::{
    AuthorityBinder, ControllerHandle, EventSink, FLOW_STEPS, FlowDeps, LeaseManager, NewFailure,
    NewSessionRequest, SessionFactory, SessionStore, parse_params, run,
};
use serde_json::json;

struct DoubleStore {
    registered: Vec<String>,
}
impl SessionStore for DoubleStore {
    fn register(&mut self, id: &str) {
        self.registered.push(id.to_owned());
    }
    fn contains(&self, id: &str) -> bool {
        self.registered.iter().any(|r| r == id)
    }
}

struct DoubleLease {
    next_token: usize,
    released: Vec<usize>,
    fail: Option<LeaseRefusal>,
}
impl LeaseManager for DoubleLease {
    fn acquire(&mut self, _transcript: &str) -> Result<usize, LeaseRefusal> {
        if let Some(fail) = &self.fail {
            return Err(match fail {
                LeaseRefusal::Held { holder } => LeaseRefusal::Held {
                    holder: holder.clone(),
                },
                LeaseRefusal::Unprovable { detail } => LeaseRefusal::Unprovable {
                    detail: detail.clone(),
                },
                LeaseRefusal::Other { detail } => LeaseRefusal::Other {
                    detail: detail.clone(),
                },
            });
        }
        self.next_token += 1;
        Ok(self.next_token)
    }
    fn release(&mut self, token: usize) {
        self.released.push(token);
    }
}

struct DoubleController {
    closed: Rc<Cell<bool>>,
}
impl ControllerHandle for DoubleController {
    fn close(&mut self) {
        self.closed.set(true);
    }
    fn plan_mode(&self) -> bool {
        false
    }
    fn session_dir(&self) -> Option<&str> {
        Some("/sessions/s1")
    }
}

struct DoubleFactory {
    fail: Option<NewFailure>,
    closed: Rc<Cell<bool>>,
}
impl SessionFactory for DoubleFactory {
    fn build(&mut self, _req: &NewSessionRequest) -> Result<Box<dyn ControllerHandle>, NewFailure> {
        if let Some(fail) = &self.fail {
            return Err(NewFailure {
                code: fail.code,
                message: fail.message.clone(),
                data_code: fail.data_code,
            });
        }
        Ok(Box::new(DoubleController {
            closed: self.closed.clone(),
        }))
    }
}

struct DoubleAuthority {
    fail: Option<String>,
    bound: bool,
}
impl AuthorityBinder for DoubleAuthority {
    fn bind(&mut self, _controller: &mut dyn ControllerHandle) -> Result<(), String> {
        self.bound = true;
        self.fail.clone().map_or(Ok(()), Err)
    }
}

struct DoubleSink {
    calls: Vec<String>,
}
impl EventSink for DoubleSink {
    fn send_available_commands(&mut self) {
        self.calls.push("available_commands".to_owned());
    }
    fn publish_mcp_status(&mut self) {
        self.calls.push("mcp_status".to_owned());
    }
    fn send_usage_update(&mut self) {
        self.calls.push("usage_update".to_owned());
    }
}

fn happy_deps<'a>(
    store: &'a mut DoubleStore,
    lease: &'a mut DoubleLease,
    factory: &'a mut DoubleFactory,
    authority: &'a mut DoubleAuthority,
    sink: &'a mut DoubleSink,
) -> FlowDeps<'a> {
    FlowDeps {
        store,
        lease,
        factory,
        authority,
        sink,
    }
}

#[test]
fn happy_path_records_sixteen_steps_then_after_response_without_usage() {
    let mut store = DoubleStore {
        registered: Vec::new(),
    };
    let mut lease = DoubleLease {
        next_token: 0,
        released: Vec::new(),
        fail: None,
    };
    let mut factory = DoubleFactory {
        fail: None,
        closed: Rc::new(Cell::new(false)),
    };
    let mut authority = DoubleAuthority {
        fail: None,
        bound: false,
    };
    let mut sink = DoubleSink { calls: Vec::new() };
    let mut log = Vec::new();

    let req = NewSessionRequest::default();
    let outcome = run(
        &mut happy_deps(
            &mut store,
            &mut lease,
            &mut factory,
            &mut authority,
            &mut sink,
        ),
        &req,
        "s1",
        Some("/sessions/s1.jsonl"),
        &mut log,
    )
    .unwrap();

    assert_eq!(outcome.session_id, "s1");
    assert_eq!(&log[..FLOW_STEPS.len()], FLOW_STEPS);
    assert!(store.contains("s1"));
    assert_eq!(sink.calls, vec!["available_commands", "mcp_status"]);
    assert!(!sink.calls.contains(&"usage_update".to_owned()));
    assert!(authority.bound);
}

#[test]
fn lease_failure_closes_controller_and_does_not_register() {
    let mut store = DoubleStore {
        registered: Vec::new(),
    };
    let mut lease = DoubleLease {
        next_token: 0,
        released: Vec::new(),
        fail: Some(LeaseRefusal::Held {
            holder: "other window".to_owned(),
        }),
    };
    let mut factory = DoubleFactory {
        fail: None,
        closed: Rc::new(Cell::new(false)),
    };
    let mut authority = DoubleAuthority {
        fail: None,
        bound: false,
    };
    let mut sink = DoubleSink { calls: Vec::new() };
    let mut log = Vec::new();

    let err = run(
        &mut happy_deps(
            &mut store,
            &mut lease,
            &mut factory,
            &mut authority,
            &mut sink,
        ),
        &NewSessionRequest::default(),
        "s1",
        Some("/sessions/s1.jsonl"),
        &mut log,
    )
    .unwrap_err();

    assert_eq!(
        err.code,
        reasonix_acp::errors::RpcErrorCode::SessionLeaseHeld
    );
    assert!(!store.contains("s1"));
    assert!(factory.closed.get());
    assert!(lease.released.is_empty());
}

#[test]
fn authority_failure_closes_controller_and_releases_lease() {
    let mut store = DoubleStore {
        registered: Vec::new(),
    };
    let mut lease = DoubleLease {
        next_token: 0,
        released: Vec::new(),
        fail: None,
    };
    let mut factory = DoubleFactory {
        fail: None,
        closed: Rc::new(Cell::new(false)),
    };
    let mut authority = DoubleAuthority {
        fail: Some("authority bind boom".to_owned()),
        bound: false,
    };
    let mut sink = DoubleSink { calls: Vec::new() };
    let mut log = Vec::new();

    let err = run(
        &mut happy_deps(
            &mut store,
            &mut lease,
            &mut factory,
            &mut authority,
            &mut sink,
        ),
        &NewSessionRequest::default(),
        "s1",
        Some("/sessions/s1.jsonl"),
        &mut log,
    )
    .unwrap_err();

    assert!(err.message.contains("authority bind boom"));
    assert!(!store.contains("s1"));
    assert!(factory.closed.get());
    assert_eq!(lease.released, vec![1]);
}

#[test]
fn factory_failure_registers_nothing_and_acquires_no_lease() {
    let mut store = DoubleStore {
        registered: Vec::new(),
    };
    let mut lease = DoubleLease {
        next_token: 0,
        released: Vec::new(),
        fail: None,
    };
    let mut factory = DoubleFactory {
        fail: Some(NewFailure::internal("factory boom")),
        closed: Rc::new(Cell::new(false)),
    };
    let mut authority = DoubleAuthority {
        fail: None,
        bound: false,
    };
    let mut sink = DoubleSink { calls: Vec::new() };
    let mut log = Vec::new();

    let err = run(
        &mut happy_deps(
            &mut store,
            &mut lease,
            &mut factory,
            &mut authority,
            &mut sink,
        ),
        &NewSessionRequest::default(),
        "s1",
        Some("/sessions/s1.jsonl"),
        &mut log,
    )
    .unwrap_err();

    assert!(err.message.contains("factory boom"));
    assert!(!store.contains("s1"));
    assert_eq!(lease.next_token, 0);
    assert!(lease.released.is_empty());
}

#[test]
fn additional_dirs_vet_rejects_too_many_before_factory() {
    let dirs: Vec<String> = (0..33).map(|i| format!("/dir/{i}")).collect();
    let err = reasonix_acp::session_new::vet_additional_dirs(&dirs).unwrap_err();
    assert_eq!(err.data_code, "acp.additional_directories.too_many");
    assert!(err.message.contains("at most 32"));

    let long = vec!["x".repeat(4097)];
    let err = reasonix_acp::session_new::vet_additional_dirs(&long).unwrap_err();
    assert_eq!(err.data_code, "acp.additional_directories.path_too_long");
}

#[test]
fn parse_params_reads_cwd_dirs_and_mcp_servers() {
    let params = json!({
        "cwd": "/proj",
        "additionalDirectories": ["/extra"],
        "mcpServers": [{"name": "fs"}]
    });
    let req = parse_params(Some(&params)).unwrap();
    assert_eq!(req.cwd, "/proj");
    assert_eq!(req.additional_dirs, vec!["/extra".to_owned()]);
    assert_eq!(req.mcp_servers.len(), 1);

    let empty = parse_params(None).unwrap();
    assert_eq!(empty.cwd, "");
}
