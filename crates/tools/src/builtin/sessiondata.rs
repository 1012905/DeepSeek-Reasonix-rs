//! Session-data guard (session_guard.go, tools.md §7.2): refuses agent writes
//! into Reasonix's own session stores and runtime ledgers. The runtime is the
//! only writer of those files (CAS ledger + autosave); an agent editing them
//! from inside a chat races the app's own saves, which surfaces as repeated
//! save-conflict copies. The zero value is unconfined, matching the confine
//! helpers, so tools registered at init keep their historical behavior.
//!
//! Real filesystem resolution, home/config-dir discovery and the bash
//! output-wiring are the host-machinery slice's gap; this module keeps the
//! pure policy decisions and the typed refusal identities.

#![forbid(unsafe_code)]

use std::env;

use super::confine::{SessionDataGuardHook, real_path, real_roots, within, within_fold};
use super::pathgo;
use crate::refusal::Refusal;

/// `workspace.trusted_state_write` — a write landed in Trusted Host State.
pub const CODE_TRUSTED_STATE_WRITE: &str = "workspace.trusted_state_write";
/// `workspace.schedule_state_write` — a write landed in the scheduled-runs store.
pub const CODE_SCHEDULE_STATE_WRITE: &str = "workspace.schedule_state_write";

/// TrustedStateDir: the state-root subdirectory holding Trusted Host State
/// (trustedstate.DirectoryName).
pub const TRUSTED_STATE_DIR: &str = "trusted";
/// ScheduleStateDir: the state-root subdirectory holding the scheduled-runs store.
pub const SCHEDULE_STATE_DIR: &str = "schedules";

// Refusal templates live inline in `check` below: `format!` needs a string
// literal, and the `{:?}` quoting mirrors Go's `%q` for the target.

/// The warning appended to bash output when a command references the guarded
/// trees (CommandHint). A notice, never a refusal.
const HINT_WARNING: &str = "WARNING: this command referenced Reasonix's own session/state data under %s. This is a notice, not a refusal: the command was not blocked, and an empty result above means it matched nothing. To read a spilled tool output (<session>.outputs/), use read_file (offset/limit) or grep rather than shell text tools. The app is actively saving those files; external modifications conflict with its saves and are preserved as conflict copies, so an edit can look like it \"did not take\". Do not modify session files from a chat — stop retrying a write and report the underlying problem instead.";

/// The session-data guard, built once for the Reasonix state root and the
/// explicit `[sandbox] allow_write` entries. Both resolve to absolute,
/// symlink-free paths here, mirroring `realRoots`. An empty state root yields
/// an unconfined guard.
#[derive(Debug, Clone, Default)]
pub struct SessionDataGuard {
    state_root: String,
    allow_roots: Vec<String>,
    hint_needles: Vec<String>,
}

impl SessionDataGuard {
    /// NewSessionDataGuard: `state_root` is `config.MemoryUserDir()`;
    /// `allow_roots` are the explicit allow_write entries.
    pub fn new(state_root: &str, allow_roots: &[String]) -> Self {
        if state_root.trim().is_empty() {
            return Self::default();
        }
        let Ok(real) = real_path(state_root) else {
            return Self::default();
        };
        let allow_roots = real_roots(allow_roots);
        let hint_needles = session_hint_needles(state_root, &real, &allow_roots);
        Self {
            state_root: real,
            allow_roots,
            hint_needles,
        }
    }

    /// Whether the guard has any state root to confine against.
    pub fn is_unconfined(&self) -> bool {
        self.state_root.is_empty()
    }

    /// securityStateFile: a state-root-direct file name that is a security
    /// boundary rather than a mere runtime ledger.
    fn security_state_file(name: &str) -> bool {
        name == "settings.json"
    }

    /// runtimeStateFile: a desktop runtime ledger the app rewrites wholesale
    /// while running. `config.toml`, credentials, skills and
    /// `heartbeat-tasks.json` stay writable; settings.json is denied
    /// separately as a security boundary.
    fn runtime_state_file(name: &str) -> bool {
        name.starts_with("desktop-")
            || name == "metrics-pending.json"
            || name == "crash-pending.json"
    }

