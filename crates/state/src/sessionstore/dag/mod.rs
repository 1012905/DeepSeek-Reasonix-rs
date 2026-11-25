//! Foreign 1.x schema-2 session DAG reader.

mod compare;
mod entry;
mod error;
mod json;
mod materialize;
mod replay;

pub use compare::dag_transcript_matches;
pub use error::DagError;
pub use materialize::DagTail;
pub use replay::{
    load_session_dag_messages, load_session_dag_messages_with_limits, load_session_dag_tail,
    load_session_dag_tail_with_limits,
};

#[cfg(test)]
mod tests;
