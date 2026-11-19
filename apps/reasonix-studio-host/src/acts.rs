//! Acts the host asks of the shell over the handshake pipe
//! (docs/contracts/host.md §3): relaunch/quit/show, one JSON line each, the
//! mutex keeping each act whole rather than interleaved with another.

use std::io::Write;
use std::sync::Mutex;

pub const ACT_RELAUNCH: &str = "relaunch";
pub const ACT_QUIT: &str = "quit";
pub const ACT_SHOW: &str = "show";

#[derive(serde::Serialize)]
struct Act<'a> {
    act: &'a str,
}

#[derive(Debug, thiserror::Error)]
#[error("ask the shell to {act}: {source}")]
pub struct ActError {
    act: String,
    #[source]
    source: std::io::Error,
}

/// The parent process, reached only through the pipe the handshake went out
/// on. Nothing is acknowledged: the shell performs each act by ending,
/// relaunching, or showing its window.
pub struct ShellOwner<W: Write> {
    to: Mutex<W>,
}

impl<W: Write> ShellOwner<W> {
    pub fn new(to: W) -> Self {
        Self { to: Mutex::new(to) }
    }

    /// The lock is for the line, not the writer: two acts interleaved inside
    /// one write would reach the parent as neither.
    pub fn ask(&self, act: &str) -> Result<(), ActError> {
        let body = serde_json::to_string(&Act { act }).map_err(|e| ActError {
            act: act.to_string(),
            source: std::io::Error::other(e),
        })?;
        let line = format!("{body}\n");
        let mut w = self.to.lock().unwrap_or_else(|p| p.into_inner());
        w.write_all(line.as_bytes())
            .and_then(|_| w.flush())
            .map_err(|source| ActError {
                act: act.to_string(),
                source,
            })
    }

    /// Nothing to do here, and that is the shape: ending is the third act.
    pub fn prepare_for_update(&self) {}

    /// Arming a restart would start a second application beside the one the
    /// Windows installer or the macOS helper brings up; Linux has nobody else.
    pub fn relaunch_after_update(&self) -> Result<(), ActError> {
        if cfg!(target_os = "linux") {
            self.ask(ACT_RELAUNCH)
        } else {
            Ok(())
        }
    }

    pub fn end_application(&self) {
        let _ = self.ask(ACT_QUIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn each_act_travels_as_its_own_line() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = SharedWriter(buf.clone());
        let owner = ShellOwner::new(writer);
        owner.ask(ACT_SHOW).unwrap();
        owner.ask(ACT_QUIT).unwrap();
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(out, "{\"act\":\"show\"}\n{\"act\":\"quit\"}\n");
    }

    #[test]
    fn concurrent_acts_still_arrive_whole() {
        // 64 threads asking at once: the mutex keeps each act on its own line,
        // so the reader sees 64 complete frames in some order (OPT-009).
        let buf = Arc::new(Mutex::new(Vec::new()));
        let owner = Arc::new(ShellOwner::new(SharedWriter(buf.clone())));
        let handles: Vec<_> = (0..64)
            .map(|i| {
                let owner = owner.clone();
                let act = if i % 2 == 0 { ACT_SHOW } else { ACT_QUIT };
                std::thread::spawn(move || owner.ask(act).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 64);
        assert!(
            lines
                .iter()
                .all(|l| *l == "{\"act\":\"show\"}" || *l == "{\"act\":\"quit\"}")
        );
        assert_eq!(
            out.lines().filter(|l| *l == "{\"act\":\"show\"}").count(),
            32
        );
        assert_eq!(
            out.lines().filter(|l| *l == "{\"act\":\"quit\"}").count(),
            32
        );
    }

    #[test]
    fn relaunch_is_asked_on_linux_only() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let owner = ShellOwner::new(SharedWriter(buf.clone()));
        owner.relaunch_after_update().unwrap();
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        if cfg!(target_os = "linux") {
            assert_eq!(out, "{\"act\":\"relaunch\"}\n");
        } else {
            assert_eq!(out, "");
        }
    }

    /// `Mutex<Vec<u8>>` itself is not `Write`, so tests share a buffer through
    /// this minimal wrapper.
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.lock().unwrap().flush()
        }
    }
}
