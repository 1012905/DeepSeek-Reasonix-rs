//! ACP-037: composition selector lock rule, two-state descriptions, switch
//! admission guard, id grammar, builtin presets, and the error-session
//! classification.

use reasonix_acp::agent_composition::{
    BUILTIN_PRESETS, CompositionChoice, CompositionLockState, CompositionLocked,
    CompositionSessionState, DEFAULT_CURRENT_ID, DEFAULT_CURRENT_NAME, DESCRIPTION_LOCKED,
    DESCRIPTION_UNLOCKED, ERR_LOCKED, PersistedOpenOutcome, admit_switch, build_option,
    classify_persisted_open, is_valid_composition_id,
};

#[test]
fn unlocked_option_offers_the_catalog_with_independent_model_description() {
    let catalog: Vec<CompositionChoice> = BUILTIN_PRESETS
        .iter()
        .map(|p| CompositionChoice {
            id: p.id.to_owned(),
            name: p.name.to_owned(),
            description: Some(p.description.to_owned()),
        })
        .collect();
    let option = build_option(
        DEFAULT_CURRENT_ID,
        DEFAULT_CURRENT_NAME,
        &CompositionLockState::default(),
        &catalog,
    );
    assert_eq!(option.id, "agent_composition");
    assert_eq!(option.name, "Agent composition");
    assert_eq!(option.category.as_deref(), Some("agent_composition"));
    assert_eq!(option.r#type, "select");
    assert_eq!(option.current_value, DEFAULT_CURRENT_ID);
    assert_eq!(option.description.as_deref(), Some(DESCRIPTION_UNLOCKED));
    assert_eq!(option.options.len(), 2);
}

#[test]
fn locked_option_collapses_to_the_single_current_binding() {
    let lock = CompositionLockState {
        pending_prompt: true,
        ..Default::default()
    };
    let catalog = vec![CompositionChoice {
        id: "builtin:minimal".to_owned(),
        name: "轻装搭档".to_owned(),
        description: None,
    }];
    let option = build_option("builtin:minimal", "轻装搭档", &lock, &catalog);
    assert_eq!(option.description.as_deref(), Some(DESCRIPTION_LOCKED));
    assert_eq!(option.options.len(), 1);
    assert_eq!(option.options[0].value, "builtin:minimal");
    assert_eq!(option.options[0].name, "轻装搭档");
}

#[test]
fn each_lock_condition_individually_locks_the_selector() {
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
fn switch_refuses_when_lock_is_held_or_session_state_changed() {
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
fn composition_id_grammar_accepts_namespaced_ids_and_rejects_the_rest() {
    assert!(is_valid_composition_id("builtin:general"));
    assert!(is_valid_composition_id("user:my-agent_1.2-3"));
    assert!(is_valid_composition_id("project:X"));
    assert!(!is_valid_composition_id("builtin:"));
    assert!(!is_valid_composition_id("builtin:_agent"));
    assert!(!is_valid_composition_id("other:general"));
}

#[test]
fn persisted_open_error_session_needs_a_present_non_cancelled_binding() {
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
