//! Command reasonix-studio-host runs Studio's kernel behind a loopback socket.
//!
//! Rust rewrite of `reference/.../cmd/reasonix-studio-host`, landing slice by
//! slice (docs/contracts/host.md, ledger HOST rows). This build carries the
//! argv precedence, flag parsing, credential/handshake/act machinery, phase
//! log, and exit-code table; the kernel surface it serves lands with the
//! serve/session/config slices and is wired in `run`.

#![forbid(unsafe_code)]

mod acts;
mod args;
mod credential;
mod handshake;
mod instance;
mod lastline;
mod launchroot;
mod lease;
mod phases;

use std::io::Write;
use std::process::ExitCode;

const BIN: &str = "reasonix-studio-host";

fn main() -> ExitCode {
    // Before flag parsing: re-entry modes whose argv belongs to an update or a
    // schedule, never to this host's flag set (docs/contracts/host.md §1.1).
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if schedule_child_args(&argv) {
        // The schedulerun machine lands with the platform slice; a recognized
        // child must not fall through to serving (OPEN_QUESTIONS.md §4).
        eprintln!("{BIN}: schedule exec child reached a build without the scheduler");
        return ExitCode::from(1);
    }
    let parsed = match args::parse(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{BIN}: {e}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    if parsed.strip_package_grants {
        // The package-grant strip lands with the platform slice; refusing by
        // name beats pretending to have stripped anything.
        eprintln!("{BIN}: -strip-package-grants is not implemented in this build");
        return ExitCode::from(1);
    }
    if parsed.instance_id {
        // Wired with the config slice's home resolution (docs/contracts/host.md §5.1).
        eprintln!("{BIN}: -instance-id is not implemented in this build");
        return ExitCode::from(1);
    }
    run(parsed)
}

fn schedule_child_args(args: &[String]) -> bool {
    args.len() >= 2 && args[0] == "schedule" && args[1] == "exec"
}

/// The startup failure posture until the kernel surface is wired: every start
/// is a startup failure (exit 1) with the reason on stderr
/// (docs/contracts/host.md §1.3).
fn run(parsed: args::Args) -> ExitCode {
    let mut logs = lastline::LastLine::new(std::io::stderr().lock());
    let mut phases = phases::StartupPhases::new(&mut logs);
    let _ = parsed.page;
    let _ = &mut phases;
    writeln!(
        logs,
        "{BIN}: the kernel surface lands with the serve slice \
        (docs/parity/LEDGER.md HOST rows; see OPEN_QUESTIONS.md §3)"
    )
    .ok();
    ExitCode::from(1)
}
