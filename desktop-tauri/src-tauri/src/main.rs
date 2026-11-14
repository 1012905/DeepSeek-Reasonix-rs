// The shell owns OS capability and no business state; the kernel is a sidecar
// this process spawns and holds a lease on — the pipe its stdin runs on, so
// the host's lifetime is exactly the shell's. The handshake is one JSON line
// on stdout naming the loopback origin the host serves the kernel APIs from
// and the token that origin's gate expects as the reasonix_token cookie. The
// page never sees the token: it asks this shell over IPC, and the shell adds
// the credential itself (the gate takes it from the cookie only — a query
// token would land in request lines). Acts the host asks of the shell arrive
// on the same pipe, one line each, after the handshake.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Deserialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

const TOKEN_COOKIE: &str = "reasonix_token";

#[derive(Deserialize)]
struct Handshake {
    version: u32,
    origin: String,
    token: String,
}

struct KernelState {
    origin: String,
    token: String,
    child: Mutex<Child>,
}

fn host_binary() -> PathBuf {
    std::env::var_os("REASONIX_STUDIO_HOST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/reasonix-studio-host.exe")
        })
}

fn spawn_host() -> Result<Child, String> {
    let bin = host_binary();
    if !bin.exists() {
        return Err(format!(
            "the kernel host binary is missing at {} — build it: go build -o src-tauri/bin/reasonix-studio-host.exe ../cmd/reasonix-studio-host",
            bin.display()
        ));
    }
    Command::new(&bin)
        .arg("-studio-app-pid")
        .arg(std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawning {}: {e}", bin.display()))
}

fn read_handshake(stdout: &mut impl BufRead) -> Result<Handshake, String> {
    let mut line = String::new();
    stdout
        .read_line(&mut line)
        .map_err(|e| format!("reading the host handshake: {e}"))?;
    if line.trim().is_empty() {
        return Err("the host ended before its handshake".into());
    }
    let handshake: Handshake = serde_json::from_str(line.trim())
        .map_err(|e| format!("parsing the host handshake: {e}"))?;
    if handshake.version != 1 {
        return Err(format!(
            "unsupported host handshake version {}",
            handshake.version
        ));
    }
    Ok(handshake)
}

// The credential crosses the loopback gate as the reasonix_token cookie, and
// the Origin header must name the listener on everything that changes state —
// both added here, so the page's own requests carry neither.
fn build_request(
    method: &str,
    authority: &str,
    path: &str,
    cookie: &str,
    body: Option<&str>,
) -> String {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\nCookie: {TOKEN_COOKIE}={cookie}\r\nConnection: close\r\n"
    );
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }
    request
}

// Acting on the acts — relaunch after an update, quit, show — is a later
// phase; for now they are drained so the pipe never fills and blocks the host.
fn drain_acts(stdout: impl BufRead + Send + 'static) {
    std::thread::spawn(move || {
        for line in stdout.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => println!("host act: {line}"),
                Err(_) => break,
                Ok(_) => {}
            }
        }
    });
}

