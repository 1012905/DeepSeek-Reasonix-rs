//! ACP-037: the `agent_composition` config option, its lock rule, the switch
//! admission refusals, and the persisted-session error-session fallback
//! (`composition.go:41-151`; `session_config.go:65-97`).
//!
//! The composition catalog, controller lifecycle, rebuild, and write-authority
//! preparation are runtime wiring. This module owns the pure selector shape:
//! the catalog choices, the four lock conditions, the switch admission table,
//! and the persisted-open classification that keeps a broken binding
//! readable as an error-session.

use std::fmt;

use crate::wire::{SessionConfigOption, SessionConfigSelectOption};

/// Option id advertised on the config surface (`composition.go:95`).
pub const OPTION_ID: &str = "agent_composition";
/// Display name (`composition.go:95`).
pub const OPTION_NAME: &str = "Agent composition";
/// Category axis (`composition.go:95`).
pub const OPTION_CATEGORY: &str = "agent_composition";
/// Implicit current composition before a prompt binds one (`composition.go:70`).
pub const DEFAULT_CURRENT_ID: &str = "builtin:general";
/// Implicit display name for that default (`composition.go:72`).
pub const DEFAULT_CURRENT_NAME: &str = "Standard";
/// Unlocked selector description (`composition.go:90`).
pub const DESCRIPTION_UNLOCKED: &str =
    "Choose a composition before the first prompt. Models are configured independently.";
/// Locked selector description (`composition.go:93`).
pub const DESCRIPTION_LOCKED: &str =
    "This conversation keeps its bound version. Create a new session to use another composition.";
/// `agentcomposition.ErrLocked` identity (`composition.go:22,109,116,140`).
pub const ERR_LOCKED: &str = "agent composition locked";
/// `agentcomposition.ErrInvalid` identity (`agentcomposition/composition.go:20`).
pub const ERR_INVALID: &str = "invalid agent composition";
/// `agentcomposition.ErrUnavailable` identity (`agentcomposition/composition.go:21`).
pub const ERR_UNAVAILABLE: &str = "agent composition unavailable";
/// `agentcomposition.ErrWorkspaceBound` identity (`agentcomposition/composition.go:23`).
pub const ERR_WORKSPACE_BOUND: &str = "agent composition is bound to its workspace";
/// Frontend-level inability to switch (`control/composition.go:86`).
pub const UNAVAILABLE_FRONTEND_MESSAGE: &str =
    "This frontend cannot switch the session's composition.";

/// One builtin preset (`builtins.json:1-16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

/// Builtin catalog baseline members.
pub const BUILTIN_PRESETS: [BuiltinPreset; 2] = [
    BuiltinPreset {
        id: "builtin:general",
        name: "日常搭档",
        description: "写代码、查资料、理清思路，和你一起把事情做好。沿用你的工具与配置，按需检索、编辑和运行。",
    },
    BuiltinPreset {
        id: "builtin:minimal",
        name: "轻装搭档",
        description: "带上终端和看图工具，轻装开工。和你一起处理小任务、快速试验，也方便对比模型表现。",
    },
];

/// Composition id grammar: `<namespace>:<rest>` where namespace is
/// `builtin` / `user` / `project`, the first rest char is alphanumeric, and the
/// rest is `[a-zA-Z0-9._-]` up to 63 more chars
/// (`agentcomposition/composition.go:24`).
pub fn is_valid_composition_id(id: &str) -> bool {
    let Some((namespace, rest)) = id.split_once(':') else {
        return false;
    };
    if !matches!(namespace, "builtin" | "user" | "project") {
        return false;
    }
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    rest.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// One catalog preset offered in the selector (`composition.go:87-89`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionChoice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// The four conditions that lock the composition selector
/// (`composition.go:71,84-85`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompositionLockState {
    /// The controller reports the composition already started.
    pub started: bool,
    /// A prompt is pending in the session.
    pub pending_prompt: bool,
    /// Background jobs still running.
    pub background_jobs: u32,
    /// The persisted binding reports it started.
    pub binding_started: bool,
}

impl CompositionLockState {
    /// Locked when any of the four conditions holds (`composition.go:71,85`).
    pub fn is_locked(&self) -> bool {
        self.started || self.pending_prompt || self.background_jobs > 0 || self.binding_started
    }
}

