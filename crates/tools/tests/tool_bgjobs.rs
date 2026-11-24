//! TOOL-026 background-job tools (Go `internal/tools/builtin/bgjobs.go`):
//! `bash_output`, `kill_shell`, `wait`. The tool shells run against a stub
//! job-manager seam; the real session-scoped table, cursors, watch channels,
//! and completion drain are the A3 jobs slice.
//!
//! Every Go rejection branch has a row here: no manager, unknown job, missing
//! `job_id`, invalid filter, invalid wait pattern, missing job context, and
//! failed-job failure carry. Messages are compared verbatim.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reasonix_tools::abi::{CallContext, ContextualReasoner, ContextualTool, SequentialTool, Tool};
use reasonix_tools::builtin::bgjobs::{
    BashOutputTool, BgJobError, JobManager, JobProgress, JobResult, JobStatus, JobView,
    KillShellTool, WaitJobTool, WaitOutcome, byte_count, clamp_wait_seconds, filter_lines,
    jobs_failed, list_running_jobs, one_line, resume_note, still_running,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct StubManager {
    session: String,
    outputs: HashMap<String, (String, JobStatus, bool)>,
    views: Vec<JobView>,
    kills: HashMap<String, bool>,
    resume: bool,
    wait: (Vec<JobResult>, WaitOutcome),
}

impl JobManager for StubManager {
    fn session(&self) -> String {
        self.session.clone()
    }

    fn output_for_session(&self, _session: &str, job_id: &str) -> (String, JobStatus, bool) {
        self.outputs
            .get(job_id)
            .cloned()
            .unwrap_or((String::new(), JobStatus::Running, false))
    }

    fn running_for_session(&self, _session: &str) -> Vec<JobView> {
        self.views.clone()
    }

    fn kill_for_session(&self, _session: &str, job_id: &str) -> bool {
        self.kills.get(job_id).copied().unwrap_or(false)
    }

    fn resumes_on_completion(&self, _session: &str) -> bool {
        self.resume
    }

    fn wait_for_session(
        &self,
        _session: &str,
        _job_ids: &[String],
        _timeout: Duration,
        _match_pattern: Option<&str>,
    ) -> (Vec<JobResult>, WaitOutcome) {
        self.wait.clone()
    }
}

fn bind(stub: StubManager) -> CallContext {
    let port: Arc<dyn JobManager> = Arc::new(stub);
    CallContext::new().with_value(Arc::new(port))
}

fn running_view(id: &str, kind: &str, label: &str, started_ms: i64) -> JobView {
    JobView {
        id: id.to_owned(),
        kind: kind.to_owned(),
        label: label.to_owned(),
        started_at_ms: started_ms,
    }
}

#[test]
fn the_tool_cards_match_the_go_names_descriptions_and_schemas() {
    let output = BashOutputTool;
    assert_eq!(output.name(), "bash_output");
    assert_eq!(
        output.description(),
        "Read new output from a background job started with bash(run_in_background=true) or \
         task(run_in_background=true). Returns the output produced since the last bash_output \
         call for that job, plus its status (running/done/failed/killed). Does not block. Omit \
         job_id to list the jobs still running, with their kind, label and elapsed time."
    );
    assert_eq!(
        output.schema(),
        json!({
            "type": "object",
            "properties": {
                "job_id": {"type": "string", "description": "The background job id (e.g. \"bash-1\") returned when it was started."},
                "filter": {"type": "string", "description": "Optional regular expression; only matching lines of the new output are returned."}
            }
        })
    );
    assert!(output.read_only());

    let kill = KillShellTool;
    assert_eq!(kill.name(), "kill_shell");
    assert_eq!(
        kill.description(),
        "Terminate a running background job (bash or task) started with run_in_background. A \
         no-op if the job has already finished or the id is unknown."
    );
    assert_eq!(
        kill.schema(),
        json!({
            "type": "object",
            "properties": {
                "job_id": {"type": "string", "description": "The background job id to terminate (e.g. \"bash-1\")."}
            },
            "required": ["job_id"]
        })
    );
    assert!(!kill.read_only());

    let wait = WaitJobTool;
    assert_eq!(wait.name(), "wait");
    assert_eq!(
        wait.description(),
        "Block until background jobs finish, then return each job's status and final \
         output/answer. Use to collect the result of a task(run_in_background) or \
         bash(run_in_background) before continuing. Omit job_ids to wait for every running job."
    );
    assert_eq!(
        wait.schema(),
        json!({
            "type": "object",
            "properties": {
                "job_ids": {"type": "array", "items": {"type": "string"}, "description": "Background job ids to wait for. Omit to wait for every currently-running job."},
                "timeout_seconds": {"type": "integer", "description": "Seconds to block before returning. Every wait is capped at 300, including when this is omitted. On timeout the result carries progress; read it, then wait again.", "minimum": 1, "maximum": 300},
                "until_output_matches": {"type": "string", "description": "Optional regular expression (RE2). Return early as soon as a job writes something matching it. Use it to wait once for the line you actually care about — a completion marker, or an error — instead of polling to check."}
            }
        })
    );
    assert!(wait.read_only());
}

#[test]
fn the_tools_hide_without_a_manager_and_refuse_with_the_typed_code() {
    for tool in [
        &BashOutputTool as &dyn Tool,
        &KillShellTool as &dyn Tool,
        &WaitJobTool as &dyn Tool,
    ] {
        let cap = tool.capabilities();
        let contextual = cap
            .get::<dyn ContextualTool>()
            .expect("contextual capability");
        assert!(!contextual.provider_visible(&CallContext::new()));
        let reasoner = cap
            .get::<dyn ContextualReasoner>()
            .expect("reasoner capability");
        let refusal = reasoner.unavailable(&CallContext::new());
        assert_eq!(refusal.code, "jobs.no_job_context");
        assert_eq!(
            refusal.message,
            "background jobs are not available in this context"
        );
    }
}

#[test]
fn the_sequential_declaration_matches_the_go_ordering() {
    let ctx = CallContext::new();
    let args = json!({});
    let output_cap = BashOutputTool.capabilities();
    assert!(output_cap.get::<dyn SequentialTool>().is_some());
    let wait_cap = WaitJobTool.capabilities();
    assert!(wait_cap.get::<dyn SequentialTool>().is_some());
    let kill_cap = KillShellTool.capabilities();
    assert!(kill_cap.get::<dyn SequentialTool>().is_none());
    assert!(
        output_cap
            .get::<dyn SequentialTool>()
            .unwrap()
            .sequential(&ctx, &args)
    );
    assert!(
        wait_cap
            .get::<dyn SequentialTool>()
            .unwrap()
            .sequential(&ctx, &args)
    );
}

#[test]
fn execute_without_a_manager_fails_closed_on_all_three() {
    let ctx = CallContext::new();
    let cases: [(&dyn Tool, Value); 3] = [
        (&BashOutputTool, json!({"job_id": "bash-1"})),
        (&KillShellTool, json!({"job_id": "bash-1"})),
        (&WaitJobTool, json!({"job_ids": ["bash-1"]})),
    ];
    for (tool, args) in cases {
        let err = tool.execute(&ctx, &args).unwrap_err();
        assert_eq!(
            err.to_string(),
            "background jobs are not available in this context"
        );
    }
}

#[test]
fn bash_output_without_an_id_lists_the_running_jobs() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: vec![running_view(
            "bash-1",
            "bash",
            "echo hello",
            1_700_000_000_000,
        )],
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let out = BashOutputTool
        .execute(&bind(stub), &json!({}))
        .expect("listing");
    assert!(out.starts_with("Running background jobs (1):"));
    assert!(out.contains("- bash-1  kind=bash"));
    assert!(out.contains("echo hello"));

    let empty = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let out = BashOutputTool
        .execute(&bind(empty), &json!({}))
        .expect("empty listing");
    assert_eq!(out, "No background jobs are running.");
}

