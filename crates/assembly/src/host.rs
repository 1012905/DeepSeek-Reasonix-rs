//! Desktop-host boot (docs/contracts/host.md §1.7): the composition the
//! reasonix-studio-host binary drives. Configuration reaches this crate
//! through the serve frontend (ADR-0001 key choice 3); the binary keeps
//! argv, the handshake line, acts, phases, and the lease.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use reasonix_serve as serve;
use reasonix_session::hub::{Hub, HubPaths};
use sha2::{Digest, Sha256};

/// The launch pieces the binary needs to announce and serve. One per launch.
pub struct HostBoot {
    pub listener: tokio::net::TcpListener,
    /// `http://127.0.0.1:<port>` — what the handshake and the gate agree on.
    pub origin: String,
    /// The credential the binary announced and the gate checks.
    pub token: String,
    pub router: axum::Router,
    pub page: Option<PathBuf>,
}

pub struct HostBootOptions {
    /// Explicit `-page` directory; `None` runs the FindPage fallback chain.
    pub page: Option<PathBuf>,
    /// The credential minted for this launch (host.md §2.1). The gate refuses
    /// everything when this is empty.
    pub token: String,
    /// The shell's executable (host.md §1.2 `-studio-app`); empty for an
    /// unpackaged launch.
    pub app_exe: PathBuf,
    /// Startup phase transitions, in the frozen 12-phase order.
    pub on_phase: Arc<dyn Fn(&str) + Send + Sync>,
    /// Boot log lines (the binary formats them with the program prefix).
    pub on_log: Arc<dyn Fn(&str) + Send + Sync>,
    pub cwd: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("{0}")]
    Page(String),
    #[error("config: {0}")]
    Config(String),
    #[error("{0}")]
    Bind(String),
    #[error("{0}")]
    Serve(String),
}

/// Phases whose machines land with later slices still emit their line: the
/// startup log is the observable boot sequence, and a missing line would
/// desync a reader watching for the frozen order.
pub async fn prepare(opts: HostBootOptions) -> Result<HostBoot, BootError> {
    let on_phase = &opts.on_phase;
    let on_log = &opts.on_log;

    let page = match serve::find_page(opts.page.as_deref()) {
        Err(e) => return Err(BootError::Page(e.to_string())),
        Ok(page) => page,
    };
    if page.is_none() {
        on_log("no built page found; serving the kernel only");
    }
    on_phase("page");

    // The one-time config upgrades land with the config upgrade machine.
    on_phase("config-upgrade");
    let config =
        serve::boot::load_config(&opts.cwd).map_err(|e| BootError::Config(e.to_string()))?;
    on_phase("config");

    // Per-runtime sink wrappers land with the agent slice: emit::turn_emit_pipeline
    // composes the chain once per runtime and hands the turn machine its side.
    on_phase("sinks");

    let hub_paths = HubPaths::from_process_roots().unwrap_or_else(|| HubPaths {
        state_root: PathBuf::new(),
    });
    // The launch project wins, then the remembered order (host.md §6.1).
    let remembered: Vec<PathBuf> =
        reasonix_session::workspacelist::launch_workspaces(&hub_paths.workspaces_path())
            .into_iter()
            .map(PathBuf::from)
            .collect();
    // The auto workspace is created here (created last, only when nothing
    // else won); the root feeds the pane runtime.
    let root = pick_launch_workspace(
        &opts.cwd,
        &opts.app_exe,
        &remembered,
        &serve::boot::auto_workspace_dir(&config),
    );
    on_phase("workspace");

    // The approval posture the configuration named, or the headless default.
    // A named posture is what makes a published runtime answerable; an
    // unnamed one keeps the Go-nil seams rather than inventing a prompt.
    let approval = serve::boot::runtime_approval(&config);
    on_phase("runtime");

    // The open machine carries the posture because it is the only code that
    // holds it at publish time (Go: `EnableInteractiveApproval` runs from
    // publish, not from the frontend that asked). The provider fields stay
    // empty until the provider-config edge lands: an open before then
    // refuses with the typed missing field rather than a faked runtime.
    let spec = reasonix_session::hub::RuntimeSpec {
        workspace_root: root.to_string_lossy().into_owned(),
        ..reasonix_session::hub::RuntimeSpec::default()
    };
    let hub = Arc::new(Hub::with_options(
        hub_paths,
        reasonix_session::hub::HubOptions {
            spec,
            kit: Arc::new(crate::emit::PaneKit::with_approval(false, approval)),
        },
    ));
    on_phase("hub");

    // Runtime adoption lands with the open machine.
    on_phase("adopt");

    // The background recovery sweep lands with the state machine.
    on_phase("background");

    let listener = serve::listen_loopback().map_err(|e| BootError::Bind(e.to_string()))?;
    let origin = serve::loopback_origin(&listener).map_err(|e| BootError::Bind(e.to_string()))?;
    let listener =
        serve::to_tokio_listener(listener).map_err(|e| BootError::Bind(e.to_string()))?;
    let router = serve::router::build_router(
        hub,
        serve::router::RouterConfig {
            token: opts.token.clone(),
            origin: origin.clone(),
            page_dir: page.clone(),
            on_tree_changed: None,
        },
    );
    on_phase("bind");

    // The credential-writing setup surface opens on this listener only; its
    // machine lands with the provider slice.
    on_phase("provider-setup");

    Ok(HostBoot {
        listener,
        origin,
        token: opts.token,
        router,
        page,
    })
}

