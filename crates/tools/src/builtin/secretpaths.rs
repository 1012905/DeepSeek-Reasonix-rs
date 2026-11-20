//! Credential and sensitive-path read denylists (parity target:
//! internal/base/secrets/credpaths.go, read-side half). A deny check that
//! fails open reads as protection, so both spellings of home are matched and
//! case folds on case-insensitive platforms.

#![forbid(unsafe_code)]

use super::pathgo;

/// `[secrets] protect_sensitive_files`. Go reads it from a process-global the
/// config writes; the Rust side keeps the same one-flag default-off shape so
/// tests flip it the same way.
static PROTECT_SENSITIVE_FILES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Sets the protect-sensitive-files denylist toggle (Go
/// `secrets.SetProtectSensitiveFiles`).
pub fn set_protect_sensitive_files(enabled: bool) {
    PROTECT_SENSITIVE_FILES.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

/// Whether the sensitive-name denylist is active (Go
/// `secrets.ProtectSensitiveFiles`).
pub fn protect_sensitive_files() -> bool {
    PROTECT_SENSITIVE_FILES.load(std::sync::atomic::Ordering::SeqCst)
}

/// Credential-file protection is always on in Go unless a test disables it;
/// the Rust side keeps the same constant-on shape.
pub const PROTECT_CREDENTIAL_FILES: bool = true;

/// Whether `abs` names a credential file (Go `secrets.CredentialReadPath`):
/// private keys under ~/.ssh (public names excepted) and the exact
/// credential stores. Files mixing secrets with settings are deliberately
/// absent — denying them breaks the tool that owns them.
pub fn credential_read_path(abs: &str) -> bool {
    if !PROTECT_CREDENTIAL_FILES {
        return false;
    }
    let clean = pathgo::clean(abs);
    let fold = fold_paths();
    for home in home_candidates() {
        if within_ssh_dir(&home, &clean, fold) {
            let name = file_base(&clean).to_lowercase();
            return !SSH_PUBLIC_NAMES.contains(&name.as_str()) && !name.ends_with(".pub");
        }
        for p in credential_exact_paths(&home) {
            if paths_equal(&clean, &p, fold) {
                return true;
            }
        }
    }
    false
}

/// The ~/.ssh entries carrying no private key material; keys take
/// user-chosen names, so "id_*" would miss them.
const SSH_PUBLIC_NAMES: [&str; 6] = [
    "config",
    "known_hosts",
    "known_hosts.old",
    "authorized_keys",
    "environment",
    "rc",
];

/// Both spellings of home, resolved and raw; matching only the resolved one
/// fails open for a caller that did not run its path through symlink
/// resolution (Go `homeCandidates`).
fn home_candidates() -> Vec<String> {
    match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        Some(raw) if !raw.is_empty() => {
            let raw = raw.to_string_lossy().into_owned();
            let resolved = std::fs::canonicalize(&raw)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| raw.clone());
            if resolved == raw {
                vec![raw]
            } else {
                vec![resolved, raw]
            }
        }
        _ => Vec::new(),
    }
}

fn within_ssh_dir(home: &str, clean: &str, fold: bool) -> bool {
    let dir = pathgo::join(&[home, ".ssh"]);
    let (dir, clean) = if fold {
        (dir.to_lowercase(), clean.to_lowercase())
    } else {
        (dir, String::from(clean))
    };
    match pathgo::rel(&dir, &clean) {
        Ok(r) => r != "." && !r.starts_with(".."),
        Err(_) => false,
    }
}

fn credential_exact_paths(home: &str) -> Vec<String> {
    [
        vec![home, ".aws", "credentials"],
        vec![home, ".pypirc"],
        vec![home, ".netrc"],
        vec![home, "_netrc"],
        vec![home, ".git-credentials"],
        vec![
            home,
            ".config",
            "gcloud",
            "application_default_credentials.json",
        ],
        vec![home, ".config", "gcloud", "credentials.db"],
        vec![home, ".config", "gcloud", "access_tokens.db"],
    ]
    .iter()
    .map(|parts| pathgo::join(parts))
    .collect()
}

/// Whether two paths name the same file, folded on case-insensitive
/// platforms (Go `pathsEqual`).
fn paths_equal(a: &str, b: &str, fold: bool) -> bool {
    if fold {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The home directories the protect toggle hides (Go
/// `secrets.SensitiveHomeDirs`): ~/.ssh.
pub fn sensitive_home_dirs() -> Vec<String> {
    home_candidates()
        .into_iter()
        .map(|home| pathgo::join(&[&home, ".ssh"]))
        .collect()
}

/// Whether a base name marks a file as holding secrets wherever it sits (Go
/// `secrets.SensitiveFileName`).
pub fn sensitive_file_name(name: &str) -> bool {
    let name = name.to_lowercase();
    match name.as_str() {
        ".env" | ".git-credentials" | ".netrc" | "id_rsa" | "id_dsa" | "id_ecdsa"
        | "id_ed25519" => return true,
        _ => {}
    }
    if let Some(rest) = name.strip_prefix(".env.") {
        return rest != "example" && rest != "sample" && rest != "template";
    }
    [".pem", ".key", ".p12", ".pfx"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// Whether deny-side path checks on this platform fold case (confine.go
/// `foldPaths`): NTFS and APFS are case-insensitive, so a case variant of a
/// forbidden path reaches the same bytes.
pub fn fold_paths() -> bool {
    cfg!(windows) || cfg!(target_os = "macos")
}

pub(crate) fn file_base(path: &str) -> String {
    let b = path.as_bytes();
    let mut i = b.len();
    while i > 0 && !pathgo::is_path_separator(b[i - 1]) {
        i -= 1;
    }
    String::from(&path[i..])
}
