//! Projection of inbox states onto the process vocabulary.

use super::types::{
    InboxState, STATE_BLOCKED, STATE_QUEUED, STATE_RUNNING, STATE_STEER_ACCEPTED,
    STATE_STEER_CONSUMED, STATE_UNCERTAIN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessClass {
    pub state: &'static str,
    pub cause: &'static str,
}

impl InboxState {
    pub fn process_class(&self) -> Option<ProcessClass> {
        if self == &STATE_QUEUED {
            Some(ProcessClass {
                state: "queued",
                cause: "",
            })
        } else if self == &STATE_BLOCKED {
            Some(ProcessClass {
                state: "queued",
                cause: "held",
            })
        } else if self == &STATE_RUNNING || self == &STATE_STEER_ACCEPTED {
            Some(ProcessClass {
                state: "running",
                cause: "",
            })
        } else if self == &STATE_STEER_CONSUMED {
            Some(ProcessClass {
                state: "completed",
                cause: "",
            })
        } else if self == &STATE_UNCERTAIN {
            Some(ProcessClass {
                state: "interrupted",
                cause: "",
            })
        } else {
            None
        }
    }
}
