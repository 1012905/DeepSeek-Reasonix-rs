//! Command reasonix-studio-host runs Studio's kernel behind a loopback socket.
//!
//! Rust rewrite of `reference/.../cmd/reasonix-studio-host` (docs/contracts/
//! host.md): the binary owns argv precedence, flag parsing, the credential,
//! the handshake and act lines, the phase log, and the lease; the kernel
//! composition it drives lives in `reasonix_assembly::host` — binaries reach
//! the kernel through assembly only (ADR-0001).

#![forbid(unsafe_code)]

#[allow(dead_code)] // wired with the notify slice (toast show, quit on end)
mod acts;
mod args;
mod credential;
mod handshake;
mod lastline;
mod lease;
mod phases;

use lastline::LastLine;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const BIN: &str = "reasonix-studio-host";

type Log = Arc<LastLine>;

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
        println!("{}", reasonix_assembly::host::instance_id());
        return ExitCode::from(0);
    }
    run(parsed)
}

fn schedule_child_args(args: &[String]) -> bool {
    args.len() >= 2 && args[0] == "schedule" && args[1] == "exec"
}

fn log_line(log: &Log, msg: &str) {
    let _ = log.write_bytes(format!("{BIN}: {msg}\n").as_bytes());
}

/// Writes through the shared log so phase lines and error lines race for the
/// last `reasonix-studio-host:` line the way Go's single `logs` writer makes
/// them.
struct PhaseWriter(Log);

impl Write for PhaseWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_bytes(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run(parsed: args::Args) -> ExitCode {
    let log: Log = Arc::new(LastLine::new());
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log_line(&log, &e.to_string());
            return ExitCode::from(1);
        }
    };
    runtime.block_on(host_run(parsed, log))
}

async fn host_run(parsed: args::Args, log: Log) -> ExitCode {
    let phases: Arc<Mutex<phases::StartupPhases<PhaseWriter>>> = Arc::new(Mutex::new(
        phases::StartupPhases::new(PhaseWriter(log.clone())),
    ));
    let on_phase: Arc<dyn Fn(&str) + Send + Sync> = {
        let phases = phases.clone();
        Arc::new(move |name: &str| {
            let mut p = phases.lock().unwrap_or_else(|p| p.into_inner());
            p.mark(name);
        })
    };
    let on_log: Arc<dyn Fn(&str) + Send + Sync> = {
        let log = log.clone();
        Arc::new(move |msg: &str| log_line(&log, msg))
    };

    // Either signal or the parent letting go of the lease ends the context;
    // the lease's EOF is the whole message, identically on every platform.
    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        });
    }
    #[cfg(unix)]
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if let Ok(mut term) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                term.recv().await;
                cancel.cancel();
            }
        });
    }
    if lease::parent_lease(std::io::stdin().is_terminal()) == lease::Lease::Supervised {
        tokio::spawn(lease::watch_lease(cancel.clone()));
    }

    // The credential is minted here rather than read (host.md §2.1) and the
    // gate is built around it; nothing from configuration reaches it.
    let token = match credential::launch_credential() {
        Ok(token) => token,
        Err(e) => {
            log_line(&log, &e.to_string());
            return ExitCode::from(1);
        }
    };
    let page = if parsed.page.as_os_str().is_empty() {
        None
    } else {
        Some(parsed.page.clone())
    };
    let boot = match reasonix_assembly::host::prepare(reasonix_assembly::host::HostBootOptions {
        page,
        token: token.clone(),
        app_exe: parsed.studio_app.clone(),
        on_phase,
        on_log,
        cwd: std::env::current_dir().unwrap_or_default(),
    })
    .await
    {
        Ok(boot) => boot,
        Err(e) => {
            log_line(&log, &e.to_string());
            return ExitCode::from(1);
        }
    };

    // The handshake goes down the pipe the parent already holds, after bind
    // and before serving; the acts share the same stdout, one line each.
    let mut out = std::io::stdout().lock();
    let said = handshake::Handshake {
        version: handshake::HANDSHAKE_VERSION,
        origin: boot.origin.clone(),
        token: boot.token.clone(),
    };
    if handshake::announce(&mut out, &said).is_err() {
        log_line(&log, "the handshake could not reach the shell");
        return ExitCode::from(1);
    }
    {
        let mut p = phases.lock().unwrap_or_else(|p| p.into_inner());
        p.mark(phases::ANNOUNCE);
    }

    match reasonix_assembly::host::serve_until(boot, cancel.cancelled_owned()).await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            log_line(&log, &e.to_string());
            ExitCode::from(1)
        }
    }
}