#[test]
fn bash_output_reads_one_running_job_with_header_and_resume_note() {
    let mut outputs = HashMap::new();
    outputs.insert(
        "bash-1".to_owned(),
        ("hello\nworld".to_owned(), JobStatus::Running, true),
    );
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs,
        views: Vec::new(),
        kills: HashMap::new(),
        resume: true,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let out = BashOutputTool
        .execute(&bind(stub), &json!({"job_id": "bash-1"}))
        .expect("output");
    assert!(out.starts_with("[bash-1] running"));
    assert!(out.contains("hello\nworld"));
    assert!(out.contains("When a running job finishes, a new turn starts"));
}

#[test]
fn bash_output_empty_output_answers_the_no_new_output_sentence() {
    let mut outputs = HashMap::new();
    outputs.insert("bash-1".to_owned(), (String::new(), JobStatus::Done, true));
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs,
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let out = BashOutputTool
        .execute(&bind(stub), &json!({"job_id": "bash-1"}))
        .expect("output");
    assert!(out.starts_with("[bash-1] done"));
    assert!(out.contains("(no new output)"));
}

#[test]
fn bash_output_unknown_job_is_a_plain_no_job_error() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let err = BashOutputTool
        .execute(&bind(stub), &json!({"job_id": "bash-9"}))
        .unwrap_err();
    assert_eq!(err.to_string(), "no background job \"bash-9\"");
}

