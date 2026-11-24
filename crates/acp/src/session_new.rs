//! `session/new` orchestration: step order, lease acquisition, registration,
//! and the afterResponse hook (session.go:91-211; matrix B §6).
//!
//! The real session store, transcript lease, controller factory, and event sink
//! are runtime wiring. This module defines the injectable seams, the pure
//! additional-directory vet rules, and a flow that records its step order so
//! tests can assert sequencing and failure cleanup without opening a session.
//!
//! Rust port divergence from the Go oracle: the authority-binding failure
//! branch closes the controller and releases the acquired lease. The reference
//! leaves that controller unclosed (matrix B §6.1 step 14 note, §6.6); the
//! port does not copy the leak.

use serde_json::Value;

use crate::errors::{RpcError, RpcErrorCode};
use crate::lifecycle::LeaseRefusal;

// ---- Pure request and directory vet --------------------------------------------

/// Inbound `session/new` parameters accepted by the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewSessionRequest {
    pub cwd: String,
    pub additional_dirs: Vec<String>,
    pub mcp_servers: Vec<Value>,
}

/// Additional-directory rejection data.code (additional_dirs.go:16-19, 38-67).
pub const DIRS_TOO_MANY: &str = "acp.additional_directories.too_many";
pub const DIRS_PATH_TOO_LONG: &str = "acp.additional_directories.path_too_long";
pub const DIRS_DISABLED: &str = "acp.additional_directories.disabled";

/// At most thirty-two additional directories per session.
pub const MAX_ADDITIONAL_DIRS: usize = 32;
/// Each additional directory path is at most 4096 characters.
pub const MAX_ADDITIONAL_DIR_LEN: usize = 4096;

/// Additional-directory vet failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirVetError {
    pub data_code: &'static str,
    pub message: String,
}

/// Vets the additional directory list: count cap, per-path length cap. Path
/// existence and root validity are checked by the store/runtime; the whole
/// list is rejected together (additional_dirs.go:38-67).
pub fn vet_additional_dirs(dirs: &[String]) -> Result<(), DirVetError> {
    if dirs.len() > MAX_ADDITIONAL_DIRS {
        return Err(DirVetError {
            data_code: DIRS_TOO_MANY,
            message: format!("session/new: at most {MAX_ADDITIONAL_DIRS} additional directories"),
        });
    }
    for dir in dirs {
        if dir.chars().count() > MAX_ADDITIONAL_DIR_LEN {
            return Err(DirVetError {
                data_code: DIRS_PATH_TOO_LONG,
                message: format!("session/new: additional directory path too long: {dir}"),
            });
        }
    }
    Ok(())
}

// ---- Injectable seams -------------------------------------------------------------

/// Session registry the flow registers into after a successful open.
pub trait SessionStore {
    /// Records an open session under its id.
    fn register(&mut self, id: &str);
    /// Whether the store already holds the id (failure-cleanup assertion).
    fn contains(&self, id: &str) -> bool;
}

/// Transcript lease manager; the runtime backs this with the sessionstore lease.
pub trait LeaseManager {
    /// Acquires the lease for a transcript path.
    fn acquire(&mut self, transcript: &str) -> Result<usize, LeaseRefusal>;
    /// Releases a lease token returned by [`LeaseManager::acquire`].
    fn release(&mut self, token: usize);
}

/// Controller handle produced by the factory. The runtime implements these
/// over the real session controller.
pub trait ControllerHandle {
    /// Releases controller resources; called on every failure cleanup path.
    fn close(&mut self);
    /// Whether the controller opened in plan mode (startMode derivation).
    fn plan_mode(&self) -> bool;
    /// Session directory when the session persists; None means no transcript.
    fn session_dir(&self) -> Option<&str>;
}

/// Builds the controller for a new session.
pub trait SessionFactory {
    /// Creates the controller; the failure class maps to `-32000` or `-32603`.
    fn build(&mut self, req: &NewSessionRequest) -> Result<Box<dyn ControllerHandle>, NewFailure>;
}

/// Binds the ACP write authority to the controller (recovery-write path).
pub trait AuthorityBinder {
    /// Binds authority; an error aborts the open after lease acquisition.
    fn bind(&mut self, controller: &mut dyn ControllerHandle) -> Result<(), String>;
}

/// Outbound event sink; the flow only calls the steps its afterResponse allows.
pub trait EventSink {
    /// Pushes the available slash commands.
    fn send_available_commands(&mut self);
    /// Broadcasts the first MCP status snapshot.
    fn publish_mcp_status(&mut self);
    /// Pushes a usage update; `session/new` afterResponse does **not** call this.
    fn send_usage_update(&mut self);
}

/// Classified open failure carried out of the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFailure {
    pub code: RpcErrorCode,
    pub message: String,
    pub data_code: Option<&'static str>,
}

impl NewFailure {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: RpcErrorCode::Internal,
            message: message.into(),
            data_code: None,
        }
    }

    pub fn to_rpc_error(&self) -> RpcError {
        let mut err = RpcError::new(self.code, self.message.clone());
        if let Some(code) = &self.data_code {
            err = err.with_data(serde_json::json!({"code": code}));
        }
        err
    }
}

// ---- Flow ---------------------------------------------------------------------------

/// Recorded afterResponse steps for a `session/new` open.
pub const AFTER_AVAILABLE_COMMANDS: &str = "afterResponse.sendAvailableCommands";
pub const AFTER_MCP_STATUS: &str = "afterResponse.publishMCPStatus";

