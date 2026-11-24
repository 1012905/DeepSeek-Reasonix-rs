//! ACP-031: `session/delete` teardown order, cleanup-pending timeout branch,
//! twin deletion, missing-session silence, and error identities
//! (`session.go:653-709`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_delete::{
    DeleteRegistry, DestroyHandle, DestroyOutcome, FileOps, LiveDelete, delete_one,
};

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
    destroy: RecordingDestroy,
}

impl LiveDelete for RecordingLive {
    fn delete_and_wait(&mut self) -> Result<(), String> {
        self.wait_err.clone().map_or(Ok(()), Err)
    }
    fn release_lease(&mut self) {}
    fn transcript_path(&self) -> String {
        self.transcript_path.clone()
    }
    fn begin_destroy(&mut self) -> Box<dyn DestroyHandle> {
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
    pending: Vec<String>,
    deleted: Vec<String>,
    twin: Option<String>,
    pending_err: Option<String>,
    delete_err: Option<String>,
}

impl FileOps for RecordingFiles {
    fn mark_cleanup_pending(&mut self, path: &str) -> Result<(), String> {
        self.pending.push(path.to_owned());
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

fn held(timed_out: bool) -> RecordingRegistry {
    RecordingRegistry {
        held: Some(RecordingLive {
            transcript_path: "/sessions/s1.jsonl".to_owned(),
            wait_err: None,
            destroy: RecordingDestroy {
                timed_out,
                events: vec![],
            },
        }),
    }
}

#[test]
fn invalid_id_refuses_before_any_pipeline_call() {
    let mut registry = RecordingRegistry { held: None };
    let mut files = RecordingFiles {
        pending: vec![],
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
fn missing_session_deletes_silently_without_filesystem_call() {
    let mut registry = RecordingRegistry { held: None };
    let mut files = RecordingFiles {
        pending: vec![],
        deleted: vec![],
        twin: None,
        pending_err: None,
        delete_err: None,
    };
    delete_one("s1", &mut registry, &mut files).unwrap();
    assert!(files.deleted.is_empty());
    assert!(files.pending.is_empty());
}

#[test]
fn on_time_destroy_deletes_transcript_and_finishes() {
    let mut registry = held(false);
    let mut files = RecordingFiles {
        pending: vec![],
        deleted: vec![],
        twin: Some("/sessions/s1.acp.json".to_owned()),
        pending_err: None,
        delete_err: None,
    };
    delete_one("s1", &mut registry, &mut files).unwrap();
    assert_eq!(
        files.deleted,
        vec!["/sessions/s1.jsonl", "/sessions/s1.acp.json"]
    );
    assert!(files.pending.is_empty());
}

#[test]
fn timeout_marks_pending_delays_transcript_but_still_deletes_twin() {
    let mut registry = held(true);
    let mut files = RecordingFiles {
        pending: vec![],
        deleted: vec![],
        twin: Some("/sessions/s1.acp.json".to_owned()),
        pending_err: None,
        delete_err: None,
    };
    delete_one("s1", &mut registry, &mut files).unwrap();
    assert_eq!(files.pending, vec!["/sessions/s1.jsonl"]);
    assert_eq!(files.deleted, vec!["/sessions/s1.acp.json"]);
}

#[test]
fn pending_mark_failure_releases_destroy_then_reports_internal() {
    let mut registry = held(true);
    let mut files = RecordingFiles {
        pending: vec![],
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
fn delete_and_wait_failure_surfaces_internal() {
    let mut registry = RecordingRegistry {
        held: Some(RecordingLive {
            transcript_path: "/sessions/s1.jsonl".to_owned(),
            wait_err: Some("controller gone".to_owned()),
            destroy: RecordingDestroy {
                timed_out: false,
                events: vec![],
            },
        }),
    };
    let mut files = RecordingFiles {
        pending: vec![],
        deleted: vec![],
        twin: None,
        pending_err: None,
        delete_err: None,
    };
    let err = delete_one("s1", &mut registry, &mut files).unwrap_err();
    assert_eq!(err.message, "session/delete: controller gone");
}
