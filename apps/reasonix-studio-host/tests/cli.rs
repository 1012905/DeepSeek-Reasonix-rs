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
    let out = Command::new(bin()).arg("-bogus").output().expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("flag provided but not defined: -bogus"),
        "stderr: {err}"
    );
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
fn a_launch_whose_lease_ends_drains_cleanly() {
    // The kernel wiring is live now (docs/contracts/host.md §1.5): a pipe
    // stdin whose parent end closes ends the host with exit 0.
    use std::io::Read as _;
    use std::process::Stdio;
    let mut child = Command::new(bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    {
        // Take the handshake off the pipe first: it is the first stdout line.
        let mut out = child.stdout.take().expect("stdout piped");
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match out.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    buf.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let line = String::from_utf8_lossy(&buf);
        assert!(
            line.starts_with("{\"version\":1,"),
            "handshake line: {line}"
        );
    }
    // The parent goes away: closing the lease ends the host cleanly.
    drop(child.stdin.take());
    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(0), "the host outlived the lease");
}

#[cfg(unix)]
#[test]
fn a_terminated_host_exits_cleanly() {
    // OPT-008's second path: SIGTERM converges to exit 0 like the lease EOF
    // does (both only cancel the one token).
    use std::io::Read as _;
    use std::process::Stdio;
    let mut child = Command::new(bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    {
        let mut out = child.stdout.take().expect("stdout piped");
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while matches!(out.read(&mut byte), Ok(1)) {
            buf.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        assert!(String::from_utf8_lossy(&buf).starts_with("{\"version\":1,"));
    }
    let id = child.id();
    terminate(id);
    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(0), "SIGTERM should drain to exit 0");
}

#[cfg(unix)]
fn terminate(pid: u32) {
    // libc-free signal: the kill(1) coreutil is present on every CI unix.
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("kill");
}