#[test]
fn bash_output_a_bad_filter_reports_the_regexp_error() {
    let mut outputs = HashMap::new();
    outputs.insert(
        "bash-1".to_owned(),
        ("line one\nline two".to_owned(), JobStatus::Running, true),
    );
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs,
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let err = BashOutputTool
        .execute(&bind(stub), &json!({"job_id": "bash-1", "filter": "["}))
        .unwrap_err();
    assert!(err.to_string().starts_with("invalid filter regexp:"));
}

#[test]
fn bash_output_a_failed_job_names_the_id_and_keeps_the_output() {
    let mut outputs = HashMap::new();
    outputs.insert(
        "bash-2".to_owned(),
        ("boom".to_owned(), JobStatus::Failed, true),
    );
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs,
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let err = BashOutputTool
        .execute(&bind(stub), &json!({"job_id": "bash-2"}))
        .unwrap_err();
    let err = err.downcast_ref::<BgJobError>().expect("bgjob error");
    assert_eq!(err.to_string(), "background job failed: bash-2");
    // The collected output rides alongside the failure, as Go's (string, error).
    assert!(err.output().contains("boom"));
}

#[test]
fn kill_shell_requires_an_id_then_reports_killed_or_noop() {
    let ctx = CallContext::new();
    let err = KillShellTool.execute(&ctx, &json!({})).unwrap_err();
    assert_eq!(err.to_string(), "job_id is required");

    let mut kills = HashMap::new();
    kills.insert("bash-1".to_owned(), true);
    let mut kills_noop = HashMap::new();
    kills_noop.insert("bash-2".to_owned(), false);

    let killed = KillShellTool
        .execute(
            &bind(StubManager {
                session: "s-1".to_owned(),
                outputs: HashMap::new(),
                views: Vec::new(),
                kills,
                resume: false,
                wait: (Vec::new(), WaitOutcome::Finished),
            }),
            &json!({"job_id": "bash-1"}),
        )
        .expect("killed");
    assert_eq!(killed, "Killed background job \"bash-1\".");

    let noop = KillShellTool
        .execute(
            &bind(StubManager {
                session: "s-1".to_owned(),
                outputs: HashMap::new(),
                views: Vec::new(),
                kills: kills_noop,
                resume: false,
                wait: (Vec::new(), WaitOutcome::Finished),
            }),
            &json!({"job_id": "bash-2"}),
        )
        .expect("noop");
    assert_eq!(
        noop,
        "Background job \"bash-2\" was not running (already finished or unknown)."
    );
}

#[test]
fn wait_with_no_jobs_answers_the_empty_sentence() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let out = WaitJobTool
        .execute(&bind(stub), &json!({"job_ids": []}))
        .expect("wait");
    assert_eq!(out, "No background jobs to wait for.");
}

fn running_result(id: &str) -> JobResult {
    JobResult {
        id: id.to_owned(),
        kind: "bash".to_owned(),
        label: String::new(),
        status: JobStatus::Running,
        output: String::new(),
        progress: None,
    }
}

#[test]
fn wait_reports_the_ceiling_when_it_times_out() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (vec![running_result("bash-1")], WaitOutcome::TimedOut),
    };
    let out = WaitJobTool.execute(&bind(stub), &json!({})).expect("wait");
    assert!(out.starts_with("Waited 300ds, the ceiling for one wait; ask again to keep waiting."));
}

#[test]
fn wait_reports_the_matched_early_return() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (vec![running_result("bash-1")], WaitOutcome::Matched),
    };
    let out = WaitJobTool
        .execute(&bind(stub), &json!({"until_output_matches": "done"}))
        .expect("wait");
    assert!(
        out.starts_with("Returned early: a job wrote something matching until_output_matches.")
    );
}

#[test]
fn wait_renders_each_result_with_status_header_and_output() {
    let results = vec![JobResult {
        id: "bash-1".to_owned(),
        kind: "bash".to_owned(),
        label: "echo hello".to_owned(),
        status: JobStatus::Done,
        output: "hello".to_owned(),
        progress: None,
    }];
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (results, WaitOutcome::Finished),
    };
    let out = WaitJobTool
        .execute(&bind(stub), &json!({"job_ids": ["bash-1"]}))
        .expect("wait");
    assert!(out.contains("[bash-1 (echo hello)] done"));
    assert!(out.contains("hello"));
}

#[test]
fn wait_a_bad_until_pattern_reports_the_regexp_error() {
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let err = WaitJobTool
        .execute(&bind(stub), &json!({"until_output_matches": "("}))
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("until_output_matches is not a valid regular expression:")
    );
}

