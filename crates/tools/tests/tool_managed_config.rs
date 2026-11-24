//! TOOL-016 managed-config write approval seam (Go
//! `internal/contract/tool/configwrite.go`, `internal/tools/builtin/managed_config.go`,
//! docs/contracts/tools.md §1.21): the request shape, the approver / session-
//! checker seams, the resolved-path matcher, and the approval decision table.
//!
//! Boundary with wave 30: the baseline [`reasonix_tools::builtin::ManagedConfigHook`]
//! trait and the writer wiring (`confine_write`, the write/edit/move tools)
//! already land the managed-target branch and its approve/decline/skip
//! flow tests in `builtin_writeconfine.rs`. This file exercises only the new
//! contract layer added here.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reasonix_tools::abi::CallContext;
use reasonix_tools::builtin::{
    CONFIG_WRITE_APPROVAL_TOOL, CONFIG_WRITE_DECLINED, CONFIG_WRITE_REASON,
    CONFIG_WRITE_SUBJECT_PREFIX, ConfigWriteApprover, ConfigWriteRequest, DECLINE_REASON_DEFAULT,
    ManagedConfigError, ManagedConfigPaths, config_write_approver_from, config_write_subject,
};

struct StubApprover {
    allow: bool,
    reason: String,
    session_allowed: bool,
    asked: AtomicUsize,
}

impl ConfigWriteApprover for StubApprover {
    fn approve_managed_config_write(
        &self,
        _req: &ConfigWriteRequest,
    ) -> Result<(bool, String), Box<dyn std::error::Error + Send + Sync>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok((self.allow, self.reason.clone()))
    }

    fn managed_config_write_session_allowed(&self, _req: &ConfigWriteRequest) -> bool {
        self.session_allowed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn managed_req() -> ConfigWriteRequest {
    ConfigWriteRequest {
        path: String::from(if cfg!(windows) {
            "C:\\Users\\tester\\.reasonix\\reasonix.toml"
        } else {
            "/home/tester/.reasonix/reasonix.toml"
        }),
    }
}

#[test]
fn approval_strings_and_tool_id_match_the_go_card() {
    assert_eq!(CONFIG_WRITE_APPROVAL_TOOL, "config_write");
    assert_eq!(CONFIG_WRITE_SUBJECT_PREFIX, "write Reasonix config: ");
    assert!(
        CONFIG_WRITE_REASON.starts_with("This write targets a Reasonix-managed configuration file")
    );
    assert_eq!(
        CONFIG_WRITE_DECLINED,
        "the user declined this Reasonix config write - do not retry it; ask how they would like to proceed."
    );
    assert!(DECLINE_REASON_DEFAULT.contains("—"));
}

#[test]
fn the_subject_is_prefix_plus_trimmed_path() {
    let req = ConfigWriteRequest {
        path: "  /home/tester/.reasonix/reasonix.toml  ".to_owned(),
    };
    assert_eq!(
        config_write_subject(&req),
        "write Reasonix config: /home/tester/.reasonix/reasonix.toml"
    );
}

#[test]
fn no_approver_fails_closed_and_names_the_boundary() {
    let paths = ManagedConfigPaths::default();
    let err = paths.approve(None, &managed_req()).unwrap_err();
    match err {
        ManagedConfigError::NoApprover { ref path } => {
            assert_eq!(path, &managed_req().path);
            let text = err.to_string();
            assert!(text.contains("Reasonix-managed config file"));
            assert!(text.contains("interactive user approval"));
            assert!(text.contains("[sandbox] allow_write in reasonix.toml"));
        }
        other => panic!("want NoApprover, got {other:?}"),
    }
}

#[test]
fn a_session_grant_passes_without_a_fresh_prompt() {
    let stub = Arc::new(StubApprover {
        allow: false,
        reason: String::new(),
        session_allowed: true,
        asked: AtomicUsize::new(0),
    });
    let paths = ManagedConfigPaths::default();
    paths
        .approve(Some(stub.as_ref()), &managed_req())
        .expect("session grant passes");
    assert_eq!(
        stub.asked.load(Ordering::SeqCst),
        0,
        "already approved this session: do not prompt again"
    );
}

#[test]
fn a_fresh_approval_passes_after_one_decision() {
    let stub = Arc::new(StubApprover {
        allow: true,
        reason: String::new(),
        session_allowed: false,
        asked: AtomicUsize::new(0),
    });
    let paths = ManagedConfigPaths::default();
    paths
        .approve(Some(stub.as_ref()), &managed_req())
        .expect("fresh approval passes");
    assert_eq!(stub.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_decline_without_words_uses_the_default_reason() {
    let stub = Arc::new(StubApprover {
        allow: false,
        reason: String::new(),
        session_allowed: false,
        asked: AtomicUsize::new(0),
    });
    let paths = ManagedConfigPaths::default();
    let err = paths
        .approve(Some(stub.as_ref()), &managed_req())
        .unwrap_err();
    assert_eq!(err.to_string(), DECLINE_REASON_DEFAULT);
}

#[test]
fn a_decline_with_words_surfaces_the_reason_verbatim() {
    let custom = "not this one — change the port first".to_owned();
    let stub = Arc::new(StubApprover {
        allow: false,
        reason: custom.clone(),
        session_allowed: false,
        asked: AtomicUsize::new(0),
    });
    let paths = ManagedConfigPaths::default();
    let err = paths
        .approve(Some(stub.as_ref()), &managed_req())
        .unwrap_err();
    assert_eq!(err.to_string(), custom);
}

#[test]
fn an_approver_error_is_the_approver_outcome_not_a_decline() {
    struct Failing;
    impl ConfigWriteApprover for Failing {
        fn approve_managed_config_write(
            &self,
            _req: &ConfigWriteRequest,
        ) -> Result<(bool, String), Box<dyn std::error::Error + Send + Sync>> {
            Err(Box::new(std::io::Error::other("prompt stopped")))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    let paths = ManagedConfigPaths::default();
    let err = paths.approve(Some(&Failing), &managed_req()).unwrap_err();
    assert!(matches!(err, ManagedConfigError::Approver(_)));
    assert!(err.to_string().contains("prompt stopped"));
}

#[test]
fn the_matcher_matches_exact_files_only_after_real_path_resolution() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("managed-config-integration")
        .tempdir_in(&base)
        .unwrap();
    let config = dir.path().join("reasonix.toml");
    std::fs::write(&config, "# managed").unwrap();
    let managed = ManagedConfigPaths::new(&[config.to_string_lossy().into_owned()]);

    assert!(managed.matches(&config.to_string_lossy()));
    let sibling = dir.path().join("notes.txt");
    assert!(!managed.matches(&sibling.to_string_lossy()));
    assert!(!managed.matches("/not/here.toml"));

    let empty = ManagedConfigPaths::new(&["  ".to_owned(), "".to_owned()]);
    assert!(!empty.matches(&config.to_string_lossy()));
}

#[test]
fn the_approver_binding_round_trips_through_the_call_context() {
    let stub = Arc::new(StubApprover {
        allow: true,
        reason: String::new(),
        session_allowed: false,
        asked: AtomicUsize::new(0),
    });
    let port: Arc<dyn ConfigWriteApprover> = stub;
    let ctx = CallContext::new().with_value(Arc::new(port));
    let bound = config_write_approver_from(&ctx).expect("approver installed");
    let (allow, _) = bound
        .approve_managed_config_write(&managed_req())
        .expect("decision");
    assert!(allow);
    assert!(config_write_approver_from(&CallContext::new()).is_none());
}
