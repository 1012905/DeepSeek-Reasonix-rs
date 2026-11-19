//! Passes stderr through and remembers the last `reasonix-studio-host:` line,
//! which is where `run` says why it is returning a failure code.

use std::io::Write;
use std::sync::Mutex;

pub struct LastLine<W: Write> {
    inner: W,
    line: Mutex<String>,
}

const PREFIX: &str = "reasonix-studio-host:";

impl<W: Write> LastLine<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, line: Mutex::new(String::new()) }
    }

    pub fn last(&self) -> String {
        let stored = self.line.lock().unwrap_or_else(|p| p.into_inner());
        if stored.is_empty() {
            "no reason was reported".to_string()
        } else {
            stored.clone()
        }
    }
}

impl<W: Write> Write for LastLine<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(&buf.clone());
        for line in text.trim_end_matches(['\r', '\n']).split('\n') {
            if line.starts_with(PREFIX) {
                let mut stored = self.line.lock().unwrap_or_else(|p| p.into_inner());
                *stored = line.trim().to_string();
            }
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_the_last_prefixed_line_and_passes_everything_through() {
        let mut buf = Vec::new();
        {
            let mut l = LastLine::new(&mut buf);
            write!(l, "noise\n").unwrap();
            write!(l, "reasonix-studio-host: startup phase=page elapsed_ms=3\n").unwrap();
            write!(l, "reasonix-studio-host: bind failed\r\nmore noise\n").unwrap();
            assert_eq!(l.last(), "reasonix-studio-host: bind failed");
        }
        assert!(String::from_utf8(buf).unwrap().contains("more noise"));
        assert_eq!(LastLine::new(Vec::new()).last(), "no reason was reported");
    }
}
