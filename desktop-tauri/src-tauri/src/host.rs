// The kernel sidecar: spawned with piped stdio, its first stdout line is the
// launch handshake and every later line an act it asks of this shell. stdin is
// the lease — the host drains when this end closes, so it cannot outlive the
// shell that started it.
use serde::Deserialize;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u32 = 1;
// A child that never sends a handshake must not grow this process while it
// waits, nor hold the launch open forever.
pub const HANDSHAKE_LIMIT: u64 = 4096;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const RETRY_WITHIN: Duration = Duration::from_secs(10);
const MIN_TOKEN_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub origin: String,
    pub token: String,
}

/// Why a launch has no kernel. Callers branch on the variant, never on text.
#[derive(Debug)]
pub enum StartError {
    Spawn(String),
    Exited(Option<i32>),
    Timeout,
    Overflow,
    Malformed(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(why) => write!(f, "the kernel could not be started: {why}"),
            Self::Exited(Some(code)) => {
                write!(f, "the kernel exited with {code} before saying anything")
            }
            Self::Exited(None) => write!(f, "the kernel ended before saying anything"),
            Self::Timeout => write!(
                f,
                "the kernel was still running but sent no handshake within {} s",
                HANDSHAKE_TIMEOUT.as_secs()
            ),
            Self::Overflow => write!(f, "the handshake ran past its limit"),
            Self::Malformed(why) => write!(f, "{why}"),
        }
    }
}

#[derive(Deserialize)]
struct Wire {
    version: Option<u32>,
    origin: Option<String>,
    token: Option<String>,
}

/// Refuses anything it cannot account for. No message carries the line: the
/// credential is in it.
pub fn parse(line: &str) -> Result<Handshake, String> {
    let wire: Wire =
        serde_json::from_str(line.trim()).map_err(|_| "the handshake was not JSON".to_string())?;
    if wire.version != Some(PROTOCOL_VERSION) {
        return Err(format!(
            "unsupported handshake version {}",
            wire.version.map_or("(none)".into(), |v| v.to_string())
        ));
    }
    let token = wire.token.unwrap_or_default();
    if token.len() < MIN_TOKEN_LEN {
        return Err("the handshake carried no usable credential".into());
    }
    let url: tauri::Url = wire
        .origin
        .unwrap_or_default()
        .parse()
        .map_err(|_| "the handshake carried no usable origin".to_string())?;
    if url.scheme() != "http" {
        return Err(format!(
            "the handshake named {}:, which this shell does not load",
            url.scheme()
        ));
    }
    if url.host_str() != Some("127.0.0.1") {
        return Err(format!(
            "the handshake named {}, which is not this machine",
            url.host_str().unwrap_or("")
        ));
    }
    let port = url.port().ok_or("the handshake named no port")?;
    Ok(Handshake {
        origin: format!("http://127.0.0.1:{port}"),
        token,
    })
}

/// Where an act line names one the shell performs; anything else is ignored.
pub fn parse_act(line: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Act {
        act: String,
    }
    serde_json::from_str::<Act>(line.trim()).ok().map(|a| a.act)
}

pub struct Running {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: BufReader<ChildStdout>,
    pub handshake: Handshake,
}

pub type StderrSink = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;

pub struct Launch<'a> {
    pub binary: &'a Path,
    pub args: Vec<String>,
    pub system_language: Option<String>,
    pub timeout: Duration,
    pub stderr: StderrSink,
}

// Drained from the moment of the spawn: what a host says before it gives up is
// usually why, and a pipe nobody reads would stall it mid-sentence.
fn drain(stderr: ChildStderr, sink: StderrSink) {
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                return;
            }
            sink(&buf[..n]);
        }
    });
}

fn spawn(launch: &Launch<'_>) -> Result<Child, StartError> {
    let mut command = Command::new(launch.binary);
    command
        .args(&launch.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(lang) = &launch.system_language {
        command.env("REASONIX_SYSTEM_LANG", lang);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map_err(|e| StartError::Spawn(e.to_string()))
}

fn first_line(
    stdout: ChildStdout,
    timeout: Duration,
) -> Result<(String, BufReader<ChildStdout>), StartError> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(HANDSHAKE_LIMIT)
            .read_until(b'\n', &mut line);
        let _ = tx.send((read, line, reader));
    });
    match rx.recv_timeout(timeout) {
        Err(_) => Err(StartError::Timeout),
        Ok((Err(e), _, _)) => Err(StartError::Malformed(format!("reading the handshake: {e}"))),
        Ok((Ok(0), _, _)) => Err(StartError::Exited(None)),
        Ok((Ok(_), line, reader)) => {
            if line.last() != Some(&b'\n') && line.len() as u64 >= HANDSHAKE_LIMIT {
                return Err(StartError::Overflow);
            }
            Ok((String::from_utf8_lossy(&line).into_owned(), reader))
        }
    }
}

