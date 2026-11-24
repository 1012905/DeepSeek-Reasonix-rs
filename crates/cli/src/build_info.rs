//! Machine- and human-readable build identity for `reasonix version` and
//! `reasonix --version`.
//!
//! Parity target: `internal/frontend/cli/buildinfo.go`. Release builds inject
//! the real identity through compile-time environment variables, mirroring the
//! reference `-ldflags` flow where unset fields fall back to `dev` / `unknown`.

/// Version reported by bare source builds. Release builds inject the real
/// version via `REASONIX_VERSION` at compile time, mirroring the reference
/// `-ldflags` flow where `version` defaults to `"dev"`.
pub const DEFAULT_VERSION: &str = "dev";

/// Build identity reported by `reasonix version` and `reasonix --version`.
///
/// Fields are limited to version, commit, build time, and target; no config
/// paths and no fixed timezone conversions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// Version string; the single-line contract for `--version`.
    pub version: String,
    /// Git commit hash, when injected at build time.
    pub git_commit: String,
    /// Build time (RFC 3339, UTC), when injected at build time.
    pub build_time_utc: String,
    /// Rust target triple, when injected at build time.
    pub build_target: String,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::from_build()
    }
}

impl BuildInfo {
    /// Reads build identity from compile-time environment. A bare source
    /// build reports `dev` with empty metadata — identical to a reference
    /// binary built without `-ldflags`.
    pub fn from_build() -> Self {
        macro_rules! inject {
            ($var:literal, $default:expr) => {
                match option_env!($var) {
                    Some(value) if !value.trim().is_empty() => value.trim(),
                    _ => $default,
                }
            };
        }
        Self {
            version: inject!("REASONIX_VERSION", DEFAULT_VERSION).to_owned(),
            git_commit: inject!("REASONIX_GIT_COMMIT", "").to_owned(),
            build_time_utc: inject!("REASONIX_BUILD_TIME_UTC", "").to_owned(),
            build_target: inject!("REASONIX_BUILD_TARGET", "").to_owned(),
        }
    }

    /// Fills reference defaults: `dev` version, `os/arch` target, `unknown`
    /// commit and build time. Mirrors `BuildInfo.withDefaults`; the Rust side
    /// has no embedded VCS revision to fall back on, so an unset commit stays
    /// `unknown` rather than being recovered from build settings.
    pub fn with_defaults(&self) -> Self {
        let mut b = self.clone();
        if b.version.trim().is_empty() {
            b.version = DEFAULT_VERSION.to_owned();
        }
        if b.build_target.trim().is_empty() {
            b.build_target = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
        }
        if b.git_commit.trim().is_empty() {
            b.git_commit = "unknown".to_owned();
        }
        if b.build_time_utc.trim().is_empty() {
            b.build_time_utc = "unknown".to_owned();
        }
        b
    }

    /// Single-line form: `reasonix <version>`.
    pub fn single_line(&self) -> String {
        let b = self.with_defaults();
        format!("reasonix {}", b.version.trim())
    }

    /// Verbose human body: the single line followed by the fixed field
    /// sequence `git_commit`, `build_time_utc`, `build_target`, `go`.
    ///
    /// The reference's `go` line carries `runtime.Version() GOOS/GOARCH`; the
    /// Rust kernel has no Go runtime to report and BREAKS.md has no substitution
    /// decision yet (docs/contracts/cli.md §9.5), so the field is retained with
    /// the `unknown` value instead of being dropped. Verbose keeps default
    /// `unknown` fields (unlike the JSON path, which drops them).
    pub fn verbose_text(&self) -> String {
        let b = self.with_defaults();
        let mut lines = vec![b.single_line()];
        let mut append = |key: &str, value: &str| {
            let value = value.trim();
            if !value.is_empty() {
                lines.push(format!("{key}: {value}"));
            }
        };
        append("git_commit", &b.git_commit);
        append("build_time_utc", &b.build_time_utc);
        append("build_target", &b.build_target);
        append("go", RUNTIME_FIELD);
        format!("{}\n", lines.join("\n"))
    }

