//! Managed-config write approval seam (tools.md §1.21; Go
//! `internal/contract/tool/configwrite.go`, `internal/tools/builtin/managed_config.go`,
//! `internal/session/control/approval_bridge.go`).
//!
//! A file-tool write targeting a Reasonix-managed configuration file outside
//! the workspace write roots needs a fresh human approval. YOLO/auto must not
//! answer it; a missing approver fails closed. A session checker can short-
//! circuit a write already approved earlier in this session without
//! prompting again.
//!
//! Boundary with the wave-30 slice (TOOL-050): the baseline
//! [`super::confine::ManagedConfigHook`] trait, the `WriteHooks.managed`
//! hook slot, the confinement gate ordering, and the write/edit/move tool
//! wiring with their stub flow tests already exist. This module adds the
//! contract request shape, the approver / session-checker seams, the pure
//! resolved-path matcher, the approval decision table, and the approval
//! card's frozen strings. The concrete hook that adapts this seam to the
//! baseline trait belongs to the composition root.
//!
//! The managed config candidate list itself is produced by the composition
//! root from `reasonix_platform::user_config::managed_config_paths` (PATHS-007
//! done); this crate does not depend on platform, so the matcher receives
//! the already-resolved candidates and matches them by exact resolved-path
//! equality with no case folding.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use thiserror::Error;

use super::confine::real_path;
use crate::abi::CallContext;

/// Approval decision tool id (Go `controller.ManagedConfigWriteApprovalTool`,
/// `permission` namespace): the tool+subject namespace a session grant
/// books under.
pub const CONFIG_WRITE_APPROVAL_TOOL: &str = "config_write";

/// Approval card subject prefix (Go `i18n.ConfigWriteSubjectPrefix`,
/// messages_en.go:233).
pub const CONFIG_WRITE_SUBJECT_PREFIX: &str = "write Reasonix config: ";

/// Approval card reason (Go `i18n.ConfigWriteReason`, messages_en.go:234).
pub const CONFIG_WRITE_REASON: &str = "This write targets a Reasonix-managed configuration file outside the workspace. It can change providers, sandbox rules, permissions, and MCP servers for future sessions, so it needs your explicit approval.";

/// Controller-side declined reason (Go `i18n.ConfigWriteDeclined`,
/// messages_en.go:235, hyphen spelling).
pub const CONFIG_WRITE_DECLINED: &str = "the user declined this Reasonix config write - do not retry it; ask how they would like to proceed.";

/// Tool-layer fallback when a fresh approval declines without leaving a
/// reason (Go `ManagedConfigPaths.approve`, managed_config.go:75, em dash
/// spelling). Kept apart from the controller i18n string because the two
/// Go sources differ only in dash style.
pub const DECLINE_REASON_DEFAULT: &str = "the user declined this Reasonix config write — do not retry it; ask how they would like to proceed.";

/// A file-tool write targeting a Reasonix-managed configuration file outside
/// the workspace write roots.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigWriteRequest {
    /// The resolved absolute target the tool wants to write.
    pub path: String,
}

/// Builds the approval subject: prefix + trimmed path (Go
/// `managedConfigWriteApprovalSubject`, approval_bridge.go:345-347).
pub fn config_write_subject(req: &ConfigWriteRequest) -> String {
    format!("{CONFIG_WRITE_SUBJECT_PREFIX}{}", req.path.trim())
}

/// Asks the user whether one write to a Reasonix-managed config file may
/// proceed (Go `tool.ConfigWriteApprover`). Fresh human decision: YOLO/auto
/// must not answer; a missing approver fails closed. The returned tuple is
/// `(allow, reason)`; an empty reason on decline is replaced by
/// [`DECLINE_REASON_DEFAULT`].
pub trait ConfigWriteApprover: Send + Sync {
    fn approve_managed_config_write(
        &self,
        req: &ConfigWriteRequest,
    ) -> Result<(bool, String), Box<dyn std::error::Error + Send + Sync>>;

    /// Session-grant arm (Go `tool.ConfigWriteSessionChecker`): whether the
    /// write has already been approved for the current session, so the user
    /// is not prompted again. The default answers `false`; a host that
    /// remembers session grants overrides it.
    fn managed_config_write_session_allowed(&self, _req: &ConfigWriteRequest) -> bool {
        false
    }

    /// Type-erased view, so a host can recognize its own concrete approver.
    fn as_any(&self) -> &dyn Any;
}

