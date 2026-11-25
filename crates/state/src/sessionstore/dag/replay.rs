use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::Path;

use reasonix_contract::provider::{Message, Role};
use serde_json::value::RawValue;

use crate::sessionstore::eventlog::reader::{NextError, SessionRecordReader};
use crate::sessionstore::eventlog::replay::decode_session_event_messages;
use crate::sessionstore::eventlog::{
    DecodeError, SESSION_EVENT_DAG_SCHEMA_VERSION, SESSION_REPLAY_MAX_PAYLOAD_BYTES,
    SESSION_TAIL_BUDGET_BYTES, SessionReplayLimits, session_replay_limit_error,
};
use crate::store::session_event_log;

use super::DagError;
use super::entry::{Entry, parse_entry};
use super::materialize::{DagTail, read_dag_payloads, tail_of_dag_payloads};

const MAIN_HEAD: &str = "main";

#[derive(Debug, Clone, Default)]
pub(crate) struct PayloadRef {
    pub start: i64,
    pub end: i64,
    pub role: Role,
}

#[derive(Debug)]
struct Node {
    parent: String,
    payload: PayloadRef,
}

#[derive(Debug, Default)]
struct Head {
    leaf: String,
    system: Option<PayloadRef>,
    last_activity: crate::sessionstore::eventlog::CreatedAt,
    last_offset: i64,
    retired: bool,
}

#[derive(Debug)]
pub(crate) struct DagState {
    path: String,
    nodes: HashMap<String, Node>,
    heads: HashMap<String, Head>,
    head_order: Vec<String>,
    patches: HashMap<String, PayloadRef>,
    redactions: HashMap<String, PayloadRef>,
    selected: String,
    collection_items: usize,
    metadata_bytes: i64,
    records: usize,
    damaged: bool,
}

