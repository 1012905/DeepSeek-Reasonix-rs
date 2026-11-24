//! TOOL-045 shared shellrun slice: classification decision table, bounded
//! collector and progress writer, login-env PATH merge, shell descriptor,
//! operative-command subject cut, and console-output decoding (tools.md §4).
//!
//! The real spawn and host probes live with the bashrun runner and the
//! OS-sandbox wave; this suite covers the pure decision tables the runner
//! feeds into.

use std::sync::{Arc, Mutex};

use reasonix_safety::sandbox::shell::ShellKind;
use reasonix_tools::builtin::shellrun::{
    Cut, Outcome, OutputCollector, ShellRunError, classify, classify_shell, command_env,
    decode_shell_output, descriptor_from_shell, display_name, login_shell_candidates,
    merge_path_lists, operative_command, parse_shell_path, platform_name, timeout_silence_note,
};
use reasonix_tools::shell::{ShellName, ShellPhase, ShellState, ShellVersion};

// ---------------------------------------------------------------------------
// Classification (§4.1)
// ---------------------------------------------------------------------------

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().copied().map(str::to_owned).collect()
}

#[test]
fn success_reads_completed_with_exit_zero_and_no_tail() {
    let out = classify(
        &argv(&["bash", "-c", "true"]),
        Outcome::Success,
        "",
        "",
        "10s",
        false,
    );
    assert_eq!(out.state, ShellState::Completed);
    assert_eq!(out.exit_code, Some(0));
    assert!(out.started);
    assert_eq!(out.output_tail, "");
    assert!(out.err.is_none());
}

#[test]
fn known_exit_code_reads_failed_execution() {
    let out = classify(
        &argv(&["bash", "-c", "exit 3"]),
        Outcome::Exit(3),
        "boom",
        "boom",
        "10s",
        false,
    );
    assert_eq!(out.state, ShellState::Failed);
    assert_eq!(out.phase, Some(ShellPhase::Execution));
    assert_eq!(out.exit_code, Some(3));
    assert_eq!(
        out.err,
        Some(ShellRunError::Exit { code: 3 }),
        "raw exit text: {:?}",
        out.err
    );
    assert_eq!(out.output_tail, "boom");
}

#[test]
fn timeout_with_no_output_names_the_silence_note() {
    let out = classify(
        &argv(&["bash", "-c", "sleep 60"]),
        Outcome::TimedOut,
        "",
        "",
        "30s",
        false,
    );
    assert_eq!(out.state, ShellState::TimedOut);
    assert_eq!(out.phase, Some(ShellPhase::Timeout));
    let err = out.err.expect("timeout carries an error");
    assert_eq!(
        err.to_string(),
        "command timed out (> 30s); it produced no output before the deadline"
    );
}

#[test]
fn timeout_with_output_keeps_no_silence_note() {
    assert_eq!(timeout_silence_note("working"), "");
    assert_eq!(
        timeout_silence_note(""),
        "; it produced no output before the deadline"
    );
    let out = classify(
        &argv(&["bash", "-c", "sleep 60"]),
        Outcome::TimedOut,
        "partial log",
        "partial log",
        "30s",
        false,
    );
    let err = out.err.expect("timeout").to_string();
    assert!(err.contains("timed out (> 30s)"));
    assert!(!err.contains("no output before the deadline"));
}

#[test]
fn cancellation_reads_the_cancellation_phase() {
    let out = classify(
        &argv(&["bash", "-c", "sleep 60"]),
        Outcome::Cancelled,
        "",
        "",
        "10s",
        false,
    );
    assert_eq!(out.state, ShellState::Cancelled);
    assert_eq!(out.phase, Some(ShellPhase::Cancellation));
    assert_eq!(out.err, Some(ShellRunError::Cancelled));
}

#[test]
fn launch_failure_and_execution_failure_split_on_started() {
    let launch = classify(
        &argv(&["missing-shell"]),
        Outcome::Failed {
            started: false,
            wait_delay_expired: false,
            message: "not found".to_owned(),
        },
        "",
        "",
        "10s",
        false,
    );
    assert_eq!(launch.state, ShellState::Failed);
    assert_eq!(launch.phase, Some(ShellPhase::Launch));
    assert!(matches!(launch.err, Some(ShellRunError::Launch { .. })));

    let execution = classify(
        &argv(&["bash", "-c", "oops"]),
        Outcome::Failed {
            started: true,
            wait_delay_expired: false,
            message: "signal".to_owned(),
        },
        "log",
        "log",
        "10s",
        false,
    );
    assert_eq!(execution.phase, Some(ShellPhase::Execution));
    assert!(matches!(
        execution.err,
        Some(ShellRunError::Execution { .. })
    ));
}

#[test]
fn preserved_wait_delay_reads_as_success() {
    let delayed = Outcome::Failed {
        started: true,
        wait_delay_expired: true,
        message: "wait delay expired".to_owned(),
    };
    let out = classify(&argv(&["bash", "-c", "true"]), delayed, "", "", "10s", true);
    assert_eq!(out.state, ShellState::Completed);
    assert!(out.err.is_none());

    let out = classify(
        &argv(&["bash", "-c", "true"]),
        Outcome::Failed {
            started: true,
            wait_delay_expired: true,
            message: "wait delay expired".to_owned(),
        },
        "",
        "",
        "10s",
        false,
    );
    assert_eq!(out.state, ShellState::Failed);
}

