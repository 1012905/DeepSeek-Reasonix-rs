//! TOOL-025 bash satellite decisions and refusal identities (Go
//! `internal/tools/builtin/bash.go`, `bash_delete.go`, `bash_gitmeta.go`,
//! `internal/safety/sandbox/sandbox.go`, `win_faults.go`).
//!
//! The pure decisions, frozen messages, and typed codes are contract: every
//! rejection branch the Go side names has a row here, with the sentence
//! compared verbatim. Spawn, the job table, leases, and sandbox fault
//! detection land with the later host-machinery slices; this file does not
//! exercise a real subprocess.

#![forbid(unsafe_code)]

use reasonix_safety::sandbox::shell::ShellKind;
use reasonix_tools::builtin::bash_tool::*;

#[test]
fn start_message_names_the_id_in_each_call_site() {
    let msg = background_start_message("bash-1");
    assert_eq!(
        msg,
        "Started background job \"bash-1\". It keeps running across turns; read new output with \
         bash_output(job_id=\"bash-1\"), wait for it with wait, or stop it with \
         kill_shell(job_id=\"bash-1\")."
    );
}

#[test]
fn background_unavailable_sentence_is_verbatim() {
    assert_eq!(
        BACKGROUND_NOT_AVAILABLE,
        "background execution is not available in this context"
    );
}

#[test]
fn delete_guard_refusals_carry_the_prefix_and_verbatim_message() {
    let cases: &[(&str, &str, &str)] = &[
        (
            CODE_DESTRUCTIVE_TARGET,
            MESSAGE_DESTRUCTIVE_TARGET,
            "target scope",
        ),
        (
            CODE_ANALYSIS_UNKNOWN,
            MESSAGE_ANALYSIS_INTERPRETER_PAYLOAD,
            "interpreter payload",
        ),
        (
            CODE_ANALYSIS_UNKNOWN,
            MESSAGE_ANALYSIS_NESTED_EXTENT,
            "nested extent",
        ),
        (
            CODE_ANALYSIS_UNKNOWN,
            MESSAGE_ANALYSIS_DYNAMIC_EXECUTABLE,
            "dynamic executable",
        ),
        (CODE_DELETE_SEQUENCE, MESSAGE_DELETE_SEQUENCE, "sequence"),
        (
            CODE_DELETE_NONLITERAL,
            MESSAGE_DELETE_NONLITERAL,
            "nonliteral",
        ),
        (CODE_DELETE_OPTION, MESSAGE_DELETE_OPTION, "option"),
        (
            CODE_PARSER_UNAVAILABLE,
            MESSAGE_PARSER_UNAVAILABLE,
            "parser unavailable",
        ),
        (
            CODE_PARSER_TIMEOUT,
            MESSAGE_PARSER_TIMEOUT,
            "parser timeout",
        ),
    ];
    for (code, message, label) in cases {
        let refusal = delete_refusal(code, message);
        assert_eq!(&refusal.code, code, "{label}");
        assert!(
            refusal.message.starts_with(DELETE_REFUSAL_PREFIX),
            "{label} missing the shared prefix"
        );
        assert_eq!(
            refusal.message,
            format!("{DELETE_REFUSAL_PREFIX}{message}"),
            "{label} message body"
        );
    }
}

#[test]
fn syntax_error_refusal_interpolates_the_parser_error_verbatim() {
    assert_eq!(
        syntax_error_message("near \";\""),
        "Fix the shell syntax and retry: near \";\""
    );
    let refusal = delete_refusal(CODE_SYNTAX_ERROR, &syntax_error_message("unexpected token"));
    assert_eq!(refusal.code, CODE_SYNTAX_ERROR);
    assert!(
        refusal
            .message
            .contains("Fix the shell syntax and retry: unexpected token")
    );
}

#[test]
fn parser_failure_codes_are_the_go_identities() {
    assert_eq!(CODE_PARSER_UNAVAILABLE, "shell.parser_unavailable");
    assert_eq!(CODE_PARSER_TIMEOUT, "shell.parser_timeout");
    assert_eq!(
        MESSAGE_PARSER_UNAVAILABLE,
        "The host shell parser is unavailable; retry when the host parser is restored."
    );
    assert_eq!(
        MESSAGE_PARSER_TIMEOUT,
        "The host shell parser timed out or was canceled; retry the call."
    );
}

#[test]
fn sandbox_unavailable_platform_defaults_match_go() {
    assert_eq!(default_unavailable_code("linux"), CODE_NO_BUBBLEWRAP);
    assert_eq!(default_unavailable_code("darwin"), CODE_NO_SANDBOX_EXEC);
    assert_eq!(default_unavailable_code("windows"), CODE_WIN_UNSUPPORTED);
    assert_eq!(
        default_unavailable_code("freebsd"),
        CODE_UNSUPPORTED_PLATFORM
    );
}