/// Already-approved check without prompting the user again (Go
/// `tool.ConfigWriteSessionChecker`). Kept as the separate Go interface;
/// the approver's provided [`ConfigWriteApprover::managed_config_write_session_allowed`]
/// is what the decision table reads.
pub trait ConfigWriteSessionChecker: Send + Sync {
    fn managed_config_write_session_allowed(&self, req: &ConfigWriteRequest) -> bool;
}

/// The approver bound to one call, when the host installed one (Go
/// `tool.ConfigWriteApproverFrom`). A [`None`] return is the fail-closed
/// condition.
pub fn config_write_approver_from(ctx: &CallContext) -> Option<Arc<dyn ConfigWriteApprover>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn ConfigWriteApprover>>()
        .cloned()
}

/// The decision-table identity (Go `ManagedConfigPaths.approve`'s three
/// outcomes). The message text is model-facing and frozen; the variant is
/// what a caller classifies on.
#[derive(Debug, Error)]
pub enum ManagedConfigError {
    /// No approver on this call: headless run, or a sub-agent whose parent
    /// has no interactive frontend. Fails closed.
    #[error(
        "path \"{path}\" is a Reasonix-managed config file outside the writable roots; writing it requires interactive user approval, which this session cannot provide. Ask the user to retry in an interactive session, or to add the directory to [sandbox] allow_write in reasonix.toml"
    )]
    NoApprover { path: String },

    /// The user declined this write; `reason` is their words or the default.
    #[error("{reason}")]
    Declined { reason: String },

    /// The approver itself failed (transport, stopped turn, ...); the
    /// decision did not land.
    #[error("managed-config approval failed: {0}")]
    Approver(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// The Reasonix-owned configuration files a file tool may write outside the
/// workspace roots, each gated by a fresh human approval (Go
/// `ManagedConfigPaths`). The zero value matches nothing, preserving plain
/// workspace confinement. Entries are individual files, never directories:
/// the Reasonix home also holds credentials, global hooks, skills, and
/// session stores, which must not become writable through this escape hatch.
#[derive(Debug, Clone, Default)]
pub struct ManagedConfigPaths {
    paths: Vec<String>,
}

impl ManagedConfigPaths {
    /// Resolves each candidate file to an absolute, cleaned path (mirroring
    /// the shared real-path resolver), dropping empty or unresolvable
    /// entries. Candidates are injected by the composition root from the
    /// platform managed-config list.
    pub fn new(candidates: &[String]) -> Self {
        let mut paths = Vec::with_capacity(candidates.len());
        for p in candidates {
            if p.trim().is_empty() {
                continue;
            }
            if let Ok(real) = real_path(p) {
                paths.push(real);
            }
        }
        Self { paths }
    }

    /// Whether `target` resolves to exactly one of the managed config files.
    /// Exact file equality with no case folding: this is an allow-side rule,
    /// and folding an allow rule on a case-sensitive filesystem would wave a
    /// genuinely different file through.
    pub fn matches(&self, target: &str) -> bool {
        if self.paths.is_empty() {
            return false;
        }
        let Ok(abs) = real_path(target) else {
            return false;
        };
        self.paths.contains(&abs)
    }

    /// The approval decision table (Go `ManagedConfigPaths.approve`). Order
    /// is frozen: no approver fails closed; an earlier session grant passes
    /// without prompting; otherwise the fresh human decision lands, and a
    /// decline without words carries the default reason.
    pub fn approve(
        &self,
        approver: Option<&dyn ConfigWriteApprover>,
        req: &ConfigWriteRequest,
    ) -> Result<(), ManagedConfigError> {
        let Some(approver) = approver else {
            return Err(ManagedConfigError::NoApprover {
                path: req.path.clone(),
            });
        };
        if approver.managed_config_write_session_allowed(req) {
            return Ok(());
        }
        let (allow, reason) = approver
            .approve_managed_config_write(req)
            .map_err(ManagedConfigError::Approver)?;
        if !allow {
            let reason = if reason.trim().is_empty() {
                DECLINE_REASON_DEFAULT.to_owned()
            } else {
                reason
            };
            return Err(ManagedConfigError::Declined { reason });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::CallContext;

    struct StubApprover {
        allow: bool,
        reason: String,
        session_allowed: bool,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl ConfigWriteApprover for StubApprover {
        fn approve_managed_config_write(
            &self,
            _req: &ConfigWriteRequest,
        ) -> Result<(bool, String), Box<dyn std::error::Error + Send + Sync>> {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok((self.allow, self.reason.clone()))
        }

        fn managed_config_write_session_allowed(&self, _req: &ConfigWriteRequest) -> bool {
            self.session_allowed
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn req() -> ConfigWriteRequest {
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
            CONFIG_WRITE_REASON
                .starts_with("This write targets a Reasonix-managed configuration file")
        );
        assert_eq!(
            CONFIG_WRITE_DECLINED,
            "the user declined this Reasonix config write - do not retry it; ask how they would like to proceed."
        );
        // The tool-layer fallback keeps the em-dash spelling from managed_config.go:75.
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
    fn no_approver_fails_closed_with_the_boundary_message() {
        let paths = ManagedConfigPaths::default();
        let err = paths.approve(None, &req()).unwrap_err();
        match err {
            ManagedConfigError::NoApprover { ref path } => {
                assert_eq!(path, &req().path);
                let text = err.to_string();
                assert!(text.contains("Reasonix-managed config file"));
                assert!(text.contains("interactive user approval"));
                assert!(text.contains("[sandbox] allow_write in reasonix.toml"));
            }
            other => panic!("want NoApprover, got {other:?}"),
        }
    }

    #[test]
    fn a_session_grant_passes_without_prompting_again() {
        let stub = Arc::new(StubApprover {
            allow: false,
            reason: String::new(),
            session_allowed: true,
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        let paths = ManagedConfigPaths::default();
        paths
            .approve(Some(stub.as_ref()), &req())
            .expect("session grant passes");
        assert_eq!(
            stub.asked.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an already-approved write must not reach the fresh decision"
        );
    }

    #[test]
    fn a_fresh_approval_passes_and_counts_one_decision() {
        let stub = Arc::new(StubApprover {
            allow: true,
            reason: String::new(),
            session_allowed: false,
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        let paths = ManagedConfigPaths::default();
        paths
            .approve(Some(stub.as_ref()), &req())
            .expect("fresh approval passes");
        assert_eq!(stub.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_decline_without_words_carries_the_default_reason() {
        let stub = Arc::new(StubApprover {
            allow: false,
            reason: String::new(),
            session_allowed: false,
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        let paths = ManagedConfigPaths::default();
        let err = paths.approve(Some(stub.as_ref()), &req()).unwrap_err();
        assert_eq!(err.to_string(), DECLINE_REASON_DEFAULT);
    }

    #[test]
    fn a_decline_with_words_surfaces_them_verbatim() {
        let custom = "not this one — change the port first".to_owned();
        let stub = Arc::new(StubApprover {
            allow: false,
            reason: custom.clone(),
            session_allowed: false,
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        let paths = ManagedConfigPaths::default();
        let err = paths.approve(Some(stub.as_ref()), &req()).unwrap_err();
        assert_eq!(err.to_string(), custom);
    }

    #[test]
    fn an_approver_transport_error_propagates_as_the_approver_outcome() {
        struct FailingApprover;
        impl ConfigWriteApprover for FailingApprover {
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
        let err = paths.approve(Some(&FailingApprover), &req()).unwrap_err();
        assert!(matches!(err, ManagedConfigError::Approver(_)));
        assert!(err.to_string().contains("prompt stopped"));
    }

    #[test]
    fn the_matcher_resolves_exact_files_and_ignores_directories_and_relatives() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("managed-config-paths")
            .tempdir_in(&base)
            .unwrap();
        let dir_path = dir.path().to_string_lossy().into_owned();
        let config = std::path::Path::new(&dir_path).join("reasonix.toml");
        std::fs::write(&config, "# managed").unwrap();
        let managed = ManagedConfigPaths::new(&[config.to_string_lossy().into_owned()]);

        assert!(managed.matches(&config.to_string_lossy()));
        // A sibling file in the same directory is not on the allow list.
        let sibling = std::path::Path::new(&dir_path).join("notes.txt");
        assert!(!managed.matches(&sibling.to_string_lossy()));
        // Empty candidates match nothing.
        let empty = ManagedConfigPaths::new(&["  ".to_owned()]);
        assert!(!empty.matches(&config.to_string_lossy()));
    }

    #[test]
    fn the_approver_binding_round_trips_through_the_call_context() {
        let stub = Arc::new(StubApprover {
            allow: true,
            reason: String::new(),
            session_allowed: false,
            asked: std::sync::atomic::AtomicUsize::new(0),
        });
        let port: Arc<dyn ConfigWriteApprover> = stub.clone();
        let ctx = CallContext::new().with_value(Arc::new(port));
        let bound = config_write_approver_from(&ctx).expect("approver installed");
        let (allow, _) = bound
            .approve_managed_config_write(&req())
            .expect("decision");
        assert!(allow);
        assert!(config_write_approver_from(&CallContext::new()).is_none());
    }
}
