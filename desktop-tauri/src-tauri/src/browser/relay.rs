// Holds the kernel's browser stream open for as long as the shell runs, and
// posts frames back one request at a time: an event that arrived before a
// reply has to reach the kernel before it.
use crate::client::KernelClient;
use serde_json::Value;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

const RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(10);

/// Splits what arrived into complete data payloads and the unfinished
/// remainder. Comments and blank keep-alives carry nothing.
pub fn sse_data(buffered: &str) -> (Vec<String>, String) {
    let mut out = Vec::new();
    let mut rest = buffered;
    while let Some(end) = rest.find("\n\n") {
        let block = &rest[..end];
        rest = &rest[end + 2..];
        let data = block
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() {
            out.push(data);
        }
    }
    (out, rest.to_string())
}

#[derive(Clone)]
pub struct Relay {
    tx: mpsc::Sender<Value>,
    stopped: Arc<AtomicBool>,
}

impl Relay {
    pub fn post(&self, frame: Value) {
        let _ = self.tx.send(frame);
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

pub fn start(
    client: KernelClient,
    on_frame: impl Fn(Value) + Send + 'static,
    on_drop: impl Fn() + Send + 'static,
) -> Relay {
    let stopped = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Value>();
    let poster = client.clone();
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut frames = vec![first];
            while let Ok(next) = rx.try_recv() {
                frames.push(next);
            }
            let body = Value::Array(frames).to_string();
            // The kernel is going away; what it did not take goes with it.
            let _ = poster.post("/browser-host/frames", &body);
        }
    });
    let flag = stopped.clone();
    std::thread::spawn(move || {
        let mut retry = RETRY;
        loop {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(mut reader) = client.stream("/browser-host/stream") {
                retry = RETRY;
                let mut buffered = String::new();
                let mut buf = [0u8; 8192];
                loop {
                    if flag.load(Ordering::SeqCst) {
                        return;
                    }
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            buffered.push_str(&String::from_utf8_lossy(&buf[..n]));
                            let (data, rest) = sse_data(&buffered);
                            buffered = rest;
                            for payload in data {
                                // A frame this process cannot read is one it
                                // must not act on.
                                if let Ok(frame) = serde_json::from_str::<Value>(&payload) {
                                    on_frame(frame);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            if flag.load(Ordering::SeqCst) {
                return;
            }
            on_drop();
            std::thread::sleep(retry);
            retry = (retry * 2).min(MAX_RETRY);
        }
    });
    Relay { tx, stopped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_blocks_split_from_the_remainder() {
        let (data, rest) = sse_data("data: {\"a\":1}\n\ndata: two\n");
        assert_eq!(data, vec!["{\"a\":1}"]);
        assert_eq!(rest, "data: two\n");
    }

    #[test]
    fn comments_and_keepalives_carry_nothing() {
        let (data, rest) = sse_data(": ping\n\nevent: x\ndata: one\ndata: two\n\n");
        assert_eq!(data, vec!["one\ntwo"]);
        assert_eq!(rest, "");
    }

    #[test]
    fn an_empty_buffer_stays_empty() {
        let (data, rest) = sse_data("");
        assert!(data.is_empty() && rest.is_empty());
    }
}
