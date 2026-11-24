//! `terminal/*` agent → client requests (clientio.go:90-191; protocol.go:
//! 697-734).
//!
//! Flow: `terminal/create` → `terminal/wait_for_exit` → `terminal/output` →
//! `terminal/release` (deferred). The client buffers at most
//! [`TERMINAL_OUTPUT_BYTE_LIMIT`] (1 MiB) of output for one command; a
//! client-truncated reply earns the fixed suffix note.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::wire::EnvVariable;

/// Output buffer bound requested from the client terminal: 1 MiB
/// (`1 << 20`, clientio.go:92).
pub const TERMINAL_OUTPUT_BYTE_LIMIT: usize = 1 << 20;

/// Suffix appended to a client-truncated output capture (clientio.go:187-189).
pub const TRUNCATED_NOTE: &str = "\n…(output truncated by the client terminal)";

/// `terminal/create` params (protocol.go:697-704).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalCreateParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<EnvVariable>>,
    #[serde(rename = "outputByteLimit", skip_serializing_if = "Option::is_none")]
    pub output_byte_limit: Option<i32>,
}

/// `terminal/create` result (protocol.go:707-709).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalCreateResult {
    #[serde(rename = "terminalId")]
    pub terminal_id: String,
}

/// A create reply counts only when its terminal id is non-blank; an empty or
/// whitespace id means creation failed (clientio.go:117-120).
pub fn terminal_id_ok(terminal_id: &str) -> bool {
    !terminal_id.trim().is_empty()
}

/// Params addressing one terminal for output / kill / wait / release
/// (protocol.go:712-715).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalIdParams {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "terminalId")]
    pub terminal_id: String,
}

/// How the command exited, shared by wait and output (protocol.go:726-733).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalExitStatus {
    #[serde(rename = "exitCode", skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// `terminal/output` result (protocol.go:718-722).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalOutputResult {
    pub output: String,
    pub truncated: bool,
    #[serde(rename = "exitStatus", skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<TerminalExitStatus>,
}

/// `terminal/wait_for_exit` result (protocol.go:725-728).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalWaitResult {
    #[serde(rename = "exitCode", skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// Converts the temporary-directory / shell env override map into ACP
/// `EnvVariable` entries. Blank names are skipped, keys are sorted for
/// stability, and an empty map serializes as omitted (clientio.go:157-175).
pub fn env_map_to_variables(map: &BTreeMap<String, String>) -> Option<Vec<EnvVariable>> {
    let entries: Vec<EnvVariable> = map
        .iter()
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|(name, value)| EnvVariable {
            name: name.clone(),
            value: value.clone(),
        })
        .collect();
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Assembles the captured output: a client that marked the reply truncated
/// gets the fixed note appended (clientio.go:186-190). The agent does not clip
/// output itself at the 1 MiB budget; that number is the buffer hint handed to
/// the client on create (matrix B §1.2, conflict C3).
pub fn assemble_output(
    output: String,
    truncated: bool,
    exit_status: Option<TerminalExitStatus>,
) -> TerminalOutputResult {
    let output = if truncated {
        format!("{output}{TRUNCATED_NOTE}")
    } else {
        output
    };
    TerminalOutputResult {
        output,
        truncated,
        exit_status,
    }
}

/// Classifies a finished terminal by its `exitStatus`, after wait errors and
/// parent cancellation have had first refusal (clientio.go:137-151).
///
/// Returns the internal error message verbatim when the command did not exit
/// cleanly: a nonzero exit code, or a non-empty termination signal.
pub fn exit_status_error(exit_status: Option<&TerminalExitStatus>) -> Option<String> {
    let status = exit_status?;
    if let Some(code) = status.exit_code
        && code != 0
    {
        return Some(format!("exit status {code}"));
    }
    match status.signal.as_deref() {
        Some(sig) if !sig.is_empty() => Some(format!("terminated by signal {sig}")),
        _ => None,
    }
}

/// Timeout error message for a command that exceeded its wait budget; the
/// killed terminal is released by the caller (clientio.go:140-143).
pub fn timeout_error(timeout: Option<std::time::Duration>) -> String {
    match timeout {
        Some(duration) => format!("command timed out after {duration:?} (terminal killed)"),
        None => "command timed out (terminal killed)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_byte_limit_is_one_mebibyte() {
        assert_eq!(TERMINAL_OUTPUT_BYTE_LIMIT, 1 << 20);
        assert_eq!(TERMINAL_OUTPUT_BYTE_LIMIT, 1_048_576);
    }

    #[test]
    fn env_variables_skip_blank_names_and_are_sorted() {
        let mut map = BTreeMap::new();
        map.insert("TMPDIR".to_owned(), "/tmp/sess".to_owned());
        map.insert("".to_owned(), "ignored".to_owned());
        map.insert("TMP".to_owned(), "/tmp/sess".to_owned());
        let vars = env_map_to_variables(&map).unwrap();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "TMP");
        assert_eq!(vars[1].name, "TMPDIR");
        assert!(env_map_to_variables(&BTreeMap::new()).is_none());
    }

    #[test]
    fn truncated_output_carries_fixed_suffix() {
        let out = assemble_output("line one".to_owned(), true, None);
        assert_eq!(out.output, format!("line one{TRUNCATED_NOTE}"));
        assert!(out.truncated);

        let full = assemble_output("line one".to_owned(), false, None);
        assert_eq!(full.output, "line one");
        assert!(!full.truncated);
    }

    #[test]
    fn exit_status_error_maps_code_and_signal() {
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
    fn timeout_message_includes_duration_when_given() {
        assert_eq!(
            timeout_error(Some(std::time::Duration::from_secs(5))),
            "command timed out after 5s (terminal killed)"
        );
        assert_eq!(timeout_error(None), "command timed out (terminal killed)");
    }

    #[test]
    fn blank_terminal_id_is_not_a_created_terminal() {
        assert!(terminal_id_ok("term-1"));
        assert!(!terminal_id_ok(""));
        assert!(!terminal_id_ok("   "));
    }

    #[test]
    fn create_params_carry_limit_and_env() {
        let params = TerminalCreateParams {
            session_id: "s1".to_owned(),
            command: "ls".to_owned(),
            args: Some(vec!["-la".to_owned()]),
            cwd: Some("/tmp".to_owned()),
            env: Some(vec![EnvVariable {
                name: "TMPDIR".to_owned(),
                value: "/tmp/sess".to_owned(),
            }]),
            output_byte_limit: Some(TERMINAL_OUTPUT_BYTE_LIMIT as i32),
        };
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["command"], "ls");
        assert_eq!(value["outputByteLimit"], TERMINAL_OUTPUT_BYTE_LIMIT as i32);
        assert_eq!(value["env"][0]["name"], "TMPDIR");
    }
}