    fn covered_by_allow(&self, abs: &str) -> bool {
        self.allow_roots.iter().any(|a| within(a, abs))
    }

    /// deniesSecurity: a state-root-direct security boundary file not covered
    /// by an explicit allow_write root.
    fn denies_security(&self, abs: &str) -> bool {
        if self.covered_by_allow(abs) {
            return false;
        }
        let rel = match pathgo::rel(&self.state_root, abs) {
            Ok(rel) => rel,
            Err(_) => return false,
        };
        if rel == "." || rel.contains(['/', '\\']) {
            return false;
        }
        Self::security_state_file(&rel)
    }

    /// deniesTrusted: inside Trusted Host State, not covered by an allow root.
    fn denies_trusted(&self, abs: &str) -> bool {
        let root = pathgo::join(&[&self.state_root, TRUSTED_STATE_DIR]);
        self.allow_roots.iter().all(|a| !within_fold(a, abs)) && within_fold(&root, abs)
    }

    /// deniesSchedule: inside the scheduled-runs store, not covered by an allow root.
    fn denies_schedule(&self, abs: &str) -> bool {
        let root = pathgo::join(&[&self.state_root, SCHEDULE_STATE_DIR]);
        self.allow_roots.iter().all(|a| !within_fold(a, abs)) && within_fold(&root, abs)
    }

    /// denies: inside a guarded session store or runtime ledger, not explicitly
    /// allowed. Deny-side comparisons fold case on case-insensitive platforms.
    fn denies(&self, abs: &str) -> bool {
        if self.covered_by_allow(abs) {
            return false;
        }
        let sessions = pathgo::join(&[&self.state_root, "sessions"]);
        if within(&sessions, abs) {
            return true;
        }
        // State-root-direct runtime ledgers (desktop-tabs.json & friends).
        if let Ok(rel) = pathgo::rel(&self.state_root, abs)
            && rel != "."
            && !rel.contains(['/', '\\'])
            && Self::runtime_state_file(&rel)
        {
            return true;
        }
        // <state root>/projects/<slug>/sessions/** — the slug segment is
        // matched positionally rather than enumerated.
        let projects = pathgo::join(&[&self.state_root, "projects"]);
        if !within(&projects, abs) {
            return false;
        }
        let rel = match pathgo::rel(&projects, abs) {
            Ok(rel) => rel,
            Err(_) => return false,
        };
        let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
        parts.len() >= 2 && parts[1] == "sessions"
    }
}

impl SessionDataGuardHook for SessionDataGuard {
    fn check(&self, target: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.state_root.is_empty() {
            return Ok(());
        }
        let Ok(abs) = real_path(target) else {
            // Unresolvable: let the caller's normal error path handle it.
            return Ok(());
        };
        if self.denies_security(&abs) {
            return Err(std::io::Error::other(format!(
                "path {:?} is a Reasonix security boundary file ({:?} holds the global hooks; hooks execute arbitrary shell commands on every future session). Agents may not modify it. Ask the user to edit it themselves, or to add the directory to [sandbox] allow_write in reasonix.toml if raw access is truly intended",
                target, self.state_root
            ))
            .into());
        }
        if self.denies_trusted(&abs) {
            return Err(Refusal::code(
                CODE_TRUSTED_STATE_WRITE,
                format!(
                    "write refused: `{target}` is inside Reasonix's trusted host state ({}). The host alone records evidence and contracts there, so agents may not modify it; report the underlying problem instead",
                    pathgo::join(&[&self.state_root, TRUSTED_STATE_DIR])
                ),
            )
            .into());
        }
        if self.denies_schedule(&abs) {
            return Err(Refusal::code(
                CODE_SCHEDULE_STATE_WRITE,
                format!(
                    "write refused: `{target}` is inside the scheduled-runs store ({}). Schedules, budgets and run records are changed by the person through the app, never by an agent, so an unattended run's limits cannot be edited from a chat; report the underlying problem instead",
                    pathgo::join(&[&self.state_root, SCHEDULE_STATE_DIR])
                ),
            )
            .into());
        }
        if !self.denies(&abs) {
            return Ok(());
        }
        Err(std::io::Error::other(format!(
            "path {:?} is inside Reasonix's own session/state data ({:?}); the app is the only writer of these files, and edits from a chat race its saves — that surfaces as repeated save-conflict copies. Do not modify session or runtime-state files directly; report the underlying problem instead. If raw access is truly intended, add the directory to [sandbox] allow_write in reasonix.toml",
            target, self.state_root
        ))
        .into())
    }