    /// Indented JSON payload. Commit, build time, and the runtime field are
    /// omitted when empty or `unknown` (reference `omitUnknown`); `build_target`
    /// always serializes. The `go` runtime field follows the same pending
    /// BREAKS.md decision as [`Self::verbose_text`].
    pub fn json_payload(&self) -> String {
        let b = self.with_defaults();
        let mut fields = vec![json_pair("version", &b.version)];
        let commit = omit_unknown(&b.git_commit);
        if !commit.is_empty() {
            fields.push(json_pair("git_commit", &commit));
        }
        let build_time = omit_unknown(&b.build_time_utc);
        if !build_time.is_empty() {
            fields.push(json_pair("build_time_utc", &build_time));
        }
        fields.push(json_pair("build_target", &b.build_target));
        let runtime = omit_unknown(RUNTIME_FIELD);
        if !runtime.is_empty() {
            fields.push(json_pair("go", &runtime));
        }
        format!("{{\n  {}\n}}\n", fields.join(",\n  "))
    }
}

/// Placeholder for the reference `go: <runtime.Version()> GOOS/GOARCH` line.
/// Rust has no Go runtime; the substitution decision is recorded as pending
/// in docs/parity/BREAKS.md (ledger CLI-005).
const RUNTIME_FIELD: &str = "unknown";

fn omit_unknown(v: &str) -> String {
    let v = v.trim();
    if v.is_empty() || v == "unknown" {
        String::new()
    } else {
        v.to_owned()
    }
}

fn json_pair(key: &str, value: &str) -> String {
    format!("\"{key}\": {}", json_string(value))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> BuildInfo {
        BuildInfo {
            version: "9.9.9-test".into(),
            git_commit: String::new(),
            build_time_utc: String::new(),
            build_target: String::new(),
        }
    }

    #[test]
    fn single_line_format_matches_reference() {
        assert_eq!(info().single_line(), "reasonix 9.9.9-test");
    }

    #[test]
    fn version_trims_whitespace() {
        let mut i = info();
        i.version = "  1.2.3  ".into();
        assert_eq!(i.single_line(), "reasonix 1.2.3");
    }

    #[test]
    fn default_build_info_reports_dev() {
        assert_eq!(BuildInfo::from_build().version, DEFAULT_VERSION);
    }

    #[test]
    fn verbose_lists_injected_fields_and_keeps_unknown_runtime_line() {
        let i = BuildInfo {
            version: "v9.9.9".into(),
            git_commit: "abcdef0123456789".into(),
            build_time_utc: "2026-08-06T12:00:00Z".into(),
            build_target: "darwin/arm64".into(),
        };
        let text = i.verbose_text();
        assert!(text.contains("reasonix v9.9.9"));
        assert!(text.contains("git_commit: abcdef0123456789"));
        assert!(text.contains("build_time_utc: 2026-08-06T12:00:00Z"));
        assert!(text.contains("build_target: darwin/arm64"));
        // Verbose keeps the runtime field even though its Rust value is unknown.
        assert!(text.contains("go: unknown"));
        assert!(!text.contains("config_path"));
        assert!(!text.contains("CST"));
    }

    #[test]
    fn json_omits_unknown_metadata_and_runtime_field() {
        let i = BuildInfo {
            version: "v1.0.0".into(),
            git_commit: "unknown".into(),
            build_time_utc: "".into(),
            build_target: "linux/amd64".into(),
        };
        let payload = i.json_payload();
        assert!(payload.contains("\"version\": \"v1.0.0\""));
        assert!(payload.contains("\"build_target\": \"linux/amd64\""));
        assert!(!payload.contains("git_commit"));
        assert!(!payload.contains("build_time_utc"));
        assert!(!payload.contains("\"go\""));
    }
}
