//! Authored-turn classification (Go: `authored_turn.go`).
//!
//! Both projections of the authored-turn number derive it here: the
//! display-index sidecar numbers a whole transcript, and a live turn start
//! names its own pending message with it, so the two can never disagree.

use reasonix_contract::provider::{Message, Role};

use super::preview::{
    is_synthetic_user_text, is_user_authored_turn, steer_text, user_message_text,
};

/// What a host published for the message a turn is about, before any work
/// that could produce it: the turn number, the session index it named, and
/// the raw authored text both were derived from (Go: `AuthoredTurnIdentity`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthoredTurnIdentity {
    /// The authored turn this message opens.
    pub authored_turn: i64,
    /// The index the message will take in the session.
    pub msg_index: i64,
    /// The raw authored text the number was derived from.
    pub raw: String,
}

/// The authored turn a message belongs to, and why it does or does not open
/// one (Go: `TurnClassification`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnClassification {
    /// Absolute authored-turn number; non-user messages inherit the prior.
    pub authored_turn: i64,
    /// This message is the one that opens `authored_turn`.
    pub starts_turn: bool,
    /// A user message that opened no turn because it is a mid-turn steer.
    pub steer: bool,
    /// A user message that opened no turn because the host injected it.
    pub synthetic: bool,
}

/// Answers which authored turn `msg` belongs to, given the turns closed
/// before it (Go: `ClassifyTurn`).
pub fn classify_turn(msg: &Message, prior_turn: i64) -> TurnClassification {
    let mut class = TurnClassification {
        authored_turn: prior_turn,
        ..TurnClassification::default()
    };
    if msg.role != Role::USER {
        return class;
    }
    let content = user_message_text(msg);
    if is_user_authored_turn(&content) {
        class.authored_turn = prior_turn + 1;
        class.starts_turn = true;
        return class;
    }
    if steer_text(&content).1 {
        class.steer = true;
    } else if is_synthetic_user_text(&content) {
        class.synthetic = true;
    }
    class
}

/// The authored-turn count over `msgs`: the number the next message appended
/// after them would follow (Go agent `PriorAuthoredTurn`). Re-derived from the
/// transcript rather than tracked beside it, so no counter can disagree with
/// the messages it claims to number.
pub fn prior_authored_turn(msgs: &[Message]) -> i64 {
    let mut turn = 0;
    for msg in msgs {
        turn = classify_turn(msg, turn).authored_turn;
    }
    turn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::preview::{HOST_NOTICE_PREFIX, MID_TURN_STEER_PREFIX};

    fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    #[test]
    fn non_user_messages_inherit_the_prior_turn() {
        let assistant = Message {
            role: Role::ASSISTANT,
            content: "working".to_string(),
            ..Message::default()
        };
        let class = classify_turn(&assistant, 4);
        assert_eq!(class.authored_turn, 4);
        assert!(!class.starts_turn);
        assert!(!class.steer);
        assert!(!class.synthetic);
    }

    #[test]
    fn authored_user_message_opens_the_next_turn() {
        let class = classify_turn(&user("fix the login bug"), 2);
        assert_eq!(class.authored_turn, 3);
        assert!(class.starts_turn);
        assert!(!class.steer && !class.synthetic);
    }

    #[test]
    fn steer_and_synthetic_users_open_no_turn() {
        let steer = classify_turn(&user(&format!("{MID_TURN_STEER_PREFIX}\nfaster")), 1);
        assert_eq!(steer.authored_turn, 1);
        assert!(!steer.starts_turn);
        assert!(steer.steer);
        assert!(!steer.synthetic);

        let host = classify_turn(&user(&format!("{HOST_NOTICE_PREFIX}\nnotice")), 1);
        assert!(host.steer);
        assert!(!host.starts_turn);

        let synthetic = classify_turn(&user("Continue pursuing the active goal"), 1);
        assert_eq!(synthetic.authored_turn, 1);
        assert!(synthetic.synthetic);
        assert!(!synthetic.steer);
        assert!(!synthetic.starts_turn);
    }

    #[test]
    fn blank_user_message_opens_no_turn() {
        let class = classify_turn(&user("   \n"), 0);
        assert_eq!(class.authored_turn, 0);
        assert!(!class.starts_turn);
        assert!(!class.steer && !class.synthetic);
    }

    #[test]
    fn prior_authored_turn_counts_authored_turns_only() {
        let msgs = vec![
            Message {
                role: Role::SYSTEM,
                content: "sys".to_string(),
                ..Message::default()
            },
            user("first question"),
            Message {
                role: Role::ASSISTANT,
                content: "answer".to_string(),
                ..Message::default()
            },
            user(&format!("{MID_TURN_STEER_PREFIX}\nhurry")),
            user("Continue pursuing the active goal"),
            user("second question"),
        ];
        assert_eq!(prior_authored_turn(&msgs), 2);
        assert_eq!(prior_authored_turn(&[]), 0);
    }
}