#[test]
fn empty_argv_is_a_launch_failure() {
    let out = classify(&[], Outcome::Success, "", "", "10s", false);
    assert_eq!(out.state, ShellState::Failed);
    assert_eq!(out.phase, Some(ShellPhase::Launch));
    assert_eq!(out.err, Some(ShellRunError::EmptyArgv));
}

// ---------------------------------------------------------------------------
// Collector and progress writer (§4.1)
// ---------------------------------------------------------------------------

fn capture_sink() -> (
    Arc<Mutex<Vec<String>>>,
    reasonix_tools::builtin::shellrun::ProgressEmit,
) {
    let captured = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink: reasonix_tools::builtin::shellrun::ProgressEmit = {
        let captured = captured.clone();
        Arc::new(move |line: &str| captured.lock().expect("sink lock").push(line.to_owned()))
    };
    (captured, sink)
}

#[test]
fn collector_keeps_combined_output_and_tail_verbatim_under_limit() {
    let (_captured, sink) = capture_sink();
    let collector = OutputCollector::new(Some(sink), "");
    collector.write(b"first line\nsecond line\n");
    assert_eq!(collector.combined_string(), "first line\nsecond line\n");
    assert_eq!(collector.tail_string(), "first line\nsecond line\n");
}

#[test]
fn progress_forwards_a_whole_ascii_line() {
    let (captured, sink) = capture_sink();
    let collector = OutputCollector::new(Some(sink), "");
    collector.write(b"hello\n");
    collector.flush_progress();
    assert_eq!(*captured.lock().expect("captured"), vec!["hello\n"]);
}

#[test]
fn split_non_ascii_stays_held_until_flush() {
    let (captured, sink) = capture_sink();
    let collector = OutputCollector::new(Some(sink), "");
    let bytes = "中文".as_bytes();
    collector.write(&bytes[..2]);
    assert!(
        captured.lock().expect("captured").is_empty(),
        "partial rune held"
    );
    collector.write(&bytes[2..]);
    collector.flush_progress();
    assert_eq!(*captured.lock().expect("captured"), vec!["中文"]);
}

#[test]
fn progress_drops_the_suppressed_whole_line() {
    let (captured, sink) = capture_sink();
    let collector = OutputCollector::new(Some(sink), "SECRET");
    collector.write(b"normal line\nSECRET token\n");
    collector.flush_progress();
    let out = captured.lock().expect("captured").join("");
    assert_eq!(out, "normal line\n");
}

#[test]
fn progress_caps_once_and_emits_the_truncation_marker() {
    let (captured, sink) = capture_sink();
    let collector = OutputCollector::new(Some(sink), "");
    collector.write(&vec![b'a'; 70_000]);
    collector.flush_progress();
    let out = captured.lock().expect("captured").join("");
    assert!(out.contains("live shell output capped at 64 KiB"));
    assert_eq!(out.matches("live shell output capped").count(), 1);
}

// ---------------------------------------------------------------------------
// Login-shell PATH merge (§4.2)
// ---------------------------------------------------------------------------

#[test]
fn merge_path_lists_dedupes_primary_first_and_skips_blanks() {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let primary = ["/a", "/b"].join(&sep.to_string());
    let secondary = ["/b", "/c", "", "/a"].join(&sep.to_string());
    assert_eq!(
        merge_path_lists(&primary, &secondary),
        ["/a", "/b", "/c"].join(&sep.to_string())
    );
}

#[test]
fn parse_shell_path_takes_the_last_marker_line() {
    let out = b"banner line\n__REASONIX_BASH_PATH__=/bin:/usr/bin\nnoise\n__REASONIX_BASH_PATH__=/opt/bin";
    assert_eq!(parse_shell_path(out, "__REASONIX_BASH_PATH__="), "/opt/bin");
    let absent = b"no marker here\n";
    assert_eq!(parse_shell_path(absent, "__REASONIX_BASH_PATH__="), "");
}

#[test]
fn command_env_merges_login_path_ahead_off_windows() {
    let env = vec!["PATH=/usr/bin:/bin".to_owned(), "HOME=/home/u".to_owned()];
    let merged = command_env(env.clone(), Some("/opt/bin"));
    if cfg!(windows) {
        assert_eq!(merged, env, "Windows passes the process env unchanged");
    } else {
        let path = merged
            .iter()
            .find(|e| e.starts_with("PATH="))
            .expect("PATH kept");
        assert_eq!(path, "PATH=/opt/bin:/usr/bin:/bin");
    }
}

#[test]
fn login_shell_candidates_order_shell_then_the_fallback_chain() {
    assert_eq!(
        login_shell_candidates(Some("/bin/zsh")),
        vec!["/bin/zsh", "/bin/zsh", "/bin/bash", "/bin/sh"]
    );
    assert_eq!(
        login_shell_candidates(None),
        vec!["/bin/zsh", "/bin/bash", "/bin/sh"]
    );
}