impl DagState {
    pub(crate) fn new(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            nodes: HashMap::new(),
            heads: HashMap::from([(MAIN_HEAD.to_owned(), Head::default())]),
            head_order: vec![MAIN_HEAD.to_owned()],
            patches: HashMap::new(),
            redactions: HashMap::new(),
            selected: String::new(),
            collection_items: 0,
            metadata_bytes: 0,
            records: 0,
            damaged: false,
        }
    }

    pub(crate) fn charge_metadata(&mut self, bytes: i64) -> Result<(), DagError> {
        self.metadata_bytes = self.metadata_bytes.saturating_add(bytes);
        if self.metadata_bytes > SESSION_REPLAY_MAX_PAYLOAD_BYTES {
            return Err(DagError::Limit(session_replay_limit_error(
                &self.path,
                "dag_metadata_bytes",
                self.metadata_bytes,
                SESSION_REPLAY_MAX_PAYLOAD_BYTES,
            )));
        }
        Ok(())
    }

    fn head(&mut self, id: &str) -> &mut Head {
        let id = if id.is_empty() { MAIN_HEAD } else { id };
        if !self.heads.contains_key(id) {
            self.heads.insert(id.to_owned(), Head::default());
            self.head_order.push(id.to_owned());
        }
        self.heads.get_mut(id).expect("head was inserted")
    }

    pub(crate) fn selected_head(&self) -> String {
        if let Some(head) = self.heads.get(&self.selected)
            && !head.retired
        {
            return self.selected.clone();
        }
        for include_retired in [false, true] {
            let mut best: Option<&str> = None;
            for id in &self.head_order {
                let head = &self.heads[id];
                if head.retired && !include_retired {
                    continue;
                }
                let replace = match best {
                    None => true,
                    Some(best) => {
                        let current = &self.heads[best];
                        head.last_activity.get() > current.last_activity.get()
                            || (head.last_activity.get() == current.last_activity.get()
                                && head.last_offset > current.last_offset)
                    }
                };
                if replace {
                    best = Some(id);
                }
            }
            if let Some(best) = best {
                return best.to_owned();
            }
        }
        MAIN_HEAD.to_owned()
    }

    pub(crate) fn chain(
        &self,
        head_id: &str,
        limits: &SessionReplayLimits,
    ) -> Result<(Vec<PayloadRef>, usize), DagError> {
        let Some(head) = self.heads.get(head_id) else {
            return Ok((Vec::new(), 0));
        };
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        let mut id = head.leaf.as_str();
        while !id.is_empty() && seen.insert(id) {
            let Some(node) = self.nodes.get(id) else {
                break;
            };
            ids.push(id);
            id = node.parent.as_str();
        }
        ids.reverse();
        let mut refs = Vec::with_capacity(ids.len() + 1);
        for id in ids {
            let mut payload = self.nodes[id].payload.clone();
            if let Some(patch) = self.patches.get(id) {
                payload = patch.clone();
            }
            if let Some(redaction) = self.redactions.get(id) {
                payload = redaction.clone();
            }
            refs.push(payload);
        }
        if let Some(system) = head.system.clone() {
            if refs
                .first()
                .is_some_and(|payload| payload.role == Role::SYSTEM)
            {
                refs[0] = system;
            } else {
                refs.insert(0, system);
            }
        }
        let dropped = if limits.tail_budget > 0 {
            tail_of_dag_payloads(&mut refs, limits.tail_budget)
        } else {
            0
        };
        let mut payload_bytes = 0i64;
        for payload in &refs {
            payload_bytes = payload_bytes.saturating_add(payload.end - payload.start);
            if let Some(err) = limits.payload_err(&self.path, payload_bytes) {
                return Err(DagError::Limit(err));
            }
        }
        Ok((refs, dropped))
    }

    fn apply(
        &mut self,
        entry: &Entry,
        raw: &[u8],
        record_start: i64,
        offset: i64,
        limits: &SessionReplayLimits,
    ) -> Result<bool, DagError> {
        match entry.kind.as_str() {
            "message" => {
                if entry.id.is_empty() {
                    return Ok(false);
                }
                let Some(payload) =
                    self.payload(raw, record_start, entry.message.clone(), limits)?
                else {
                    return Ok(false);
                };
                if self.nodes.contains_key(&entry.id) {
                    return Ok(true);
                }
                self.nodes.insert(
                    entry.id.clone(),
                    Node {
                        parent: entry.parent.clone(),
                        payload,
                    },
                );
                let head = self.head(&entry.head);
                head.leaf = entry.id.clone();
                head.last_activity = entry.at;
                head.last_offset = offset;
            }
            "patch" | "system" | "redact" => {
                return self.apply_overlay(entry, raw, record_start, limits);
            }
            "fork" => {
                if entry.new_head.is_empty() {
                    return Ok(false);
                }
                if !self.heads.contains_key(&entry.new_head) {
                    let system = self.head(&entry.head).system.clone();
                    self.heads.insert(
                        entry.new_head.clone(),
                        Head {
                            leaf: entry.from.clone(),
                            system,
                            last_activity: entry.at,
                            last_offset: offset,
                            ..Head::default()
                        },
                    );
                    self.head_order.push(entry.new_head.clone());
                }
            }
            "rewind" => {
                let head = self.head(&entry.head);
                head.leaf = entry.to.clone();
                head.last_activity = entry.at;
                head.last_offset = offset;
            }
            "select" => self.selected = entry.head.clone(),
            "retire" => self.head(&entry.head).retired = true,
            "log" | "writer" | "rename" | "turn_begin" | "turn_end" | "compaction"
            | "checkpoint" => {}
            _ => {
                return Err(DagError::UnsupportedEntry {
                    path: self.path.clone(),
                    entry_type: entry.kind.clone(),
                });
            }
        }
        Ok(true)
    }

    fn apply_overlay(
        &mut self,
        entry: &Entry,
        raw: &[u8],
        record_start: i64,
        limits: &SessionReplayLimits,
    ) -> Result<bool, DagError> {
        match entry.kind.as_str() {
            "patch" => {
                if !self.nodes.contains_key(&entry.target) {
                    return Ok(true);
                }
                let Some(payload) =
                    self.payload(raw, record_start, entry.message.clone(), limits)?
                else {
                    return Ok(false);
                };
                self.patches.insert(entry.target.clone(), payload);
            }
            "system" => {
                let Some(payload) =
                    self.payload(raw, record_start, entry.message.clone(), limits)?
                else {
                    return Ok(false);
                };
                self.head(&entry.head).system = Some(payload);
            }
            "redact" => {
                for (id, reference) in &entry.targets {
                    let Some(payload) =
                        self.payload(raw, record_start, Some(reference.clone()), limits)?
                    else {
                        return Ok(false);
                    };
                    self.redactions.insert(id.clone(), payload);
                }
            }
            _ => unreachable!("overlay caller filters entry types"),
        }
        Ok(true)
    }

    fn payload(
        &mut self,
        raw: &[u8],
        record_start: i64,
        reference: Option<PayloadRef>,
        limits: &SessionReplayLimits,
    ) -> Result<Option<PayloadRef>, DagError> {
        let Some(reference) = reference else {
            return Ok(None);
        };
        let Ok(begin) = usize::try_from(reference.start - record_start) else {
            return Ok(None);
        };
        let Ok(end) = usize::try_from(reference.end - record_start) else {
            return Ok(None);
        };
        let Some(bytes) = raw.get(begin..end) else {
            return Ok(None);
        };
        let Ok(raw_value) = serde_json::from_slice::<&RawValue>(bytes) else {
            return Ok(None);
        };
        let (messages, items) = match decode_session_event_messages(
            &self.path,
            Some(raw_value),
            self.nodes.len(),
            self.collection_items,
            limits,
        ) {
            Ok(decoded) => decoded,
            Err(DecodeError::Limit(err)) => return Err(DagError::Limit(err)),
            Err(DecodeError::Other(_)) => return Ok(None),
        };
        if messages.len() != 1 {
            return Ok(None);
        }
        self.collection_items = items;
        self.charge_metadata(messages[0].role.as_str().len() as i64)?;
        let mut reference = reference;
        reference.role = messages[0].role.clone();
        Ok(Some(reference))
    }
}