/// Serves the prepared boot until `shutdown` completes, then drains.
pub async fn serve_until(
    boot: HostBoot,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    serve::run_graceful_handler(shutdown, boot.listener, boot.router).await
}

/// The Studio instance id (host.md §5.1): one data home is one Studio, and
/// the identity is the canonicalized home, not the spelling.
pub fn instance_id() -> String {
    instance_id_for(&serve::boot::reasonix_home_dir().to_string_lossy())
}

pub fn instance_id_for(home: &str) -> String {
    const PREFIX: &str = "io.reasonix.studio";
    let trimmed = home.trim();
    if trimmed.is_empty() {
        return PREFIX.to_string();
    }
    let mut root = trimmed.to_string();
    if let Some(parent) = canonical_existing(
        &Path::new(&root)
            .join(".reasonix-home.identity")
            .to_string_lossy(),
    )
    .and_then(|marker| Path::new(&marker).parent().map(|p| p.to_path_buf()))
    {
        root = parent.to_string_lossy().into_owned();
    }
    // The hash input is the OS-separator Clean of the root: on Windows Go's
    // filepath.Clean spells backslashes, so the identity matches the
    // reference host byte for byte (M-3).
    let cleaned = reasonix_session::paths::clean(&root);
    let sum = Sha256::digest(cleaned.as_bytes());
    format!("{}.{}", PREFIX, hex(&sum[..8]))
}

/// Physical path of an existing file, symlinks resolved, the Windows `\\?\`
/// prefix stripped — the shape the identity hashes. `None` when the path does
/// not exist; the walk-through-nearest-existing-ancestor of the reference is
/// state-crate machinery this crate must not reach (frozen dependency
/// policy), so a not-yet-created home hashes as spelled (OPEN_QUESTIONS §8).
fn canonical_existing(path: &str) -> Option<String> {
    let resolved = std::fs::canonicalize(path).ok()?;
    let mut key = resolved.to_string_lossy().into_owned();
    if cfg!(windows) {
        if let Some(stripped) = key.strip_prefix("\\\\?\\UNC\\") {
            key = format!("\\\\{}", stripped);
        } else if let Some(stripped) = key.strip_prefix("\\\\?\\") {
            key = stripped.to_string();
        }
    }
    Some(key)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                HEX[(b >> 4) as usize] as char,
                HEX[(b & 0x0f) as usize] as char,
            ]
        })
        .collect()
}

/// The launch workspace (host.md §6.1): a chosen directory, then a remembered
/// project, then the auto workspace — created last — never the install tree
/// or the home directory by inheritance.
pub fn pick_launch_workspace(
    cwd: &Path,
    app_exe: &Path,
    remembered: &[PathBuf],
    auto: &Path,
) -> PathBuf {
    if user_chose_dir(cwd, app_exe) {
        return cwd.to_path_buf();
    }
    for dir in remembered {
        if dir.is_dir() {
            return dir.clone();
        }
    }
    if !auto.as_os_str().is_empty() && std::fs::create_dir_all(auto).is_ok() {
        return auto.to_path_buf();
    }
    cwd.to_path_buf()
}

/// The cwd counts only when someone chose it: a launcher inherits the install
/// directory or the filesystem root, and neither is a project.
fn user_chose_dir(cwd: &Path, app_exe: &Path) -> bool {
    if cwd.as_os_str().is_empty() || cwd.parent().is_none() {
        return false;
    }
    let install = match app_exe.parent() {
        Some(dir) if !dir.as_os_str().is_empty() && app_exe.file_name().is_some() => dir,
        // No stated application: an unpackaged launch keeps its cwd.
        _ => return true,
    };
    // Go's inside rule reduces to "the relative path does not climb out with
    // ..": "." (and the equal-path empty case) and subdirectories are inside;
    // anything climbing out — including exactly ".." — was chosen.
    let inside = match relative_to(install, cwd) {
        None => false,
        Some(rel) => !rel.starts_with(".."),
    };
    !inside
}

/// Lexical `filepath.Rel`: `Some` when `target` sits under `base` (or equals
/// it), `None` when no relative path exists.
fn relative_to(base: &Path, target: &Path) -> Option<PathBuf> {
    let mut base = base.components().peekable();
    let mut target = target.components().peekable();
    while let (Some(b), Some(t)) = (base.peek(), target.peek()) {
        if !same_component(b.as_os_str(), t.as_os_str()) {
            break;
        }
        base.next();
        target.next();
    }
    if base.peek().is_some() {
        return None;
    }
    let mut rel = PathBuf::new();
    for c in target {
        rel.push(c.as_os_str());
    }
    Some(rel)
}

