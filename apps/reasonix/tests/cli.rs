//! End-to-end CLI contract tests for the `reasonix` binary.
//!
//! These drive the real binary (`CARGO_BIN_EXE_reasonix`), not the library,
//! so they pin the process-level contract: stdout bytes, stderr, exit codes.

use std::process::Command;

fn reasonix() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reasonix"))
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = reasonix().args(args).output().expect("spawn reasonix");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// `reasonix --version` prints exactly `reasonix <version>\n` and exits 0
/// (ledger CLI-001; reference cli.go `versionCommand(nil, info, false)`).
#[test]
fn version_flag_prints_single_line() {
    let (stdout, stderr, code) = run(&["--version"]);
    assert_eq!(stdout, "reasonix dev\n");
    assert!(stderr.is_empty(), "stderr should stay quiet: {stderr}");
    assert_eq!(code, Some(0));
}

/// `reasonix -v` matches `--version` byte for byte (ledger CLI-002).
#[test]
fn short_version_flag_matches() {
    let (stdout_long, _, code_long) = run(&["--version"]);
    let (stdout_short, _, code_short) = run(&["-v"]);
    assert_eq!(stdout_long, stdout_short);
    assert_eq!(code_long, code_short);
    assert_eq!(code_short, Some(0));
}

/// `reasonix version` prints the same single line (ledger CLI-003).
#[test]
fn version_subcommand_prints_single_line() {
    let (stdout, stderr, code) = run(&["version"]);
    assert_eq!(stdout, "reasonix dev\n");
    assert!(stderr.is_empty(), "stderr should stay quiet: {stderr}");
    assert_eq!(code, Some(0));
}

/// Unknown commands exit 2: the command name lands on stderr and the usage
/// body on stdout, matching the reference `unknownCommand` path (ledger
/// CLI-015).
#[test]
fn unknown_command_exits_two() {
    let (stdout, stderr, code) = run(&["definitely-not-a-command"]);
    assert!(
        stderr.starts_with("unknown command \"definitely-not-a-command\""),
        "stderr: {stderr}"
    );
    assert!(
        stdout.starts_with("reasonix — "),
        "stdout should carry the usage body: {stdout}"
    );
    assert_eq!(code, Some(2));
}
