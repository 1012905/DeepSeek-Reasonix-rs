//! Inline tests for the manager/seam layer in [`super`].

#![forbid(unsafe_code)]

use super::*;

struct Recorder {
    starts: Mutex<Vec<String>>,
    done: Mutex<Vec<(String, Status)>>,
}

impl TaskRecorder for Recorder {
    fn record_start(&self, id: &str, _kind: &str, _label: &str) {
        self.starts.lock().expect("starts lock").push(id.to_owned());
    }
    fn record_done(&self, id: &str, status: Status, _err: Option<&str>) {
        self.done
            .lock()
            .expect("done lock")
            .push((id.to_owned(), status));
    }
}

#[test]
fn status_maps_to_strings_cause_and_process_class() {
    assert_eq!(Status::Running.as_str(), "running");
    assert_eq!(Status::from_str("done"), Some(Status::Done));
    assert_eq!(
        Status::Interrupted.cause(),
        " — the session that ran it ended before it finished (a restart or close), not a kill; its output up to then is kept"
    );
    assert_eq!(Status::Done.cause(), "");
    assert_eq!(Status::Killed.process_class(), ("stopped", true));
    assert!(!Status::Running.is_terminal());
    assert!(Status::Failed.is_terminal());
}

#[test]
fn start_registers_a_running_job_and_records_the_start() {
    let manager = Arc::new(Manager::new(None));
    let recorder = Arc::new(Recorder {
        starts: Mutex::new(Vec::new()),
        done: Mutex::new(Vec::new()),
    });
    manager.set_task_recorder(Some(recorder.clone()));
    let job = manager
        .start("", "bash", "build", Presentation::default(), 1_000)
        .expect("valid start");
    assert_eq!(job.id, "bash-1");
    assert_eq!(job.status(), Status::Running);
    assert!(!job.is_done());
    assert_eq!(recorder.starts.lock().expect("starts lock").len(), 1);
}

#[test]
fn an_invalid_kind_registers_a_visible_failed_placeholder() {
    let manager = Arc::new(Manager::new(None));
    let err = validate_path_segment("", "kind").expect_err("empty kind");
    let job = manager.register_invalid("", "", &err, 1_000);
    assert!(job.id.starts_with("invalid-"));
    assert_eq!(job.status(), Status::Failed);
    assert!(job.is_done());
    let (text, status) = job.take_output();
    assert_eq!(status, Status::Failed);
    assert!(text.contains("job artifact incomplete: jobs: kind must not be empty"));
}

#[test]
fn kill_flips_only_a_running_job_synchronously() {
    let manager = Arc::new(Manager::new(None));
    let job = manager
        .start("", "bash", "", Presentation::default(), 1_000)
        .unwrap();
    assert!(manager.kill_for_session("", &job.id));
    assert_eq!(job.status(), Status::Killed);
    assert!(!manager.kill_for_session("", &job.id), "already killed");
}

#[test]
fn completion_queues_before_flip_and_closes_done_before_publish() {
    let manager = Arc::new(Manager::new(None));
    let job = manager
        .start("", "bash", "", Presentation::default(), 1_000)
        .unwrap();
    manager.complete_job(&job, Status::Done, None, None, String::new(), 2_000);
    assert_eq!(job.status(), Status::Done);
    assert!(job.is_done());
    let drained = manager.drain_completions("");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].event_id, completion_event_id(&job.id));
    assert_eq!(drained[0].text, format!("{} — done", job.id));
}

#[test]
fn context_binding_round_trips_and_masking_hides_the_manager() {
    let manager = Arc::new(Manager::new(None));
    let ctx = bind_manager(manager.clone());
    assert!(manager_from(&ctx).is_some());
    let masked = without_manager();
    assert!(manager_from(&masked).is_none());
}

#[test]
fn job_key_joins_trimmed_parts_with_nul() {
    assert_eq!(job_key(" s1 ", " j1 "), "s1\x00j1");
}
