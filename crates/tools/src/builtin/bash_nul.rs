//! The file Git Bash leaves when a Windows habit meets an MSYS path
//! (parity target: `bash_nul.go`). MSYS programs take NUL as an ordinary
//! path, so output meant to be discarded lands in a file the host can name
//! the cause of.

#![forbid(unsafe_code)]

use std::path::Path;

use reasonix_safety::sandbox::shell::ShellKind;

/// Notices a regular file named NUL that a Git Bash command created in its
/// working directory. Arming only under Windows bash, and only when no NUL
/// file is there already: one the command did not create is not its to
/// report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NulWatch {
    dir: String,
    armed: bool,
}

pub fn arm_nul_watch(sh_kind: ShellKind, dir: &str) -> NulWatch {
    if !cfg!(windows) || sh_kind != ShellKind::Bash || dir.is_empty() || nul_file_in(dir) {
        return NulWatch::default();
    }
    NulWatch {
        dir: dir.to_owned(),
        armed: true,
    }
}

impl NulWatch {
    /// The note the command's output carries, or "" when nothing fired.
    pub fn note(&self) -> String {
        if !self.armed || !nul_file_in(&self.dir) {
            return String::new();
        }
        format!(
            "This command created a file named NUL in {}: Git Bash programs take NUL as an ordinary path \
             (ssh -o UserKnownHostsFile=NUL, curl -o NUL), so nothing was discarded. \
             Remove it with rm ./NUL and use /dev/null instead.",
            self.dir
        )
    }

    /// Whether the watch is armed.
    pub fn is_armed(&self) -> bool {
        self.armed
    }
}

/// Reads the directory rather than stat-ing the name: every Windows path
/// API resolves NUL to the device, so a stat answers for the device.
pub fn nul_file_in(dir: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(Path::new(dir)) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name().to_string_lossy().eq_ignore_ascii_case("NUL")
            && e.file_type().map(|t| t.is_file()).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> tempfile::TempDir {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(&base)
            .unwrap()
    }

    fn make_nul(dir: &str) {
        // On Windows only the extended-length form reaches the name; every
        // other path resolves it to the device. The prefix demands a fully
        // normalized absolute path, so the directory is canonicalized
        // first (canonicalize itself returns the \\?\ form).
        let path = if cfg!(windows) {
            let abs = std::fs::canonicalize(dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            format!("{abs}\\NUL")
        } else {
            format!("{dir}/NUL")
        };
        std::fs::write(&path, b"x").unwrap();
    }

    #[test]
    fn nul_watch_reports_only_a_file_the_command_created() {
        let dir = tmp("bash-nul-arm");
        let dir = dir.path().to_string_lossy().into_owned();
        let w = arm_nul_watch(ShellKind::Bash, &dir);
        let armed_expected = cfg!(windows);
        assert_eq!(w.is_armed(), armed_expected, "arms only on Windows bash");
        if !armed_expected {
            return;
        }
        assert_eq!(w.note(), "", "reported a NUL file before any was created");
        make_nul(&dir);
        let note = w.note();
        assert!(note.contains("/dev/null"), "note = {note}");
        assert!(note.contains(&dir), "note names the directory: {note}");
        assert!(
            !arm_nul_watch(ShellKind::Bash, &dir).is_armed(),
            "armed over a NUL file that was already there"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nul_watch_arms_only_where_msys_runs() {
        let dir = tmp("bash-nul-noarm");
        let dir = dir.path().to_string_lossy().into_owned();
        let pw = arm_nul_watch(ShellKind::PowerShell, &dir);
        assert!(!pw.is_armed(), "PowerShell is not an MSYS shell");
        assert_eq!(pw, NulWatch::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
