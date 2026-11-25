use std::io::{self, Read, Seek, SeekFrom};

use reasonix_contract::provider::{Message, Role};
use serde_json::value::RawValue;

use crate::sessionstore::eventlog::replay::decode_session_event_messages;
use crate::sessionstore::eventlog::{DecodeError, SessionReplayLimits};

use super::DagError;
use super::json::has_host_origin;
use super::replay::PayloadRef;

#[derive(Debug, Default)]
pub struct DagTail {
    pub messages: Vec<Message>,
    pub dropped: usize,
}

pub(crate) fn read_dag_payloads<R: Read + Seek>(
    reader: &mut R,
    path: &str,
    refs: &[PayloadRef],
    limits: &SessionReplayLimits,
) -> Result<Vec<Message>, DagError> {
    let mut messages = Vec::with_capacity(refs.len());
    for reference in refs {
        let size = usize::try_from(reference.end - reference.start)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid payload range"))?;
        let start = u64::try_from(reference.start)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid payload offset"))?;
        reader.seek(SeekFrom::Start(start))?;
        let mut raw = vec![0u8; size];
        reader.read_exact(&mut raw)?;
        let raw_value = serde_json::from_slice::<&RawValue>(&raw)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let (decoded, _) = match decode_session_event_messages(path, Some(raw_value), 0, 0, limits)
        {
            Ok(decoded) => decoded,
            Err(DecodeError::Limit(err)) => return Err(DagError::Limit(err)),
            Err(DecodeError::Other(err)) => {
                return Err(DagError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    err,
                )));
            }
        };
        if decoded.len() != 1 {
            return Err(DagError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected session event must hold one message",
            )));
        }
        let mut message = decoded.into_iter().next().expect("one message");
        if has_host_origin(&raw).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))? {
            message.host_authored = true;
        }
        messages.push(message);
    }
    Ok(messages)
}

pub(crate) fn tail_of_dag_payloads(refs: &mut Vec<PayloadRef>, budget: i64) -> usize {
    let start = if refs
        .first()
        .is_some_and(|payload| payload.role == Role::SYSTEM)
    {
        1
    } else {
        0
    };
    let mut total = refs.iter().fold(0i64, |total, payload| {
        total.saturating_add(payload.end - payload.start)
    });
    let mut cut = start;
    while cut + 1 < refs.len() && total > budget {
        total = total.saturating_sub(refs[cut].end - refs[cut].start);
        cut += 1;
    }
    if cut == start {
        return 0;
    }
    for (next, payload) in refs.iter().enumerate().skip(cut) {
        if payload.role == Role::USER {
            cut = next;
            break;
        }
    }
    let dropped = cut - start;
    refs.drain(start..cut);
    dropped
}