/// Builds the advertised `agent_composition` option. A locked session collapses
/// the catalog to its single current binding (`composition.go:86-95`).
pub fn build_option(
    current_id: &str,
    current_name: &str,
    lock: &CompositionLockState,
    catalog: &[CompositionChoice],
) -> SessionConfigOption {
    let description: &str;
    let options: Vec<SessionConfigSelectOption>;
    if lock.is_locked() {
        description = DESCRIPTION_LOCKED;
        options = vec![SessionConfigSelectOption {
            value: current_id.to_owned(),
            name: current_name.to_owned(),
            description: None,
        }];
    } else {
        description = DESCRIPTION_UNLOCKED;
        options = catalog
            .iter()
            .map(|choice| SessionConfigSelectOption {
                value: choice.id.clone(),
                name: choice.name.clone(),
                description: choice.description.clone(),
            })
            .collect();
    }

    SessionConfigOption {
        id: OPTION_ID.to_owned(),
        name: OPTION_NAME.to_owned(),
        description: Some(description.to_owned()),
        category: Some(OPTION_CATEGORY.to_owned()),
        r#type: "select".to_owned(),
        current_value: current_id.to_owned(),
        options,
    }
}

/// Switch refused: the selector lock could not be acquired, or the session
/// state changed underneath the switch (`composition.go:107-116,137-140`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositionLocked;

impl fmt::Display for CompositionLocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(ERR_LOCKED)
    }
}

impl std::error::Error for CompositionLocked {}

/// Live-session state the switch admission reads (`composition.go:112-116`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompositionSessionState {
    pub deleted: bool,
    pub running: bool,
    pub maintenance_in_flight: bool,
    /// The controller still matches the one the switch started from.
    pub controller_matches: bool,
}

/// Admits a composition switch. `lock_free` reports whether the state-change
/// mutex was acquired; any deleted / running / maintenance-in-flight /
/// controller-changed state refuses with [`CompositionLocked`]
/// (`composition.go:108-116`).
pub fn admit_switch(
    state: &CompositionSessionState,
    lock_free: bool,
) -> Result<(), CompositionLocked> {
    if !lock_free {
        return Err(CompositionLocked);
    }
    if state.deleted || state.running || state.maintenance_in_flight || !state.controller_matches {
        return Err(CompositionLocked);
    }
    Ok(())
}

/// Outcome of opening a persisted session (`composition.go:41-52`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistedOpenOutcome {
    /// The factory built a controller.
    Controller,
    /// The factory failed but a binding exists and the failure was not
    /// cancellation: build the history-carrying error-session controller.
    ErrorSession,
    /// Hard failure the caller must surface.
    Failure,
}

