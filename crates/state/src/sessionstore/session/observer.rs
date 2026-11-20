//! Non-blocking persist-observer projection hooks (Go:
//! session_persist_observer.go).
//!
//! The host projects session updates into its own views after each
//! commit. The observer is intentionally lossy: enqueue must never
//! block the save path, and events carry only the cheap known state.

use std::sync::Arc;

use super::Session;

/// Event handed to a registered observer after a save commits (Go:
/// `SessionPersistEvent`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPersistEvent {
    pub path: String,
    pub revision: i64,
    pub content_digest: String,
    pub message_count: usize,
    pub append_from: i64,
    pub rewrite: bool,
    pub removed: bool,
}

/// Post-commit projection hook (Go: `SessionPersistObserver`).
/// `enqueue_session_persist` must not block.
pub trait SessionPersistObserver: Send + Sync {
    fn enqueue_session_persist(&self, event: SessionPersistEvent) -> bool;
}

impl Session {
    /// Registers a session-local observer (Go: `SetPersistObserver`).
    pub fn set_persist_observer(&self, observer: Arc<dyn SessionPersistObserver>) {
        let mut st = self.write();
        st.persist_observer = Some(observer);
    }

    /// Notifies the observer after a save commits (Go:
    /// `notifyPersisted`). Unknown-state sessions still notify
    /// with zeroed revision/digest so removals stay observable.
    pub(crate) fn notify_persisted(&self, path: &str, rewrite: bool, append_from: i64) {
        let (observer, message_count) = {
            let st = self.read();
            let Some(observer) = st.persist_observer.clone() else {
                return;
            };
            (observer, st.messages.len())
        };
        let mut event = SessionPersistEvent {
            path: path.to_string(),
            message_count,
            append_from,
            rewrite,
            ..Default::default()
        };
        if let Some(state) = self.persisted_state(path) {
            event.revision = state.revision;
            event.content_digest = state.digest_hex;
        }
        observer.enqueue_session_persist(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::provider::{Message, Role};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<SessionPersistEvent>>,
    }

    impl SessionPersistObserver for Recorder {
        fn enqueue_session_persist(&self, event: SessionPersistEvent) -> bool {
            self.events.lock().unwrap().push(event);
            true
        }
    }

    const D: [u8; 32] = [9u8; 32];

    #[test]
    fn observer_receives_known_state_event() {
        let s = Session::new("");
        s.add(Message {
            role: Role::USER,
            content: "hi".to_string(),
            ..Default::default()
        });
        let recorder = Arc::new(Recorder::default());
        s.set_persist_observer(recorder.clone());

        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl").to_string_lossy().into_owned();
        s.mark_persisted(&p, &D, s.transcript_version(), 12, 0);
        s.notify_persisted(&p, false, 3);

        let events = recorder.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, p);
        assert_eq!(events[0].revision, 12);
        assert_eq!(
            events[0].content_digest,
            crate::sessionstore::eventlog::writer_id::digest_string(&D)
        );
        assert_eq!(events[0].message_count, 1);
        assert_eq!(events[0].append_from, 3);
        assert!(!events[0].rewrite);
        assert!(!events[0].removed);
    }

    #[test]
    fn observer_unknown_state_defaults() {
        let s = Session::new("");
        let recorder = Arc::new(Recorder::default());
        s.set_persist_observer(recorder.clone());

        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir
            .path()
            .join("never.jsonl")
            .to_string_lossy()
            .into_owned();
        s.notify_persisted(&p, true, -1);

        let events = recorder.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].revision, 0);
        assert_eq!(events[0].content_digest, "");
        assert_eq!(events[0].message_count, 0);
        assert!(events[0].rewrite);
    }

    #[test]
    fn no_observer_is_a_noop() {
        let s = Session::new("");
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        s.notify_persisted(&dir.path().join("x.jsonl").to_string_lossy(), false, 0);
    }
}
