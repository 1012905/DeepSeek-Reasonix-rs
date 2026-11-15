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
    pub fn host_tail(&self) -> String {
        self.tail
            .lock()
            .map(|tail| {
                String::from_utf8_lossy(&tail.iter().copied().collect::<Vec<_>>())
                    .trim()
                    .to_string()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