/// Result of a successful open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewSessionOutcome {
    pub session_id: String,
    pub after_steps: Vec<String>,
}

/// The 16 orchestration steps, recorded in order as the flow runs
/// (matrix B §6.1). Tests assert the log contains these in this order.
pub const FLOW_STEPS: [&str; 16] = [
    "parse_params",
    "resolve_cwd",
    "mcp_specs",
    "vet_additional_dirs",
    "config_state",
    "posture_runtime_state",
    "new_session_id",
    "build_sink",
    "assemble_params",
    "factory_new_session",
    "bind_approval",
    "construct_session",
    "bind_health_events",
    "acquire_lease",
    "register_session",
    "merge_extension_dir_model",
];

/// Dependencies assembled for one open.
pub struct FlowDeps<'a> {
    pub store: &'a mut dyn SessionStore,
    pub lease: &'a mut dyn LeaseManager,
    pub factory: &'a mut dyn SessionFactory,
    pub authority: &'a mut dyn AuthorityBinder,
    pub sink: &'a mut dyn EventSink,
}

/// Runs the `session/new` flow, recording each step. After a successful open the
/// afterResponse hook sends available commands and the MCP status snapshot; it
/// does not send a usage update (matrix B §6.4).
pub fn run(
    deps: &mut FlowDeps<'_>,
    req: &NewSessionRequest,
    session_id: &str,
    transcript: Option<&str>,
    log: &mut Vec<String>,
) -> Result<NewSessionOutcome, NewFailure> {
    let record = |log: &mut Vec<String>, step: &str| log.push(step.to_owned());

    record(log, FLOW_STEPS[0]);
    record(log, FLOW_STEPS[1]);
    record(log, FLOW_STEPS[2]);
    record(log, FLOW_STEPS[3]);
    vet_additional_dirs(&req.additional_dirs).map_err(|err| NewFailure {
        code: RpcErrorCode::InvalidParams,
        message: err.message,
        data_code: Some(err.data_code),
    })?;
    record(log, FLOW_STEPS[4]);
    record(log, FLOW_STEPS[5]);
    record(log, FLOW_STEPS[6]);
    record(log, FLOW_STEPS[7]);
    record(log, FLOW_STEPS[8]);

    record(log, FLOW_STEPS[9]);
    // Factory internal cleanup owns its internals; nothing is registered yet.
    let mut controller = deps.factory.build(req)?;

    record(log, FLOW_STEPS[10]);
    record(log, FLOW_STEPS[11]);
    record(log, FLOW_STEPS[12]);

    record(log, FLOW_STEPS[13]);
    let lease_token = match transcript {
        Some(path) => match deps.lease.acquire(path) {
            Ok(token) => Some(token),
            Err(refusal) => {
                // Lease failure: close the controller before returning the refusal.
                controller.close();
                return Err(lease_failure(refusal));
            }
        },
        None => None,
    };

    if let Err(err) = deps.authority.bind(&mut *controller) {
        // Authority binding failure: the port closes the controller and releases
        // the acquired lease instead of leaking either (Go divergence, §6.6).
        controller.close();
        if let Some(token) = lease_token {
            deps.lease.release(token);
        }
        return Err(NewFailure::internal(format!("session/new: {err}")));
    }

    record(log, FLOW_STEPS[14]);
    deps.store.register(session_id);

    record(log, FLOW_STEPS[15]);

    // afterResponse runs only after the success result has been written to the
    // wire. New opens advertise commands and the first MCP status; no usage update.
    let mut after_steps = Vec::new();
    deps.sink.send_available_commands();
    after_steps.push(AFTER_AVAILABLE_COMMANDS.to_owned());
    deps.sink.publish_mcp_status();
    after_steps.push(AFTER_MCP_STATUS.to_owned());

    Ok(NewSessionOutcome {
        session_id: session_id.to_owned(),
        after_steps,
    })
}

/// Maps a lease refusal onto the classified open failure (session.go:64-74).
fn lease_failure(refusal: LeaseRefusal) -> NewFailure {
    match refusal {
        LeaseRefusal::Held { holder } => NewFailure {
            code: RpcErrorCode::SessionLeaseHeld,
            message: format!("session/new: {holder}"),
            data_code: Some(crate::lifecycle::CODE_SESSION_IN_USE),
        },
        LeaseRefusal::Unprovable { detail } => NewFailure {
            code: RpcErrorCode::SessionLeaseUnprovable,
            message: format!("session/new: {detail}"),
            data_code: Some(crate::lifecycle::CODE_LEASE_UNPROVABLE),
        },
        LeaseRefusal::Other { detail } => NewFailure {
            code: RpcErrorCode::Internal,
            message: format!("session/new: session lease: {detail}"),
            data_code: None,
        },
    }
}

/// Convenience map for tests and runtime callers that hold an open parameter map.
pub fn parse_params(params: Option<&Value>) -> Result<NewSessionRequest, NewFailure> {
    let Some(params) = params else {
        return Ok(NewSessionRequest::default());
    };
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let additional_dirs = params
        .get("additionalDirectories")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mcp_servers = params
        .get("mcpServers")
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let mcp_servers = mcp_servers.as_array().cloned().unwrap_or_default();
    Ok(NewSessionRequest {
        cwd,
        additional_dirs,
        mcp_servers,
    })
}