fn same_component(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/assembly-host");
        let dir = base.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_id_is_prefix_plus_eight_bytes_hex_and_stable_per_home() {
        let id = instance_id_for("/some/home");
        assert!(id.starts_with("io.reasonix.studio."));
        let suffix = &id["io.reasonix.studio.".len()..];
        assert_eq!(suffix.len(), 16);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(instance_id_for("  /some/home  "), id);
    }

    #[test]
    fn an_empty_home_names_the_bare_prefix() {
        assert_eq!(instance_id_for(""), "io.reasonix.studio");
    }

    #[test]
    fn an_existing_marker_canonicalizes_the_home() {
        let base = tmp_dir("marker");
        std::fs::write(base.join(".reasonix-home.identity"), "x").unwrap();
        let id = instance_id_for(&base.to_string_lossy());
        let cleaned = reasonix_session::paths::clean(&base.to_string_lossy());
        let suffix = hex(&Sha256::digest(cleaned.as_bytes())[..8]);
        assert_eq!(id, format!("io.reasonix.studio.{}", suffix));
    }

    #[test]
    fn the_hash_input_keeps_the_os_separator_shape_like_go() {
        // Go's filepath.Clean spells the platform separator; the identity
        // hashes those bytes, not a rewritten "/" form (M-3).
        let (spelled, want_clean) = if cfg!(windows) {
            ("C:\\a\\.\\b\\..\\c\\", "C:\\a\\c")
        } else {
            ("/a/./b/../c/", "/a/c")
        };
        let suffix = hex(&Sha256::digest(want_clean.as_bytes())[..8]);
        assert_eq!(
            instance_id_for(spelled),
            format!("io.reasonix.studio.{}", suffix)
        );
    }

    #[test]
    fn the_install_directory_is_not_a_chosen_workspace() {
        let install = if cfg!(windows) {
            "C:\\apps\\studio\\bin"
        } else {
            "/apps/studio/bin"
        };
        let cwd = Path::new(install).parent().unwrap();
        assert!(!user_chose_dir(cwd, Path::new(install)));
    }

    #[test]
    fn the_filesystem_root_is_never_a_chosen_workspace() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let exe = if cfg!(windows) {
            "C:\\apps\\studio\\bin"
        } else {
            "/apps/studio/bin"
        };
        assert!(!user_chose_dir(Path::new(root), Path::new(exe)));
    }

    #[test]
    fn an_unrelated_cwd_counts_as_chosen() {
        let cwd = if cfg!(windows) {
            "C:\\work\\proj"
        } else {
            "/work/proj"
        };
        let exe = if cfg!(windows) {
            "C:\\apps\\studio\\bin\\host.exe"
        } else {
            "/apps/studio/bin/host"
        };
        assert!(user_chose_dir(Path::new(cwd), Path::new(exe)));
    }

    #[test]
    fn fallback_order_is_remembered_then_auto_then_cwd() {
        // The cwd sits in the install tree (launcher-inherited), so it is not
        // "chosen" and the fallback applies.
        let cwd = if cfg!(windows) {
            "C:\\apps\\studio\\bin"
        } else {
            "/apps/studio/bin"
        };
        let base = tmp_dir("fallback");
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let auto_dir = base.join("auto");
        let got = pick_launch_workspace(
            Path::new(cwd),
            Path::new(if cfg!(windows) {
                "C:\\apps\\studio\\bin\\host.exe"
            } else {
                "/apps/studio/bin/host"
            }),
            &[PathBuf::from("/missing/dir"), real.clone()],
            &auto_dir,
        );
        assert_eq!(got, real);
        let got = pick_launch_workspace(
            Path::new(cwd),
            Path::new(if cfg!(windows) {
                "C:\\apps\\studio\\bin\\host.exe"
            } else {
                "/apps/studio/bin/host"
            }),
            &[PathBuf::from("/missing/dir")],
            &auto_dir,
        );
        assert_eq!(got, auto_dir);
        assert!(auto_dir.is_dir());
    }

    #[test]
    fn with_nothing_at_all_the_cwd_wins() {
        let cwd = Path::new(if cfg!(windows) {
            "C:\\apps\\studio\\bin"
        } else {
            "/apps/studio/bin"
        });
        let exe = Path::new(if cfg!(windows) {
            "C:\\apps\\studio\\bin\\host.exe"
        } else {
            "/apps/studio/bin/host"
        });
        // The auto branch must be unreachable, and deterministically so: an
        // uncreatable path under a FILE, not one that guesses at what the
        // machine refuses. A root-level spelling like /nowhere depends on
        // host policy and leaves the created tree behind on machines where
        // creation succeeds — pollution this suite then reads back.
        let blocker = tmp_dir("auto-blocker");
        std::fs::write(blocker.join("not-a-dir"), b"").unwrap();
        let auto = blocker.join("not-a-dir").join("deep");
        let got = pick_launch_workspace(cwd, exe, &[], &auto);
        assert_eq!(got, cwd);
    }
}
