//! Writes stderr and remembers the last `reasonix-studio-host:` line, which
//! is where `run` says why it is returning a failure code. Shareable: every
//! write takes the stderr lock, so no handle is held across threads.

use std::io::Write;
use std::sync::Mutex;

const PREFIX: &str = "reasonix-studio-host:";

pub struct LastLine {
    line: Mutex<String>,
}

impl LastLine {
    pub fn new() -> Self {
        Self {
            line: Mutex::new(String::new()),
        }
    }

    // Consumed by the fatal-log recording (HOST-007), landing with the
    // platform slice.
    #[allow(dead_code)]
    pub fn last(&self) -> String {
        let stored = self.line.lock().unwrap_or_else(|p| p.into_inner());
        if stored.is_empty() {
            "no reason was reported".to_string()
        } else {
            stored.clone()
        }
    }

    /// Tracks prefixed lines and passes everything through to stderr.
    pub fn write_bytes(&self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        for line in text.trim_end_matches(['\r', '\n']).split('\n') {
            if line.starts_with(PREFIX) {
                let mut stored = self.line.lock().unwrap_or_else(|p| p.into_inner());
                *stored = line.trim().to_string();
            }
        }
        let mut err = std::io::stderr().lock();
        err.write(buf)
    }
}

impl Write for LastLine {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_bytes(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_the_last_prefixed_line() {
        let l = LastLine::new();
        l.write_bytes(b"noise\n").unwrap();
        assert_eq!(l.last(), "no reason was reported");
        l.write_bytes(b"reasonix-studio-host: startup phase=page elapsed_ms=3\n")
            .unwrap();
        assert_eq!(
            l.last(),
            "reasonix-studio-host: startup phase=page elapsed_ms=3"
        );
        l.write_bytes(b"reasonix-studio-host: bind failed\r\nmore noise\n")
            .unwrap();
        assert_eq!(l.last(), "reasonix-studio-host: bind failed");
    }
}
