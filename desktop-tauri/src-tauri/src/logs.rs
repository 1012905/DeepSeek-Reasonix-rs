// The shell's own log and the host's stderr, under the app's log directory. A
// launch from a shortcut has no console, so these files are the only place a
// failure before the window can be read back from. The previous launch's pair
// is kept beside them.
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const TAIL_BYTES: usize = 4096;

pub struct Logs {
    shell: Mutex<Option<File>>,
    host: Mutex<Option<File>>,
    tail: Mutex<VecDeque<u8>>,
}

fn rotate(dir: &Path, name: &str) -> Option<File> {
    let path = dir.join(name);
    let _ = std::fs::rename(&path, dir.join(format!("{name}.1")));
    File::create(path).ok()
}

impl Logs {
    pub fn open(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            shell: Mutex::new(rotate(&dir, "shell.log")),
            host: Mutex::new(rotate(&dir, "host.log")),
            tail: Mutex::new(VecDeque::with_capacity(TAIL_BYTES)),
        }
    }

    pub fn line(&self, text: &str) {
        if cfg!(debug_assertions) {
            eprintln!("shell: {text}");
        }
        if let Ok(mut file) = self.shell.lock()
            && let Some(file) = file.as_mut()
        {
            let _ = writeln!(file, "{text}");
        }
    }

    pub fn host(&self, bytes: &[u8]) {
        if cfg!(debug_assertions) {
            let _ = std::io::stderr().write_all(bytes);
        }
        if let Ok(mut file) = self.host.lock()
            && let Some(file) = file.as_mut()
        {
            let _ = file.write_all(bytes);
        }
        if let Ok(mut tail) = self.tail.lock() {
            tail.extend(bytes);
            let over = tail.len().saturating_sub(TAIL_BYTES);
            tail.drain(..over);
        }
    }

    /// What the host said last, for a launch that failed before its window.
    /// The launch credential joins the redaction list before anything here
    /// reaches a dialog (host.md promises the shell redacts it).
    pub fn host_tail(&self) -> String {
        self.tail
            .lock()
            .map(|tail| {
                redact_token(&String::from_utf8_lossy(
                    &tail.iter().copied().collect::<Vec<_>>(),
                ))
                .trim()
                .to_string()
            })
            .unwrap_or_default()
    }
}

/// Drops the value of every `reasonix_token` cookie spelling in the text —
/// the value runs to the next `;` or whitespace.
fn redact_token(text: &str) -> String {
    const MARK: &str = "reasonix_token=";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        out.push_str(&rest[..at]);
        out.push_str(MARK);
        out.push_str("[redacted]");
        rest = &rest[at + MARK.len()..];
        let end = rest
            .find(|c: char| c == ';' || c.is_whitespace())
            .unwrap_or(rest.len());
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_redacts_the_launch_credential() {
        let text = "started token reasonix_token=abc123; ok\nreasonix_token=xyz456 end";
        assert_eq!(
            redact_token(text),
            "started token reasonix_token=[redacted]; ok\nreasonix_token=[redacted] end"
        );
        assert_eq!(redact_token("nothing here"), "nothing here");
    }

    #[test]
    fn the_tail_keeps_only_the_last_words() {
        let dir = crate::scratch("logs");
        let logs = Logs::open(dir.clone());
        logs.host(&vec![b'a'; TAIL_BYTES]);
        logs.host(b"last words\n");
        let tail = logs.host_tail();
        assert!(tail.ends_with("last words"));
        assert!(tail.len() <= TAIL_BYTES);
        assert!(dir.join("host.log").exists());
        drop(logs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
