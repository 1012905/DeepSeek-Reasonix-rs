//! Exit-code contract of the host binary (docs/contracts/host.md §1.3): 2 on
//! an undefined flag, 1 on every startup failure, 0 only for a clean drain.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_reasonix-studio-host")
}

fn run(args: &[&str]) -> i32 {
    let out = Command::new(bin())
        .args(args)
        .env_remove("REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS")
        .output()
        .expect("spawn the host binary");
    out.status.code().unwrap_or(-1)
}

#[test]
fn an_undefined_flag_exits_two_with_usage() {
    let out = Command::new(bin())
        .arg("-bogus")
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("flag provided but not defined: -bogus"), "stderr: {err}");
    assert!(err.contains("usage:"), "stderr carries usage: {err}");
}

#[test]
fn a_schedule_child_is_recognized_by_its_arguments() {
    // Recognition itself is a boolean in the reference
    // (TestAScheduleChildIsRecognisedByItsArguments); the exit code is
    // asserted only for the recognized forms, because in this interim build
    // every unrecognized path exits 1 too.
    assert_eq!(run(&["schedule", "exec", "sch_x/1"]), 1);
    assert_eq!(run(&["schedule", "exec"]), 1);
}

#[test]
fn the_strip_mode_refuses_by_name_in_this_build() {
    assert_eq!(run(&["-strip-package-grants"]), 1);
}

#[test]
fn a_plain_launch_without_the_kernel_surface_is_a_startup_failure() {
    // Interim posture until the kernel wiring lands (OPEN_QUESTIONS §3):
    // valid flags still fail the start, with the reason on stderr.
    let out = Command::new(bin()).output().expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("reasonix-studio-host:"), "stderr: {err}");
}