fn attempt(launch: &Launch<'_>) -> Result<Running, StartError> {
    let mut child = spawn(launch)?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let stdin = child.stdin.take().expect("stdin is piped");
    drain(
        child.stderr.take().expect("stderr is piped"),
        launch.stderr.clone(),
    );
    let read = first_line(stdout, launch.timeout);
    let (line, stdout) = match read {
        Ok(found) => found,
        Err(StartError::Exited(_)) => {
            let code = child.wait().ok().and_then(|s| s.code());
            return Err(StartError::Exited(code));
        }
        Err(err) => {
            let _ = child.kill();
            // A killed child is reaped here, not left for the app's
            // lifetime while a retry spawns a replacement.
            let _ = child.wait();
            return Err(err);
        }
    };
    let handshake = parse(&line).map_err(|why| {
        let _ = child.kill();
        let _ = child.wait();
        StartError::Malformed(why)
    })?;
    Ok(Running {
        child,
        stdin,
        stdout,
        handshake,
    })
}

/// One more attempt only for a kernel that stopped, or could not be run, soon
/// after it was started: a scanner holding a fresh binary lets go about that fast.
pub fn should_retry(err: &StartError, attempt: u32, elapsed: Duration) -> bool {
    attempt < 2
        && matches!(err, StartError::Exited(_) | StartError::Spawn(_))
        && elapsed <= RETRY_WITHIN
}

pub fn start(launch: &Launch<'_>, on_retry: impl Fn(&StartError)) -> Result<Running, StartError> {
    let mut tries = 0;
    loop {
        tries += 1;
        let began = Instant::now();
        match attempt(launch) {
            Ok(running) => return Ok(running),
            Err(err) if should_retry(&err, tries, began.elapsed()) => {
                on_retry(&err);
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(err) => return Err(err),
        }
    }
}

/// Carries on after the handshake, one act per line. A child writing without
/// newlines is cut at the limit rather than allowed to grow this process.
pub fn read_acts(stdout: BufReader<ChildStdout>, on_act: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        let mut reader = stdout;
        loop {
            let mut line = Vec::new();
            match (&mut reader)
                .take(HANDSHAKE_LIMIT)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    // host.md: a buffer over the limit without a newline is
                    // dropped whole — drain the tail so it cannot come back
                    // around as an act of its own.
                    if line.last() != Some(&b'\n') {
                        discard_to_newline(&mut reader);
                        continue;
                    }
                    if let Some(act) = parse_act(&String::from_utf8_lossy(&line)) {
                        on_act(act);
                    }
                }
            }
        }
    });
}

/// Consumes the tail of an over-limit line, in bounded steps, to its
/// newline or the end of the stream.
fn discard_to_newline(mut reader: &mut BufReader<ChildStdout>) {
    loop {
        let mut discard = Vec::new();
        match (&mut reader)
            .take(HANDSHAKE_LIMIT)
            .read_until(b'\n', &mut discard)
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if discard.last() == Some(&b'\n') {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn line(version: u32, origin: &str, token: &str) -> String {
        format!("{{\"version\":{version},\"origin\":\"{origin}\",\"token\":\"{token}\"}}\n")
    }

    #[test]
    fn parse_accepts_a_loopback_handshake() {
        let bound = parse(&line(1, "http://127.0.0.1:5281", TOKEN)).expect("parses");
        assert_eq!(
            bound,
            Handshake {
                origin: "http://127.0.0.1:5281".into(),
                token: TOKEN.into()
            }
        );
    }

    #[test]
    fn parse_normalises_the_origin() {
        let bound = parse(&line(1, "http://127.0.0.1:5281/", TOKEN)).expect("parses");
        assert_eq!(bound.origin, "http://127.0.0.1:5281");
    }

    #[test]
    fn parse_refuses_what_it_cannot_account_for() {
        assert!(parse("not json").is_err());
        assert!(parse(&line(2, "http://127.0.0.1:5281", TOKEN)).is_err());
        assert!(parse(&line(1, "http://127.0.0.1:5281", "short")).is_err());
        assert!(parse(&line(1, "https://127.0.0.1:5281", TOKEN)).is_err());
        assert!(parse(&line(1, "http://localhost:5281", TOKEN)).is_err());
        assert!(parse(&line(1, "http://0.0.0.0:5281", TOKEN)).is_err());
        assert!(parse(&line(1, "http://127.0.0.1", TOKEN)).is_err());
    }

    #[test]
    fn parse_never_repeats_the_credential() {
        let err = parse(&line(2, "http://127.0.0.1:5281", TOKEN)).unwrap_err();
        assert!(!err.contains(TOKEN));
    }

    #[test]
    fn parse_act_reads_only_named_acts() {
        assert_eq!(parse_act("{\"act\":\"show\"}\n"), Some("show".into()));
        assert_eq!(parse_act("{\"other\":1}"), None);
        assert_eq!(parse_act("garbage"), None);
    }

    #[test]
    fn retry_is_once_and_only_for_an_early_stop() {
        let soon = Duration::from_secs(1);
        assert!(should_retry(&StartError::Exited(Some(1)), 1, soon));
        assert!(should_retry(&StartError::Spawn("busy".into()), 1, soon));
        assert!(!should_retry(&StartError::Exited(Some(1)), 2, soon));
        assert!(!should_retry(&StartError::Timeout, 1, soon));
        assert!(!should_retry(
            &StartError::Exited(Some(1)),
            1,
            Duration::from_secs(30)
        ));
    }
}