#[test]
fn sandbox_unavailable_fault_codes_are_pinned_by_platform_group() {
    let linux = [
        CODE_BWRAP_NETWORK_DENIED,
        CODE_BWRAP_NAMESPACE_DENIED,
        CODE_BWRAP_UNRUNNABLE,
        CODE_NO_BUBBLEWRAP,
    ];
    assert!(linux.iter().all(|c| c.starts_with("sandbox.")));
    assert_eq!(CODE_NO_BUBBLEWRAP, "sandbox.no_bubblewrap");
    assert_eq!(CODE_NO_SANDBOX_EXEC, "sandbox.no_sandbox_exec");
    let windows = [
        CODE_WIN_GRANT_UNREADABLE,
        CODE_WIN_GRANT_DENIED,
        CODE_WIN_NO_LOGON_SID,
        CODE_WIN_RECORD_EXPOSED,
        CODE_WIN_LAUNCHER,
        CODE_WIN_NOT_PREPARED,
    ];
    assert!(windows.iter().all(|c| c.starts_with("sandbox.win_")));
    assert_eq!(CODE_UNSUPPORTED_PLATFORM, "sandbox.unsupported_platform");
}

#[test]
fn git_metadata_error_text_is_the_refusal_code() {
    assert_eq!(CODE_GIT_METADATA_LINKED, "sandbox.git_metadata_linked");
    let err = GitMetadataLinkedError;
    assert_eq!(err.to_string(), CODE_GIT_METADATA_LINKED);
}

#[test]
fn keepalive_detection_needs_both_background_and_a_wrapper() {
    let cases: &[(&str, bool)] = &[
        ("nohup sleep 120 &", true),
        ("setsid firefox &", true),
        ("FOO=1 nohup sleep 120 &", true),
        ("env nohup sleep 120 &", true),
        ("sleep 120 &", false),
        ("nohup sleep 120", false),
        ("$CMD sleep 120 &", false),
        ("echo 'disown'", false),
        ("", false),
    ];
    for (command, want) in cases {
        assert_eq!(
            has_explicit_background_keepalive(command),
            *want,
            "{command:?}"
        );
    }
}

#[test]
fn track_and_reap_follow_the_preserve_and_shell_rules() {
    let bash = ShellKind::Bash;
    let pwsh = ShellKind::PowerShell;

    // Deliberate daemon: neither tracked nor reaped under bash.
    assert!(!should_track_shell_process(false, bash, "nohup x &", false));
    assert!(!should_reap_after_run(false, bash, "nohup x &", false));

    // Plain bash child: tracked and reaped.
    assert!(should_track_shell_process(false, bash, "echo x", false));
    assert!(should_reap_after_run(false, bash, "echo x", false));

    // A non-bash shell keeps the default teardown regardless of the wrapper.
    assert!(should_track_shell_process(false, pwsh, "nohup x &", false));
    assert!(should_reap_after_run(false, pwsh, "nohup x &", false));

    // An intentional preserve untracks and spares the group.
    assert!(!should_track_shell_process(false, bash, "echo x", true));
    assert!(!should_reap_after_run(false, bash, "echo x", true));

    // A cancelled context always reaps stray members.
    assert!(should_reap_after_run(true, bash, "echo x", false));
}

#[test]
fn wrapped_windows_child_is_untracked() {
    #[cfg(windows)]
    assert!(!should_track_shell_process(
        true,
        ShellKind::Bash,
        "echo x",
        false
    ));
    #[cfg(not(windows))]
    assert!(should_track_shell_process(
        true,
        ShellKind::Bash,
        "echo x",
        false
    ));
}

#[test]
fn wait_delay_hold_is_suppressed_only_under_preserve_with_a_live_context() {
    assert!(suppress_wait_delay_error(true, false, true));
    assert!(!suppress_wait_delay_error(true, true, true));
    assert!(!suppress_wait_delay_error(false, false, true));
    assert!(!suppress_wait_delay_error(true, false, false));
}

#[test]
fn progress_marker_line_is_suppressed_only_when_the_probe_is_active() {
    let marker = "__rx_pipestatus_abc";
    assert!(progress_line_suppressed(
        marker,
        "  __rx_pipestatus_abc 0 1"
    ));
    assert!(!progress_line_suppressed(marker, "go test: failing 1 case"));
    assert!(!progress_line_suppressed("", "__rx_pipestatus_abc 0 1"));
}

#[test]
fn preview_flattens_newlines_and_truncates_at_the_rune_budget() {
    assert_eq!(command_preview("echo hi"), "echo hi");
    assert_eq!(command_preview("  ls -la\n/path  "), "ls -la /path");
    let long = "x".repeat(60);
    let got = command_preview(&long);
    assert!(got.ends_with("…"));
    assert_eq!(got.chars().count(), COMMAND_PREVIEW_MAX_RUNES + 1);
}

#[test]
fn snip_hint_defaults_match_the_go_head_tail_shape() {
    let hint = BashSnipHint::default();
    assert_eq!(hint.head, BASH_SNIP_HEAD);
    assert_eq!(hint.tail, BASH_SNIP_TAIL);
    assert_eq!(hint.head_chars, BASH_SNIP_HEAD_CHARS);
    assert_eq!(hint.tail_chars, BASH_SNIP_TAIL_CHARS);
    assert_eq!(BASH_SNIP_HEAD, 40);
    assert_eq!(BASH_SNIP_TAIL, 40);
    assert_eq!(BASH_SNIP_HEAD_CHARS, 8000);
    assert_eq!(BASH_SNIP_TAIL_CHARS, 8000);
}
