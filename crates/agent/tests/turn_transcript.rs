//! Durability: the admission saver seam and the transcript bytes the
//! sessionstore writes for a finished turn (Go: `AdmissionSaver` + the
//! controller's save points).

mod support;

use std::sync::Arc;

use reasonix_agent::seams::TranscriptSaver;
use reasonix_state::sessionstore::Session;

use support::{
    Fixture, chunked_sse, fixture, registry_with_readers, serve_responses, text_frame, usage_frame,
};

/// The host's admission save: makes the user turn durable the moment it
/// lands (Go: the controller wiring of `SaveAdmittedMessage`).
struct AdmissionSave {
    session: Arc<Session>,
    path: String,
}

impl TranscriptSaver for AdmissionSave {
    fn save_admitted_message(&self) {
        self.session.save(&self.path).expect("admission save");
    }
}

#[tokio::test]
async fn the_admitted_user_turn_is_durable_before_the_answer() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("session.jsonl");
    let path = path.to_string_lossy().into_owned();

    // The endpoint never answers: the turn fails, but the user message is
    // already durable (Go: a process killed mid-turn still has the prompt).
    let base = serve_responses(vec![
        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ])
    .await;
    let f = fixture(&base, registry_with_readers()).await;
    let saver: Arc<dyn TranscriptSaver> = Arc::new(AdmissionSave {
        session: f.session.clone(),
        path: path.clone(),
    });
    f.agent.set_transcript_saver(saver);

    let _ = f.agent.run("the durable prompt").await;

    let bytes = std::fs::read(&path).expect("transcript bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("the durable prompt"), "{text}");
}

#[tokio::test]
async fn a_finished_turns_messages_are_all_in_the_transcript_bytes() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("session.jsonl");
    let path = path.to_string_lossy().into_owned();

    let base = serve_responses(vec![chunked_sse(&[
        &text_frame("the final answer"),
        &usage_frame(7, 3, "stop"),
        "[DONE]",
    ])])
    .await;
    let Fixture { agent, session, .. } = fixture(&base, registry_with_readers()).await;
    agent.run("a finished turn").await.expect("turn");

    // The host saves at its own point after the turn (Go: the controller).
    session.save(&path).expect("save");

    let bytes = std::fs::read(&path).expect("transcript bytes");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("a finished turn"), "{text}");
    assert!(text.contains("the final answer"), "{text}");
    assert!(text.contains("relay/m"), "{text}");
}