/// Classifies the persisted open (`composition.go:47-50`). `cancelled` folds
/// both `context.Canceled` and `context.DeadlineExceeded`.
pub fn classify_persisted_open(
    factory_failed: bool,
    binding_present: bool,
    cancelled: bool,
) -> PersistedOpenOutcome {
    match (factory_failed, binding_present, cancelled) {
        (false, _, _) => PersistedOpenOutcome::Controller,
        (true, true, false) => PersistedOpenOutcome::ErrorSession,
        _ => PersistedOpenOutcome::Failure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocked_option_offers_the_catalog_with_the_independent_model_description() {
        let catalog = vec![
            CompositionChoice {
                id: "builtin:general".to_owned(),
                name: "Standard".to_owned(),
                description: None,
            },
            CompositionChoice {
                id: "builtin:minimal".to_owned(),
                name: "Minimal".to_owned(),
                description: Some("Bare bones".to_owned()),
            },
        ];
        let option = build_option(
            DEFAULT_CURRENT_ID,
            DEFAULT_CURRENT_NAME,
            &CompositionLockState::default(),
            &catalog,
        );
        assert_eq!(option.id, OPTION_ID);
        assert_eq!(option.name, OPTION_NAME);
        assert_eq!(option.category.as_deref(), Some(OPTION_CATEGORY));
        assert_eq!(option.r#type, "select");
        assert_eq!(option.current_value, DEFAULT_CURRENT_ID);
        assert_eq!(option.description.as_deref(), Some(DESCRIPTION_UNLOCKED));
        assert_eq!(option.options.len(), 2);
        assert_eq!(option.options[1].description.as_deref(), Some("Bare bones"));
    }

    #[test]
    fn locked_option_collapses_to_the_current_binding() {
        let lock = CompositionLockState {
            pending_prompt: true,
            ..Default::default()
        };
        let catalog = vec![CompositionChoice {
            id: "builtin:minimal".to_owned(),
            name: "Minimal".to_owned(),
            description: None,
        }];
        let option = build_option("builtin:minimal", "Minimal", &lock, &catalog);
        assert_eq!(option.description.as_deref(), Some(DESCRIPTION_LOCKED));
        assert_eq!(option.options.len(), 1);
        assert_eq!(option.options[0].value, "builtin:minimal");
        assert_eq!(option.options[0].name, "Minimal");
    }

    #[test]
    fn every_lock_condition_individually_locks_the_selector() {
        let states = [
            CompositionLockState {
                started: true,
                ..Default::default()
            },
            CompositionLockState {
                pending_prompt: true,
                ..Default::default()
            },
            CompositionLockState {
                background_jobs: 1,
                ..Default::default()
            },
            CompositionLockState {
                binding_started: true,
                ..Default::default()
            },
        ];
        for state in states {
            assert!(state.is_locked());
        }
        assert!(!CompositionLockState::default().is_locked());
    }

    #[test]
    fn switch_refuses_when_the_lock_is_held_or_the_session_state_changed() {
        let healthy = CompositionSessionState {
            controller_matches: true,
            ..Default::default()
        };
        assert!(admit_switch(&healthy, true).is_ok());
        assert_eq!(admit_switch(&healthy, false), Err(CompositionLocked));

        let refused = [
            CompositionSessionState {
                deleted: true,
                ..healthy
            },
            CompositionSessionState {
                running: true,
                ..healthy
            },
            CompositionSessionState {
                maintenance_in_flight: true,
                ..healthy
            },
            CompositionSessionState {
                controller_matches: false,
                ..healthy
            },
        ];
        for state in refused {
            assert_eq!(admit_switch(&state, true), Err(CompositionLocked));
        }
        assert_eq!(CompositionLocked.to_string(), ERR_LOCKED);
    }

    #[test]
    fn builtin_presets_match_the_frozen_catalog() {
        assert_eq!(BUILTIN_PRESETS[0].id, "builtin:general");
        assert_eq!(BUILTIN_PRESETS[0].name, "日常搭档");
        assert_eq!(
            BUILTIN_PRESETS[0].description,
            "写代码、查资料、理清思路，和你一起把事情做好。沿用你的工具与配置，按需检索、编辑和运行。"
        );
        assert_eq!(BUILTIN_PRESETS[1].id, "builtin:minimal");
        assert_eq!(BUILTIN_PRESETS[1].name, "轻装搭档");
        assert_eq!(
            BUILTIN_PRESETS[1].description,
            "带上终端和看图工具，轻装开工。和你一起处理小任务、快速试验，也方便对比模型表现。"
        );
    }

    #[test]
    fn composition_id_grammar_follows_the_regex() {
        assert!(is_valid_composition_id("builtin:general"));
        assert!(is_valid_composition_id("user:my-agent_1.2-3"));
        assert!(is_valid_composition_id("project:X"));
        assert!(!is_valid_composition_id("builtin:"));
        assert!(!is_valid_composition_id("builtin:_agent"));
        assert!(!is_valid_composition_id("other:general"));
        assert!(!is_valid_composition_id("builtin:general:extra"));
        let long = format!("builtin:{}", "a".repeat(65));
        assert!(!is_valid_composition_id(&long));
    }

    #[test]
    fn persisted_open_falls_back_to_error_session_only_for_a_present_binding() {
        assert_eq!(
            classify_persisted_open(false, true, false),
            PersistedOpenOutcome::Controller
        );
        assert_eq!(
            classify_persisted_open(true, true, false),
            PersistedOpenOutcome::ErrorSession
        );
        assert_eq!(
            classify_persisted_open(true, false, false),
            PersistedOpenOutcome::Failure
        );
        assert_eq!(
            classify_persisted_open(true, true, true),
            PersistedOpenOutcome::Failure
        );
    }
}
