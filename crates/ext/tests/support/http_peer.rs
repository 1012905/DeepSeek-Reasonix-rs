//! Hermetic HTTP/1.1 peer for MCP transport tests.
//!
//! No third-party HTTP stack: a `std::net::TcpListener` accepts on an
//! ephemeral loopback port and one OS thread serves each connection. The same
//! peer backs both the Streamable HTTP tests now and the legacy SSE wave's
//! long-lived GET stream later: handlers can answer a JSON body, start an SSE
//! stream they keep open, or read the raw socket.
//!
//! Every received request is recorded under a mutex so assertions can inspect
//! method, headers, and body after the fact.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One decoded inbound request. Header keys are stored lowercased.
#[derive(Clone, Debug, Default)]
pub struct PeerRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl PeerRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// A recorded request, kept for assertions.
#[derive(Clone, Debug, Default)]
pub struct LoggedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl LoggedRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Debug, Default)]
struct PeerLog {
    requests: Vec<LoggedRequest>,
}

/// What a test does with one connection.
pub trait PeerHandler: Send + Sync + 'static {
    fn handle(&self, req: &PeerRequest, conn: &mut PeerConn);
}

impl<F> PeerHandler for F
where
    F: Fn(&PeerRequest, &mut PeerConn) + Send + Sync + 'static,
{
    fn handle(&self, req: &PeerRequest, conn: &mut PeerConn) {
        self(req, conn);
    }
}

/// One connection handed to the handler.
pub struct PeerConn {
    stream: TcpStream,
}

/// SSE writer over an open connection: `event:`/`data:` frames per event.
pub struct SseWriter {
    stream: TcpStream,
}

impl SseWriter {
    /// Writes one SSE event terminated by a blank line, then flushes.
    pub fn send_event(&mut self, event: &str, data: &str) {
        let _ = write!(self.stream, "event: {event}\r\ndata: {data}\r\n\r\n");
        let _ = self.stream.flush();
    }

    /// Writes an SSE comment comment line, then flushes.
    pub fn comment(&mut self, text: &str) {
        let _ = writeln!(self.stream, ": {text}");
        let _ = self.stream.flush();
    }
}

/// One outbound SSE frame queued for a long-lived GET stream.
#[derive(Clone, Debug)]
pub struct SseFrame {
    pub event: String,
    pub data: String,
}

/// Shared frame queue between the GET connection handler and whatever produces
/// later events (a POST handler answering a call, the test script). The GET
/// handler drains it; producers push through the shared bus. Dropping every
/// sender releases the waiting GET handler.
pub struct SseBus {
    tx: mpsc::Sender<SseFrame>,
    rx: Mutex<mpsc::Receiver<SseFrame>>,
}

impl Default for SseBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SseBus {
    /// Binds a fresh queue pair.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }

    /// Queues one frame for the open GET stream.
    pub fn push(&self, event: &str, data: &str) {
        let _ = self.tx.send(SseFrame {
            event: event.to_string(),
            data: data.to_string(),
        });
    }

    /// Blocks the GET handler until the next frame arrives; `None` once every
    /// sender is gone.
    pub fn drain_next(&self) -> Option<SseFrame> {
        let rx = self.rx.lock().expect("sse bus mutex");
        rx.recv().ok()
    }
}

impl PeerConn {
    /// Answers with a fixed status and body; closes the connection after.
    pub fn respond(&mut self, status: u16, headers: &[(&str, &str)], body: &[u8]) {
        let reason = reason_phrase(status);
        let mut head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = self.stream.write_all(head.as_bytes());
        let _ = self.stream.write_all(body);
        let _ = self.stream.flush();
    }

    /// Starts a `text/event-stream` answer and hands the stream to the handler.
    pub fn start_sse(&mut self) -> SseWriter {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
        let _ = self.stream.write_all(head.as_bytes());
        let _ = self.stream.flush();
        SseWriter {
            stream: self.stream.try_clone().expect("clone peer stream"),
        }
    }

    /// Raw access for hand-written framing a test needs.
    pub fn raw(&mut self) -> &mut TcpStream {
        &mut self.stream
    }
}

/// A loopback peer listening on an ephemeral port.
pub struct HttpPeer {
    base_url: String,
    log: Arc<Mutex<PeerLog>>,
    listener: Arc<TcpListener>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl HttpPeer {
    /// Binds the listener and starts the accept thread.
    pub fn new(handler: impl PeerHandler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer port");
        listener
            .set_nonblocking(false)
            .expect("peer listener blocking mode");
        let port = listener.local_addr().expect("peer local addr").port();
        let listener = Arc::new(listener);
        let log = Arc::new(Mutex::new(PeerLog::default()));
        let handler = Arc::new(handler);
        let accept = listener.clone();
        let accept_log = log.clone();
        let accept_handle = std::thread::spawn(move || {
            loop {
                let Ok((stream, _)) = accept.accept() else {
                    return;
                };
                stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
                stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
                let handler = handler.clone();
                let log = accept_log.clone();
                std::thread::spawn(move || {
                    serve_one(stream, handler, log);
                });
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            log,
            listener,
            handle: Mutex::new(Some(accept_handle)),
        }
    }

    /// The absolute base URL tests hand to the transport.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Snapshot of recorded requests in arrival order.
    pub fn requests(&self) -> Vec<LoggedRequest> {
        self.log.lock().expect("peer log mutex").requests.clone()
    }

    /// Stops the accept thread; open connections finish on their own.
    pub fn stop(self) {
        drop(self.listener);
        if let Some(handle) = self.handle.lock().expect("peer handle mutex").take() {
            let _ = handle.join();
        }
    }
}

fn serve_one(stream: TcpStream, handler: Arc<dyn PeerHandler>, log: Arc<Mutex<PeerLog>>) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone peer stream"));
    let Ok(req) = read_request(&mut reader) else {
        return;
    };
    {
        let mut guard = log.lock().expect("peer log mutex");
        guard.requests.push(LoggedRequest {
            method: req.method.clone(),
            path: req.path.clone(),
            headers: req.headers.clone(),
            body: req.body.clone(),
        });
    }
    let mut conn = PeerConn { stream };
    handler.handle(&req, &mut conn);
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Result<PeerRequest, ()> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|_| ())?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err(());
    }
    let mut headers = BTreeMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|_| ())?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).map_err(|_| ())?;
    }
    Ok(PeerRequest {
        method,
        path,
        headers,
        body,
    })
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

/// Unbounded reply channel helper kept for peers that answer asynchronously.
pub fn reply_channel<T>() -> (mpsc::Sender<T>, mpsc::Receiver<T>) {
    mpsc::channel()
}