    fn command_hint(&self, work_dir: &str, command: &str) -> String {
        if self.state_root.is_empty() || command.is_empty() {
            return String::new();
        }
        let warn = HINT_WARNING.replace("%s", &self.state_root);
        let haystack = pathgo::to_slash(command).to_lowercase();
        if self
            .hint_needles
            .iter()
            .any(|n| haystack.contains(n.as_str()))
        {
            return warn;
        }
        if work_dir.is_empty() {
            return String::new();
        }
        let Ok(abs_work) = real_path(work_dir) else {
            return String::new();
        };
        if self.denies(&abs_work) {
            return warn;
        }
        if within_fold(&self.state_root, &abs_work) {
            for sub in ["sessions", "projects"] {
                let target = pathgo::join(&[&self.state_root, sub]);
                let Ok(rel) = pathgo::rel(&abs_work, &target) else {
                    continue;
                };
                let needle = pathgo::to_slash(&rel).to_lowercase();
                if haystack.contains(&needle) {
                    return warn;
                }
            }
        }
        String::new()
    }
}

/// sessionHintNeedles: lowercase, slash-normalized textual forms of the
/// guarded trees as they may appear in a command. A tree wholly covered by an
/// allow_write root is skipped — the user sanctioned raw access there, so
/// warnings would only nag.
fn session_hint_needles(raw_root: &str, real_root: &str, allow_roots: &[String]) -> Vec<String> {
    let mut prefixes: Vec<String> = Vec::new();
    for prefix in [raw_root, real_root] {
        if let Ok(abs) = pathgo::abs_path(prefix) {
            let clean = pathgo::clean(&abs);
            if !prefixes.contains(&clean) {
                prefixes.push(clean);
            }
        }
    }
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_default();
    let config_dir = env::var("APPDATA")
        .or_else(|_| env::var("XDG_CONFIG_HOME"))
        .unwrap_or_default();

    let mut needles: Vec<String> = Vec::new();
    let subs = [
        "sessions",
        "projects",
        "desktop-",
        "metrics-pending.json",
        "crash-pending.json",
    ];
    for prefix in &prefixes {
        for sub in subs {
            let tree = pathgo::join(&[prefix, sub]);
            let covered = allow_roots
                .iter()
                .any(|a| within_fold(a, &pathgo::join(&[real_root, sub])));
            if covered {
                continue;
            }
            needles.push(pathgo::to_slash(&tree).to_lowercase());
            abbreviate(&home, &tree, &["~", "$home", "${home}"], &mut needles);
            abbreviate(
                &config_dir,
                &tree,
                &["%appdata%", "$env:appdata"],
                &mut needles,
            );
        }
    }
    needles
}

