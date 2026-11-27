use super::*;

#[test]
fn every_known_inbox_state_has_the_go_process_projection() {
    let cases = [
        (STATE_QUEUED, "queued", ""),
        (STATE_BLOCKED, "queued", "held"),
        (STATE_RUNNING, "running", ""),
        (STATE_STEER_ACCEPTED, "running", ""),
        (STATE_STEER_CONSUMED, "completed", ""),
        (STATE_UNCERTAIN, "interrupted", ""),
    ];

    for (state, process_state, cause) in cases {
        assert_eq!(
            state.process_class(),
            Some(ProcessClass {
                state: process_state,
                cause,
            }),
            "{state:?}"
        );
    }
}

#[test]
fn unknown_and_empty_inbox_states_have_no_process_projection() {
    assert_eq!(InboxState::default().process_class(), None);
    assert_eq!(InboxState::new("future_state").process_class(), None);
    assert_eq!(InboxState::new(" running ").process_class(), None);
}