// The live event stream: one GET /events per shell, frames forwarded to the
// webview as kernel-frame events. A dropped stream reconnects with
// Last-Event-ID, so resume costs only what the replay log cannot reach — the
// same contract the SPA's EventSource rides. Keepalive comments fall through.
fn bridge_events(app: tauri::AppHandle, origin: String, token: String) {
    std::thread::spawn(move || {
        let mut last_seq: i64 = 0;
        loop {
            if let Err(err) = events_stream(
                &|payload: &str| {
                    app.emit("kernel-frame", payload)
                        .map_err(|e| format!("emitting a frame: {e}"))
                },
                &origin,
                &token,
                &mut last_seq,
            ) {
                eprintln!("kernel events: {err}");
            }
            // The stream ended — the hub stopped or the connection dropped.
            // Either way the replay picks up from the watermark after a beat.
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

enum SseLine {
    Id(i64),
    Data(String),
    Other,
}

// One line of the host's /events stream: numbered frames, JSON data, and
// keepalive comments that carry no frame.
fn parse_sse_line(line: &str) -> SseLine {
    if let Some(seq) = line.strip_prefix("id: ") {
        return seq
            .parse::<i64>()
            .map(SseLine::Id)
            .unwrap_or(SseLine::Other);
    }
    if let Some(data) = line.strip_prefix("data: ") {
        return SseLine::Data(data.to_string());
    }
    SseLine::Other
}

fn events_stream(
    emit: &dyn Fn(&str) -> Result<(), String>,
    origin: &str,
    token: &str,
    last_seq: &mut i64,
) -> Result<(), String> {
    let url: tauri::Url = format!("{origin}/events")
        .parse()
        .map_err(|e| format!("parsing /events: {e}"))?;
    let authority = format!(
        "{}:{}",
        url.host_str().ok_or("the kernel origin has no host")?,
        url.port_or_known_default()
            .ok_or("the kernel origin has no port")?
    );
    let request = [
        "GET /events HTTP/1.1\r\n".to_string(),
        format!("Host: {authority}\r\n"),
        format!("Cookie: reasonix_token={token}\r\n"),
        format!("Last-Event-ID: {last_seq}\r\n"),
        "Accept: text/event-stream\r\n".to_string(),
        "\r\n".to_string(),
    ]
    .concat();
    let mut stream = std::net::TcpStream::connect(&authority)
        .map_err(|e| format!("connecting to /events: {e}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("sending /events: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("reading the event stream: {e}"))?;
        if read == 0 {
            return Ok(()); // the hub ended its stream
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        match parse_sse_line(trimmed) {
            SseLine::Id(seq) => *last_seq = seq,
            SseLine::Data(payload) => emit(&payload)?,
            SseLine::Other => {}
        }
    }
}

fn run() -> Result<(), String> {
    tauri::Builder::default()
        .setup(|app| {
            let mut kernel = spawn_host()?;
            let bound = {
                let stdout = kernel
                    .stdout
                    .take()
                    .ok_or("the host stdout pipe is missing")?;
                let mut reader = BufReader::new(stdout);
                let bound = read_handshake(&mut reader)?;
                drain_acts(reader);
                bound
            };
            app.manage(KernelState {
                origin: bound.origin.clone(),
                token: bound.token.clone(),
                child: Mutex::new(kernel),
            });
            bridge_events(app.handle().clone(), bound.origin, bound.token);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            kernel_connected,
            kernel_get,
            kernel_post
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}

#[derive(serde::Serialize)]
struct KernelStatus {
    connected: bool,
}

// Connected means the host process is still running; an exit flips the badge
// on the next poll without the shell restarting anything.
#[tauri::command]
fn kernel_connected(state: State<KernelState>) -> Result<KernelStatus, String> {
    let mut child = state.child.lock().map_err(|e| e.to_string())?;
    let connected = child.try_wait().map_err(|e| e.to_string())?.is_none();
    Ok(KernelStatus { connected })
}

fn kernel_request(
    state: &KernelState,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<String, String> {
    if !path.starts_with('/') || path.contains("..") {
        return Err("the kernel path must be absolute inside the host".into());
    }
    let url: tauri::Url = format!("{}{path}", state.origin)
        .parse()
        .map_err(|e| format!("parsing the kernel path: {e}"))?;
    let authority = format!(
        "{}:{}",
        url.host_str().ok_or("the kernel origin has no host")?,
        url.port_or_known_default()
            .ok_or("the kernel origin has no port")?
    );
    let request = build_request(method, &authority, path, &state.token, body);
    let mut stream = std::net::TcpStream::connect(&authority)
        .map_err(|e| format!("reaching the kernel: {e}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("sending the kernel request: {e}"))?;
    // Half-close the request side: with Content-Length the host needs no
    // more bytes, and a reader that waits for end-of-request is never left
    // guessing where the body ends.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader
        .read_line(&mut status)
        .map_err(|e| format!("reading the kernel response: {e}"))?;
    if !status.contains(" 2") {
        return Err(format!("the kernel refused {}: {}", path, status.trim()));
    }
    let mut body = Vec::new();
    reader
        .read_to_end(&mut body)
        .map_err(|e| format!("reading the kernel response body: {e}"))?;
    let text = String::from_utf8_lossy(&body);
    Ok(text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or(text.to_string()))
}

// The page's window onto the kernel — the host's HTTP API, with the
// credential added by this shell and never by the page.
#[tauri::command]
fn kernel_get(state: State<KernelState>, path: String) -> Result<String, String> {
    kernel_request(&state, "GET", &path, None)
}

#[tauri::command]
fn kernel_post(state: State<KernelState>, path: String, body: String) -> Result<String, String> {
    kernel_request(&state, "POST", &path, Some(&body))
}

fn main() {
    if let Err(err) = run() {
        panic!("Failed to setup app: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;

    #[test]
    fn handshake_parses_the_host_line() {
        let mut line = Cursor::new(
            "{\"version\":1,\"origin\":\"http://127.0.0.1:5281\",\"token\":\"t3\"}\n".to_string(),
        );
        let bound = read_handshake(&mut line).expect("parses");
        assert_eq!(bound.origin, "http://127.0.0.1:5281");
        assert_eq!(bound.token, "t3");
    }

    #[test]
    fn handshake_refuses_unknown_versions() {
        let mut line =
            Cursor::new("{\"version\":2,\"origin\":\"x\",\"token\":\"t\"}\n".to_string());
        assert!(read_handshake(&mut line).is_err());
    }

    #[test]
    fn handshake_refuses_an_ended_pipe() {
        let mut line = Cursor::new(String::new());
        assert!(read_handshake(&mut line).is_err());
    }

    // The request builder is the credential boundary: the cookie and the
    // naming Origin are what the loopback gate reads, and neither may leak
    // into what the page composes.
    #[test]
    fn build_request_carries_the_credential_and_names_the_listener() {
        let request = build_request("GET", "127.0.0.1:5281", "/sessions", "t3", None);
        assert!(request.starts_with("GET /sessions HTTP/1.1\r\n"));
        assert!(request.contains("Host: 127.0.0.1:5281\r\n"));
        assert!(request.contains("Origin: http://127.0.0.1:5281\r\n"));
        assert!(request.contains("Cookie: reasonix_token=t3\r\n"));
        assert!(request.contains("Connection: close\r\n"));
        assert!(!request.contains("Content-Length"));
    }

    #[test]
    fn build_request_carries_the_body_with_its_length() {
        let request = build_request(
            "POST",
            "127.0.0.1:5281",
            "/turns",
            "t3",
            Some("{\"task\":\"x\"}"),
        );
        assert!(request.starts_with("POST /turns HTTP/1.1\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        let body = "{\"task\":\"x\"}";
        assert!(request.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(request.ends_with(body));
    }

    #[test]
    fn parse_sse_line_reads_ids_data_and_skips_the_rest() {
        assert!(matches!(parse_sse_line("id: 42"), SseLine::Id(42)));
        assert!(matches!(
            parse_sse_line("data: {\"kind\":\"text\"}"),
            SseLine::Data(_)
        ));
        assert!(matches!(parse_sse_line(": ping"), SseLine::Other));
        assert!(matches!(parse_sse_line(""), SseLine::Other));
    }

    // The credential boundary of the stream: the frames ride the cookie the
    // shell already holds, never a query token. The listener closes its side
    // after the GET (which carries no body), and the clean end is what the
    // function reports.
    #[test]
    fn events_request_carries_the_cookie_and_resume_point() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            stream.read_to_end(&mut request).expect("read");
            String::from_utf8_lossy(&request).to_string()
        });
        let frames: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let sink = |payload: &str| {
            frames.lock().expect("frames").push(payload.to_string());
            Ok(())
        };
        let _ = events_stream(&sink, "http://127.0.0.1:PORT", "t3", &mut 7i64);
        let request = handle.join().expect("the mock thread");
        assert!(request.contains("GET /events HTTP/1.1\r\n"));
        assert!(request.contains("Cookie: reasonix_token=t3\r\n"));
        assert!(request.contains("Last-Event-ID: 7\r\n"));
        assert!(frames.lock().expect("frames").is_empty());
    }

    // The kernel_request round trip runs against a real loopback listener so
    // the boundary — credential headers in, response body out — is the thing
    // under test. The response is written before the assertions: a failing
    // assert panics this thread and the client must never be left waiting on
    // a response that will not come.
    #[test]
    fn kernel_request_round_trips_the_credential() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            // The client half-closes its side after the request, so the whole
            // request arrives as one read: no line parsing, no place to block.
            let mut request = Vec::new();
            stream.read_to_end(&mut request).expect("read");
            let request = String::from_utf8_lossy(&request).to_string();
            assert!(request.starts_with("GET /health HTTP/1.1\r\n"));
            assert!(request.contains("Cookie: reasonix_token=t3\r\n"));
            assert!(request.contains(&format!("Origin: http://127.0.0.1:{port}\r\n")));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .expect("write");
        });
        let state = KernelState {
            origin: format!("http://127.0.0.1:{port}").to_string(),
            token: "t3".to_string(),
            child: Mutex::new(spawn_stub_child()),
        };
        let body = kernel_request(&state, "GET", "/health", None).expect("round trips");
        assert_eq!(body, "ok");
        handle.join().expect("the mock thread");
    }

    fn spawn_stub_child() -> Child {
        // The liveness state only needs a live Child to try_wait; the command
        // is irrelevant and the stub re-lists the test binary itself — the one
        // executable that exists on every platform the tests run on.
        Command::new(std::env::current_exe().expect("current exe"))
            .arg("--list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn")
    }
}