/// Adds abbreviated spellings of `tree` under `base` when the tree sits
/// inside it, slash-normalized and lowercased.
fn abbreviate(base: &str, tree: &str, forms: &[&str], out: &mut Vec<String>) {
    if base.is_empty() {
        return;
    }
    let rel = match pathgo::rel(base, tree) {
        Ok(rel) => rel,
        Err(_) => return,
    };
    if rel == "." || rel.starts_with("..") {
        return;
    }
    let slash_rel = pathgo::to_slash(&rel).to_lowercase();
    for form in forms {
        out.push(format!("{form}/{slash_rel}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::confine::real_path;

    fn state_root() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = real_path(&dir.path().to_string_lossy()).unwrap();
        (dir, root)
    }

    #[test]
    fn an_empty_state_root_is_unconfined() {
        let guard = SessionDataGuard::new("", &[]);
        assert!(guard.is_unconfined());
        assert!(guard.check("anything").is_ok());
        assert_eq!(guard.command_hint("", "ls ~/sessions"), "");
    }

    #[test]
    fn denies_the_sessions_tree_and_allows_an_allow_root() {
        let (_dir, root) = state_root();
        let sessions = pathgo::join(&[&root, "sessions"]);
        std::fs::create_dir_all(&sessions).unwrap();
        let guard = SessionDataGuard::new(&root, &[]);
        let target = pathgo::join(&[&sessions, "chat-1.json"]);
        let err = guard.check(&target).unwrap_err();
        assert!(err.to_string().contains("session/state data"));

        let allowed = SessionDataGuard::new(&root, std::slice::from_ref(&sessions));
        assert!(allowed.check(&target).is_ok());
    }

    #[test]
    fn trusted_and_schedule_stores_carry_typed_refusals() {
        let (_dir, root) = state_root();
        let trusted = pathgo::join(&[&root, TRUSTED_STATE_DIR]);
        let schedule = pathgo::join(&[&root, SCHEDULE_STATE_DIR]);
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::create_dir_all(&schedule).unwrap();
        let guard = SessionDataGuard::new(&root, &[]);

        let trusted_err = guard
            .check(&pathgo::join(&[&trusted, "evidence.json"]))
            .unwrap_err();
        let trusted = trusted_err
            .downcast_ref::<Refusal>()
            .expect("trusted refusal is typed");
        assert_eq!(trusted.code, CODE_TRUSTED_STATE_WRITE);

        let schedule_err = guard
            .check(&pathgo::join(&[&schedule, "run-1.json"]))
            .unwrap_err();
        let schedule = schedule_err
            .downcast_ref::<Refusal>()
            .expect("schedule refusal is typed");
        assert_eq!(schedule.code, CODE_SCHEDULE_STATE_WRITE);
    }

    #[test]
    fn security_boundary_settings_json_is_denied_at_the_root() {
        let (_dir, root) = state_root();
        let guard = SessionDataGuard::new(&root, &[]);
        let boundary = pathgo::join(&[&root, "settings.json"]);
        std::fs::write(&boundary, "{}").unwrap();
        let err = guard.check(&boundary).unwrap_err();
        assert!(err.to_string().contains("security boundary file"));

        // A desktop runtime ledger at the root is denied; heartbeat-tasks stays
        // writable per product policy.
        let ledger = pathgo::join(&[&root, "desktop-tabs.json"]);
        std::fs::write(&ledger, "[]").unwrap();
        assert!(guard.check(&ledger).is_err());
        let heartbeat = pathgo::join(&[&root, "heartbeat-tasks.json"]);
        std::fs::write(&heartbeat, "[]").unwrap();
        assert!(guard.check(&heartbeat).is_ok());
    }

    #[test]
    fn per_project_sessions_match_positionally() {
        let (_dir, root) = state_root();
        let store = pathgo::join(&[&root, "projects", "abcd", "sessions"]);
        std::fs::create_dir_all(&store).unwrap();
        let guard = SessionDataGuard::new(&root, &[]);
        let target = pathgo::join(&[&store, "spill.json"]);
        assert!(guard.check(&target).is_err());
        // A plain projects/<slug> file outside sessions is not denied.
        let outside = pathgo::join(&[&root, "projects", "abcd", "config.json"]);
        std::fs::write(&outside, "{}").unwrap();
        assert!(guard.check(&outside).is_ok());
    }

    #[test]
    fn command_hint_warns_on_a_lexical_reference_but_never_blocks() {
        let (_dir, root) = state_root();
        let guard = SessionDataGuard::new(&root, &[]);
        let rel = pathgo::to_slash(&root);
        let hint = guard.command_hint("", &format!("cat {rel}/sessions/chat-1.json"));
        assert!(hint.starts_with("WARNING:"));
        assert!(
            guard
                .check(&pathgo::join(&[&root, "sessions", "x"]))
                .is_err()
        );
        let quiet = guard.command_hint("", "ls ~/Documents");
        assert_eq!(quiet, "");
    }
}