// ---------------------------------------------------------------------------
// Descriptor (§4.3)
// ---------------------------------------------------------------------------

#[test]
fn descriptor_classifies_powershell_pwsh_git_bash_and_bash() {
    let (pwsh, version) = classify_shell(
        ShellKind::PowerShell,
        r"C:\Program Files\PowerShell\7\pwsh.exe",
        true,
    );
    assert_eq!(pwsh, ShellName::Pwsh);
    assert_eq!(version, Some(ShellVersion::Ps7));

    let (ps5, version) = classify_shell(ShellKind::PowerShell, "powershell.exe", false);
    assert_eq!(ps5, ShellName::PowerShell);
    assert_eq!(version, Some(ShellVersion::Ps51));

    let (git_bash, version) =
        classify_shell(ShellKind::Bash, r"C:\Program Files\Git\bin\bash.exe", true);
    assert_eq!(git_bash, ShellName::GitBash);
    assert_eq!(version, None);

    let (bash, version) = classify_shell(ShellKind::Bash, "/bin/bash", true);
    assert_eq!(bash, ShellName::Bash);
    assert_eq!(version, None);
}

#[test]
fn display_names_and_platform_label_match_go() {
    assert_eq!(display_name(ShellName::GitBash), "Git Bash");
    assert_eq!(display_name(ShellName::PowerShell), "Windows PowerShell");
    assert_eq!(display_name(ShellName::Pwsh), "PowerShell 7+");
    assert_eq!(display_name(ShellName::Bash), "bash");
    assert!(["windows", "darwin", "linux"].contains(&platform_name()));
}

#[test]
fn descriptor_from_shell_carries_identity_only() {
    let shell = reasonix_safety::sandbox::shell::Shell {
        kind: ShellKind::Bash,
        path: String::from("/bin/bash"),
        launcher_args: String::new(),
        fallback: reasonix_safety::sandbox::shell::FallbackReason::None,
    };
    let exec = descriptor_from_shell(&shell);
    assert_eq!(exec.shell, Some(ShellName::Bash));
    assert!(exec.supports_and_and);
    assert!(exec.platform.is_some());
    assert!(exec.state.is_none(), "descriptor carries no run state");
}

// ---------------------------------------------------------------------------
// Operative command (§4.4)
// ---------------------------------------------------------------------------

#[test]
fn subject_drops_leading_assignments_only_when_a_command_follows() {
    let cases: &[(&str, &str, bool)] = &[
        ("go test ./...", "go test ./...", false),
        ("CGO_ENABLED=0 go build ./...", "go build ./...", true),
        (
            "GOOS=linux GOARCH=arm64 go build ./cmd/reasonix",
            "go build ./cmd/reasonix",
            true,
        ),
        (
            "PATH=$(echo $PATH | tr ':' '\\n' | grep -v emsdk | paste -sd: -) go test ./...",
            "go test ./...",
            true,
        ),
        (
            "MSG=\"a b\" printf '%s\\n' \"$MSG\"",
            "printf '%s\\n' \"$MSG\"",
            true,
        ),
        ("FOO=bar", "FOO=bar", false),
        ("export FOO=bar", "export FOO=bar", false),
        (
            "go build ./... && CGO_ENABLED=0 go test ./...",
            "go build ./... && CGO_ENABLED=0 go test ./...",
            false,
        ),
        ("go test ./... && (", "go test ./... && (", false),
        ("", "", false),
        (
            "FOO=1 go test ./... | tee out.txt",
            "go test ./... | tee out.txt",
            true,
        ),
    ];
    for (command, want, changed) in cases {
        let (subject, did_change) = operative_command(command);
        assert_eq!(did_change, *changed, "{command}");
        assert_eq!(subject, *want, "{command}");
    }
}

// ---------------------------------------------------------------------------
// Output decoding (§4.5)
// ---------------------------------------------------------------------------

#[test]
fn decode_reads_utf8_plain_and_sniffs_code_page_bytes() {
    assert_eq!(decode_shell_output(b"plain", Cut::default()), "plain");
    assert_eq!(
        decode_shell_output("中文".as_bytes(), Cut::default()),
        "中文"
    );
    // GBK bytes for 中文 decode through the sniffing path.
    let gbk = vec![0xD6, 0xD0, 0xCE, 0xC4];
    assert_eq!(decode_shell_output(&gbk, Cut::default()), "中文");
}

#[test]
fn decode_trims_split_runes_at_cut_edges() {
    let bytes = "中文".as_bytes();
    // A partial trailing rune on a tail cut.
    assert_eq!(
        decode_shell_output(
            &bytes[..2],
            Cut {
                head: false,
                tail: true
            }
        ),
        ""
    );
    // A partial leading rune on a head cut.
    assert_eq!(
        decode_shell_output(
            &bytes[1..2],
            Cut {
                head: true,
                tail: false
            }
        ),
        ""
    );
}
