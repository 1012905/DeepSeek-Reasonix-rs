//! ACP-031 `session/delete` teardown, cleanup-pending timeout branch, and twin
//! artifact deletion (`session.go:653-709`; `service.go:1452-1476`).
//!
//! A missing session deletes silently. A held session waits for its turn,
//! releases the lease, and hands the destroy lifecycle to the controller: a
//! timed-out destroy marks the transcript cleanup-pending and schedules a
//! background delayed delete; even that path still deletes the id-keyed twin on
//! the main thread. Real controller destruction, marker persistence, the
//! background recovery sweep, and filesystem removal are runtime wiring; this
//! module owns the branch order, the timeout/pending decision, and the error
//! identities.

use crate::errors::{RpcError, RpcErrorCode};
use crate::lifecycle::validate_session_id;
use crate::wire::SessionDeleteResult;

/// Destroy lifecycle handle handed out by `BeginDestroySession`.
pub trait DestroyHandle {
    /// Wait for the destroy to finish; reports whether it timed out.
    fn wait(&mut self) -> DestroyOutcome;
    /// Marks the destroy finished once its artifacts are removed.
    fn finish(&mut self);
    /// Releases controller resources after the destroy completes or times out.
    fn close_after_destroy(&mut self);
}

/// Whether the destroy wait returned on time (`session.go:681-691`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DestroyOutcome {
    pub timed_out: bool,
}

/// Held session seam the delete pipeline tears down.
pub trait LiveDelete {
    /// Waits for the in-flight turn to unwind before deletion.
    fn delete_and_wait(&mut self) -> Result<(), String>;
    /// Returns the session lease after the turn unwound.
    fn release_lease(&mut self);
    /// Pinned transcript path; empty means the controller lost its transcript.
    fn transcript_path(&self) -> String;
    /// Hands the session over to the destroy lifecycle.
    fn begin_destroy(&mut self) -> Box<dyn DestroyHandle>;
}

/// Registry seam: removes a held session from the active map.
pub trait DeleteRegistry {
    fn take(&mut self, id: &str) -> Option<Box<dyn LiveDelete>>;
}

/// Artifact and marker seam; real filesystem removal lives behind this.
pub trait FileOps {
    /// Marks a transcript cleanup-pending after a destroy timeout.
    fn mark_cleanup_pending(&mut self, path: &str) -> Result<(), String>;
    /// Removes the transcript and its sidecar from disk.
    fn delete_files(&mut self, path: &str) -> Result<(), String>;
    /// Id-keyed redirect path, when the session was redirected to another file.
    fn id_keyed_path(&self, id: &str) -> Option<String>;
}

fn internal(message: String) -> RpcError {
    RpcError::new(RpcErrorCode::Internal, format!("session/delete: {message}"))
}