pub(crate) fn replay_session_dag(
    file: &mut File,
    log_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<DagState, DagError> {
    let path = log_path.to_string_lossy().into_owned();
    let size = i64::try_from(file.metadata()?.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "session log is too large"))?;
    let mut state = DagState::new(&path);
    if size > limits.max_bytes {
        return Err(DagError::Limit(session_replay_limit_error(
            &path,
            "encoded_bytes",
            size,
            limits.max_bytes,
        )));
    }
    let mut reader = SessionRecordReader::new(&mut *file, &path, limits.max_bytes);
    loop {
        let (raw, start) = match reader.next() {
            Ok(record) => record,
            Err(NextError::Eof) => return Ok(state),
            Err(NextError::Limit(err)) => return Err(DagError::Limit(err)),
            Err(NextError::Damaged(_)) => {
                state.damaged = true;
                return Ok(state);
            }
        };
        let entry = match parse_entry(&raw, start) {
            Ok(entry) => entry,
            Err(_) => {
                state.damaged = true;
                return Ok(state);
            }
        };
        if entry.schema_version != SESSION_EVENT_DAG_SCHEMA_VERSION {
            return Err(DagError::UnsupportedSchema {
                path,
                version: entry.schema_version,
            });
        }
        if state.records >= limits.max_records {
            return Err(DagError::Limit(session_replay_limit_error(
                &path,
                "event_records",
                state.records as i64 + 1,
                limits.max_records as i64,
            )));
        }
        state.charge_metadata(entry.metadata_bytes())?;
        if !state.apply(&entry, &raw, start, reader.offset(), limits)? {
            state.damaged = true;
            return Ok(state);
        }
        state.records += 1;
    }
}

fn replay_session(
    session_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<(File, DagState), DagError> {
    let log = session_event_log(session_path);
    let mut file = File::open(&log)?;
    let state = replay_session_dag(&mut file, &log, limits)?;
    Ok((file, state))
}

pub fn load_session_dag_messages_with_limits(
    session_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<(Vec<Message>, bool), DagError> {
    let (mut file, state) = replay_session(session_path, limits)?;
    let selected = state.selected_head();
    let (refs, _) = state.chain(&selected, limits)?;
    let path = session_event_log(session_path)
        .to_string_lossy()
        .into_owned();
    let messages = read_dag_payloads(&mut file, &path, &refs, limits)?;
    Ok((messages, state.damaged))
}

pub fn load_session_dag_messages(session_path: &Path) -> Result<(Vec<Message>, bool), DagError> {
    load_session_dag_messages_with_limits(session_path, &SessionReplayLimits::default())
}

pub fn load_session_dag_tail_with_limits(
    session_path: &Path,
    limits: &SessionReplayLimits,
) -> Result<DagTail, DagError> {
    let (mut file, state) = replay_session(session_path, limits)?;
    let selected = state.selected_head();
    let (refs, dropped) = state.chain(&selected, limits)?;
    let path = session_event_log(session_path)
        .to_string_lossy()
        .into_owned();
    let messages = read_dag_payloads(&mut file, &path, &refs, limits)?;
    Ok(DagTail { messages, dropped })
}

pub fn load_session_dag_tail(session_path: &Path) -> Result<DagTail, DagError> {
    let limits = SessionReplayLimits {
        tail_budget: SESSION_TAIL_BUDGET_BYTES,
        ..SessionReplayLimits::default()
    };
    load_session_dag_tail_with_limits(session_path, &limits)
}