#[test]
fn wait_a_failed_job_names_the_id() {
    let results = vec![JobResult {
        id: "bash-7".to_owned(),
        kind: "bash".to_owned(),
        label: String::new(),
        status: JobStatus::Failed,
        output: "exit 1".to_owned(),
        progress: None,
    }];
    let stub = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (results, WaitOutcome::Finished),
    };
    let err = WaitJobTool
        .execute(&bind(stub), &json!({"job_ids": ["bash-7"]}))
        .unwrap_err();
    assert_eq!(err.to_string(), "background job failed: bash-7");
}

#[test]
fn byte_count_follows_the_go_unit_ladder() {
    assert_eq!(byte_count(1023), "1023 B");
    assert_eq!(byte_count(1024), "1.0 KB");
    assert_eq!(byte_count(1_048_524), "1023.9 KB");
    assert_eq!(byte_count(1_048_525), "1.0 MB");
    assert_eq!(byte_count(1 << 20), "1.0 MB");
}

#[test]
fn one_line_folds_whitespace_and_truncates_runes() {
    assert_eq!(one_line("  a   b\tc  ", 80), "a b c");
    let long = "x".repeat(100);
    let got = one_line(&long, 80);
    assert!(got.ends_with("..."));
    assert_eq!(got.chars().count(), 83);
}

#[test]
fn still_render_progress_branches_match_the_go_phrasing() {
    assert_eq!(still_running(None), "");
    let output = JobProgress {
        running_ms: 65_000,
        produced: 2_048,
        delta: 512,
        steps: 0,
        step_delta: 0,
    };
    let text = still_running(Some(&output));
    assert!(text.contains("running 1m5s"));
    assert!(text.contains("2.0 KB of output, 512 B of it while waiting"));

    let steps = JobProgress {
        running_ms: 12_000,
        produced: 0,
        delta: 0,
        steps: 7,
        step_delta: 2,
    };
    let text = still_running(Some(&steps));
    assert!(text.contains("no output text (it does not write here)"));
    assert!(text.contains("7 step(s) reported, 2 while waiting"));

    let quiet = JobProgress::default();
    let text = still_running(Some(&quiet));
    assert!(text.contains("no output text and no steps reported yet"));
}

#[test]
fn clamp_wait_seconds_applies_the_300_ceiling_and_reports_it() {
    assert_eq!(clamp_wait_seconds(0), (300, true));
    assert_eq!(clamp_wait_seconds(-1), (300, true));
    assert_eq!(clamp_wait_seconds(301), (300, true));
    assert_eq!(clamp_wait_seconds(120), (120, false));
    assert_eq!(clamp_wait_seconds(300), (300, false));
}

#[test]
fn filter_lines_keeps_only_matching_lines() {
    let text = "alpha\nbeta-1\ngamma\nbeta-2";
    let kept = filter_lines(text, "beta").expect("regexp");
    assert_eq!(kept, "beta-1\nbeta-2");
    assert!(filter_lines(text, "[").is_err());
}

#[test]
fn listing_counts_hidden_jobs_beyond_the_fifty_ceiling() {
    let now = 1_700_000_100_000;
    let one = list_running_jobs(
        &[running_view("bash-1", "bash", "label", now - 10_000)],
        now,
    );
    assert!(one.contains("Running background jobs (1):"));
    assert!(!one.contains("more running job"));

    let many: Vec<JobView> = (0..55)
        .map(|i| running_view(&format!("bash-{i}"), "bash", "", now - 10_000))
        .collect();
    let listed = list_running_jobs(&many, now);
    assert!(listed.contains("Running background jobs (55):"));
    assert!(listed.contains("5 more running job(s) not listed."));
}

#[test]
fn resume_note_absent_when_the_manager_does_not_resume() {
    let noop = StubManager {
        session: "s-1".to_owned(),
        outputs: HashMap::new(),
        views: Vec::new(),
        kills: HashMap::new(),
        resume: false,
        wait: (Vec::new(), WaitOutcome::Finished),
    };
    let resumes = StubManager {
        resume: true,
        ..noop.clone()
    };
    assert!(resume_note(&noop, "s-1").is_empty());
    assert!(resume_note(&resumes, "s-1").contains("the job keeps running and is not abandoned"));
}

#[test]
fn jobs_failed_collects_only_failed_ids() {
    let done = JobResult {
        id: "a".to_owned(),
        kind: String::new(),
        label: String::new(),
        status: JobStatus::Done,
        output: String::new(),
        progress: None,
    };
    let failed = JobResult {
        id: "b".to_owned(),
        kind: String::new(),
        label: String::new(),
        status: JobStatus::Failed,
        output: String::new(),
        progress: None,
    };
    assert!(jobs_failed(std::slice::from_ref(&done)).is_none());
    let err = jobs_failed(&[done, failed]).expect("failure");
    assert_eq!(err.to_string(), "background job failed: b");
}