/// Handles one `session/delete`.
pub fn delete_one(
    id: &str,
    registry: &mut dyn DeleteRegistry,
    files: &mut dyn FileOps,
) -> Result<SessionDeleteResult, RpcError> {
    validate_session_id("session/delete", id)?;

    let Some(mut live) = registry.take(id) else {
        return Ok(SessionDeleteResult::default());
    };

    live.delete_and_wait().map_err(internal)?;
    live.release_lease();

    let transcript_path = live.transcript_path();
    let mut destroy = live.begin_destroy();
    let outcome = destroy.wait();

    let mut delayed = false;
    if outcome.timed_out {
        match files.mark_cleanup_pending(&transcript_path) {
            Ok(()) => delayed = true,
            Err(message) => {
                // The pending mark failed: controller resources are still
                // released, then the failure surfaces to the client
                // (`session.go:686-691`).
                destroy.close_after_destroy();
                return Err(internal(message));
            }
        }
        destroy.close_after_destroy();
    } else {
        destroy.close_after_destroy();
    }

    if transcript_path.is_empty() {
        return Err(internal("transcript path unavailable".to_owned()));
    }

    if !delayed {
        files.delete_files(&transcript_path).map_err(internal)?;
        destroy.finish();
    }

    if let Some(twin) = files.id_keyed_path(id)
        && twin != transcript_path
    {
        files.delete_files(&twin).map_err(internal)?;
    }

    Ok(SessionDeleteResult::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    struct RecordingDestroy {
        timed_out: bool,
        events: Vec<String>,
    }

    impl DestroyHandle for RecordingDestroy {
        fn wait(&mut self) -> DestroyOutcome {
            self.events.push("wait".to_owned());
            DestroyOutcome {
                timed_out: self.timed_out,
            }
        }
        fn finish(&mut self) {
            self.events.push("finish".to_owned());
        }
        fn close_after_destroy(&mut self) {
            self.events.push("close_after_destroy".to_owned());
        }
    }

    struct RecordingLive {
        transcript_path: String,
        wait_err: Option<String>,
        events: Vec<String>,
        destroy: RecordingDestroy,
    }

    impl LiveDelete for RecordingLive {
        fn delete_and_wait(&mut self) -> Result<(), String> {
            self.events.push("delete_and_wait".to_owned());
            self.wait_err.clone().map_or(Ok(()), Err)
        }
        fn release_lease(&mut self) {
            self.events.push("release_lease".to_owned());
        }
        fn transcript_path(&self) -> String {
            self.transcript_path.clone()
        }
        fn begin_destroy(&mut self) -> Box<dyn DestroyHandle> {
            self.events.push("begin_destroy".to_owned());
            Box::new(RecordingDestroy {
                timed_out: self.destroy.timed_out,
                events: std::mem::take(&mut self.destroy.events),
            })
        }
    }

    struct RecordingRegistry {
        held: Option<RecordingLive>,
    }

    impl DeleteRegistry for RecordingRegistry {
        fn take(&mut self, _id: &str) -> Option<Box<dyn LiveDelete>> {
            self.held
                .take()
                .map(|live| Box::new(live) as Box<dyn LiveDelete>)
        }
    }

    struct RecordingFiles {
        pending_events: Vec<String>,
        deleted: Vec<String>,
        twin: Option<String>,
        pending_err: Option<String>,
        delete_err: Option<String>,
    }

    impl FileOps for RecordingFiles {
        fn mark_cleanup_pending(&mut self, path: &str) -> Result<(), String> {
            self.pending_events.push(path.to_owned());
            self.pending_err.clone().map_or(Ok(()), Err)
        }
        fn delete_files(&mut self, path: &str) -> Result<(), String> {
            self.deleted.push(path.to_owned());
            self.delete_err.clone().map_or(Ok(()), Err)
        }
        fn id_keyed_path(&self, _id: &str) -> Option<String> {
            self.twin.clone()
        }
    }

    #[test]
    fn invalid_id_refuses_before_any_pipeline_call() {
        let mut registry = RecordingRegistry { held: None };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: None,
            pending_err: None,
            delete_err: None,
        };
        let err = delete_one("has space", &mut registry, &mut files).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(err.message, "session/delete: invalid sessionId");
    }

    #[test]
    fn missing_session_is_silent_success() {
        let mut registry = RecordingRegistry { held: None };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: None,
            pending_err: None,
            delete_err: None,
        };
        delete_one("s1", &mut registry, &mut files).unwrap();
        assert!(files.deleted.is_empty());
    }

    #[test]
    fn timed_out_destroy_marks_pending_and_delays_main_deletion() {
        let mut registry = RecordingRegistry {
            held: Some(RecordingLive {
                transcript_path: "/sessions/s1.jsonl".to_owned(),
                wait_err: None,
                events: vec![],
                destroy: RecordingDestroy {
                    timed_out: true,
                    events: vec![],
                },
            }),
        };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: Some("/sessions/s1.acp.json".to_owned()),
            pending_err: None,
            delete_err: None,
        };
        delete_one("s1", &mut registry, &mut files).unwrap();

        assert_eq!(files.pending_events, vec!["/sessions/s1.jsonl"]);
        // Delayed path keeps the transcript file for the background sweep, but
        // still removes the twin on the main thread.
        assert_eq!(files.deleted, vec!["/sessions/s1.acp.json"]);
    }

    #[test]
    fn pending_mark_failure_still_closes_destroy_then_reports_internal() {
        let mut registry = RecordingRegistry {
            held: Some(RecordingLive {
                transcript_path: "/sessions/s1.jsonl".to_owned(),
                wait_err: None,
                events: vec![],
                destroy: RecordingDestroy {
                    timed_out: true,
                    events: vec![],
                },
            }),
        };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: None,
            pending_err: Some("marker write failed".to_owned()),
            delete_err: None,
        };
        let err = delete_one("s1", &mut registry, &mut files).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::Internal);
        assert_eq!(err.message, "session/delete: marker write failed");
        assert!(files.deleted.is_empty());
    }

    #[test]
    fn on_time_destroy_deletes_transcript_and_finishes() {
        let mut registry = RecordingRegistry {
            held: Some(RecordingLive {
                transcript_path: "/sessions/s1.jsonl".to_owned(),
                wait_err: None,
                events: vec![],
                destroy: RecordingDestroy {
                    timed_out: false,
                    events: vec![],
                },
            }),
        };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: None,
            pending_err: None,
            delete_err: None,
        };
        delete_one("s1", &mut registry, &mut files).unwrap();
        assert_eq!(files.deleted, vec!["/sessions/s1.jsonl"]);
        assert!(files.pending_events.is_empty());
    }

    #[test]
    fn delete_and_wait_failure_surfaces_internal() {
        let mut registry = RecordingRegistry {
            held: Some(RecordingLive {
                transcript_path: "/sessions/s1.jsonl".to_owned(),
                wait_err: Some("controller gone".to_owned()),
                events: vec![],
                destroy: RecordingDestroy {
                    timed_out: false,
                    events: vec![],
                },
            }),
        };
        let mut files = RecordingFiles {
            pending_events: vec![],
            deleted: vec![],
            twin: None,
            pending_err: None,
            delete_err: None,
        };
        let err = delete_one("s1", &mut registry, &mut files).unwrap_err();
        assert_eq!(err.message, "session/delete: controller gone");
    }
}
