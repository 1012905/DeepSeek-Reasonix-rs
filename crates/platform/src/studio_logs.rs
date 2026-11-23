//! Studio log layout (paths.md §14, config.md §12.6, CONFIG_PATHS.md
//! "Logs and crash files").
//!
//! Studio logs sit beside the window profile, never in the Reasonix home. The
//! shell owns the OS base directory and the instance segment; this slice lays
//! them out once both are injected. The kernel cache root itself is resolved
//! by reasonix-config (REASONIX_CACHE_HOME, pinned-home fallback) and handed in
//! by the caller; this slice does not re-derive it.

use std::path::{Path, PathBuf};

pub const STUDIO_APP_DIR_NAME: &str = "Reasonix Studio";
/// Prefix of the instance id (`io.reasonix.studio.<16hex>`, host.md §5.1).
pub const INSTANCE_PREFIX: &str = "io.reasonix.studio";
pub const LOGS_DIR_NAME: &str = "logs";
pub const CRASH_DIR_NAME: &str = "crash";
pub const SHELL_LOG_NAME: &str = "shell.log";
pub const HOST_LOG_NAME: &str = "host.log";
pub const FATAL_LOG_PREFIX: &str = "host-";
pub const FATAL_LOG_SUFFIX: &str = ".log";

/// Newest fatal logs kept (Go `keepFatalLogs`).
pub const FATAL_LOG_KEEP: usize = 5;
/// Survivor tail retained when a fatal log is cut down (Go `maxFatalLogBytes`).
pub const MAX_FATAL_LOG_BYTES: u64 = 256 << 10;

/// `<os-base>/Reasonix Studio/<instance>/logs` (paths.md §14).
pub fn studio_logs_dir(os_base: &Path, instance: &str) -> PathBuf {
    os_base
        .join(STUDIO_APP_DIR_NAME)
        .join(instance)
        .join(LOGS_DIR_NAME)
}

/// `<logs>/crash` — fatal runtime dumps live one level under the logs dir.
pub fn studio_crash_dir(logs_dir: &Path) -> PathBuf {
    logs_dir.join(CRASH_DIR_NAME)
}

/// Name of one fatal runtime dump: `host-<utc-time>-<version>-<pid>.log`
/// (Go `InstallFatalLog` + `pathSafe`). `time` is the already-formatted UTC
/// stamp; the version is sanitized to a safe path component.
pub fn fatal_log_name(time: &str, version: &str, pid: u32) -> String {
    format!(
        "{FATAL_LOG_PREFIX}{time}-{}-{pid}{FATAL_LOG_SUFFIX}",
        version_path_safe(version)
    )
}

fn version_path_safe(version: &str) -> String {
    let mut out: String = version
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        out = "unknown".to_string();
    }
    if out.len() > 32 {
        out.truncate(32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_dir_layout_matches_reference() {
        let base = Path::new("/home/user/Library/Application Support");
        let logs = studio_logs_dir(base, "io.reasonix.studio.ab12cd34ef567890");
        assert_eq!(
            logs,
            base.join("Reasonix Studio")
                .join("io.reasonix.studio.ab12cd34ef567890")
                .join("logs")
        );
    }

    #[test]
    fn crash_dir_sits_under_logs() {
        let logs = Path::new("/app/Reasonix Studio/io.reasonix.studio.x/logs");
        assert_eq!(studio_crash_dir(logs), logs.join(CRASH_DIR_NAME));
    }

    #[test]
    fn fatal_log_name_follows_reference_pattern() {
        let name = fatal_log_name("20260102T150405Z", "0.3.0", 1234);
        assert_eq!(name, "host-20260102T150405Z-0.3.0-1234.log");
    }

    #[test]
    fn fatal_version_is_sanitized_to_path_safe_component() {
        assert_eq!(
            fatal_log_name("20260102T150405Z", "0.3.0-beta.1+dev", 7),
            "host-20260102T150405Z-0.3.0-beta.1-dev-7.log"
        );
        assert_eq!(
            fatal_log_name("20260102T150405Z", "   ", 7),
            "host-20260102T150405Z-unknown-7.log"
        );
        let long = "a".repeat(40);
        let name = fatal_log_name("20260102T150405Z", &long, 7);
        let version = name
            .strip_prefix("host-20260102T150405Z-")
            .unwrap()
            .strip_suffix("-7.log")
            .unwrap();
        assert_eq!(version.len(), 32);
    }

    #[test]
    fn retention_constants_match_reference() {
        assert_eq!(FATAL_LOG_KEEP, 5);
        assert_eq!(MAX_FATAL_LOG_BYTES, 256 * 1024);
    }

    #[test]
    fn layout_and_file_name_constants_match_contract() {
        assert_eq!(STUDIO_APP_DIR_NAME, "Reasonix Studio");
        assert_eq!(INSTANCE_PREFIX, "io.reasonix.studio");
        assert_eq!(LOGS_DIR_NAME, "logs");
        assert_eq!(CRASH_DIR_NAME, "crash");
        assert_eq!(SHELL_LOG_NAME, "shell.log");
        assert_eq!(HOST_LOG_NAME, "host.log");
        assert_eq!(FATAL_LOG_PREFIX, "host-");
        assert_eq!(FATAL_LOG_SUFFIX, ".log");
    }
}
