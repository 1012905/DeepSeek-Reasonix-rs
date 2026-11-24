//! ACP-007: terminal create/wait/output/release shapes, 1 MiB hint, exit mapping.

use std::collections::BTreeMap;

use reasonix_acp::terminal::{
    TERMINAL_OUTPUT_BYTE_LIMIT, TRUNCATED_NOTE, TerminalCreateParams, TerminalExitStatus,
    TerminalIdParams, assemble_output, env_map_to_variables, exit_status_error, timeout_error,
};
use reasonix_acp::wire::EnvVariable;

#[test]
fn output_limit_is_one_mebibyte_hint_to_client() {
    assert_eq!(TERMINAL_OUTPUT_BYTE_LIMIT, 1 << 20);
    assert_eq!(TERMINAL_OUTPUT_BYTE_LIMIT, 1_048_576);
}

#[test]
fn create_params_serialize_with_limit_and_sorted_env() {
    let mut env = BTreeMap::new();
    env.insert("TMP".to_owned(), "/tmp/sess".to_owned());
    env.insert("TMPDIR".to_owned(), "/tmp/sess".to_owned());
    let params = TerminalCreateParams {
        session_id: "sess-1".to_owned(),
        command: "echo hi".to_owned(),
        args: None,
        cwd: Some("/proj".to_owned()),
        env: env_map_to_variables(&env),
        output_byte_limit: Some(TERMINAL_OUTPUT_BYTE_LIMIT as i32),
    };
    let v = serde_json::to_value(&params).unwrap();
    assert_eq!(v["sessionId"], "sess-1");
    assert_eq!(v["command"], "echo hi");
    assert!(v.get("args").is_none());
    assert_eq!(v["outputByteLimit"], 1_048_576);
    assert_eq!(v["env"][0]["name"], "TMP");
    assert_eq!(v["env"][1]["name"], "TMPDIR");
}

#[test]
fn env_variables_skip_blank_names_and_omit_empty_map() {
    let mut map = BTreeMap::new();
    map.insert("".to_owned(), "ignored".to_owned());
    assert!(env_map_to_variables(&map).is_none());

    map.insert("TMPDIR".to_owned(), "/tmp/x".to_owned());
    let vars = env_map_to_variables(&map).unwrap();
    assert_eq!(
        vars,
        vec![EnvVariable {
            name: "TMPDIR".to_owned(),
            value: "/tmp/x".to_owned(),
        }]
    );
}

#[test]
fn shared_id_params_for_wait_output_kill_release() {
    let params = TerminalIdParams {
        session_id: "sess-1".to_owned(),
        terminal_id: "term-1".to_owned(),
    };
    let v = serde_json::to_value(&params).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"sessionId": "sess-1", "terminalId": "term-1"})
    );
}

#[test]
fn truncated_output_gets_fixed_suffix_only_when_client_reports_truncated() {
    let truncated = assemble_output("line one".to_owned(), true, None);
    assert_eq!(truncated.output, format!("line one{TRUNCATED_NOTE}"));
    assert!(truncated.truncated);

    let full = assemble_output("line one".to_owned(), false, None);
    assert_eq!(full.output, "line one");
    assert!(!full.truncated);
}

#[test]
fn exit_status_maps_code_and_signal_verbatim() {
    let nonzero = TerminalExitStatus {
        exit_code: Some(7),
        signal: None,
    };
    assert_eq!(
        exit_status_error(Some(&nonzero)).as_deref(),
        Some("exit status 7")
    );

    let signal = TerminalExitStatus {
        exit_code: None,
        signal: Some("SIGTERM".to_owned()),
    };
    assert_eq!(
        exit_status_error(Some(&signal)).as_deref(),
        Some("terminated by signal SIGTERM")
    );

    let clean = TerminalExitStatus {
        exit_code: Some(0),
        signal: Some(String::new()),
    };
    assert!(exit_status_error(Some(&clean)).is_none());
    assert!(exit_status_error(None).is_none());
}

#[test]
fn timeout_messages_match_templates() {
    assert_eq!(
        timeout_error(Some(std::time::Duration::from_secs(5))),
        "command timed out after 5s (terminal killed)"
    );
    assert_eq!(timeout_error(None), "command timed out (terminal killed)");
}
