use std::collections::BTreeMap;
use std::fs::{self, DirEntry, File, FileTimes, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use reasonix_contract::provider::{Message, Role, ToolCall};
use reasonix_core::fileencoding::read_file_utf8;
use reasonix_core::fileutil::atomic_write_file;
use serde::Deserialize;
use serde_json::Deserializer;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use super::types::{OperationError, ProjectDirectory};
use crate::sessionstore::branch::load_branch_meta;
use crate::sessionstore::eventlog::probe_session_event_log;
use crate::sessionstore::recovery_trash::list_subagents_by_parent;
use crate::sessionstore::session::{SaveError, Session, load_session};
use crate::store::{
    is_session_transcript_name, is_subagent_transcript_name, session_meta,
    session_path_for_event_log,
};

pub(crate) const ROUTED_HOME: &str = ".legacy-imported.v2-routed";
pub(crate) const ROUTED_CONFIG: &str = ".legacy-imported.v0-events-config.v2-routed";
pub(crate) const JSONL_MARKER: &str = ".legacy-imported.v3-jsonl";
const FLAT_MARKERS: [&str; 3] = [
    ".legacy-imported",
    ".legacy-imported.v0-events-home",
    ".legacy-imported.v0-events-config",
];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegacyMeta {
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyEvent {
    #[serde(default, rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reasoning_content: String,
    #[serde(default)]
    pub tool_calls: Vec<LegacyToolCall>,
    #[serde(default)]
    pub call_id: String,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegacyToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub function: LegacyFunction,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegacyFunction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub thought_signature: String,
}

#[derive(Debug, Deserialize)]
struct LegacyJsonlToolCall {
    #[serde(default)]
    id: String,
    function: Option<LegacyFunction>,
}

pub(crate) fn read_entries(dir: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

pub(crate) fn source_metadata(path: &Path) -> Result<fs::Metadata, OperationError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OperationError::UnreadableFormat);
    }
    Ok(metadata)
}

pub(crate) fn is_event_name(name: &str) -> bool {
    name.ends_with(".events.jsonl")
}

pub(crate) fn event_base(name: &str) -> &str {
    name.strip_suffix(".events.jsonl").unwrap_or(name)
}

pub(crate) fn transcript_base(name: &str) -> &str {
    name.strip_suffix(".jsonl").unwrap_or(name)
}

pub(crate) fn is_backup_name(name: &str) -> bool {
    name.ends_with(".jsonl.bak")
}

pub(crate) fn backup_base(name: &str) -> &str {
    name.strip_suffix(".jsonl.bak").unwrap_or(name)
}

pub(crate) fn is_flat_subagent(base: &str) -> bool {
    is_subagent_transcript_name(std::ffi::OsStr::new(base))
}

pub(crate) fn is_transcript_name(name: &str) -> bool {
    is_session_transcript_name(std::ffi::OsStr::new(name))
}

pub(crate) fn read_legacy_meta(dir: &Path, base: &str) -> LegacyMeta {
    let Ok(bytes) = read_file_utf8(&dir.join(format!("{base}.meta.json"))) else {
        return LegacyMeta::default();
    };
    let Ok(mut meta) = serde_json::from_slice::<LegacyMeta>(&bytes) else {
        return LegacyMeta::default();
    };
    meta.workspace = meta.workspace.trim().to_owned();
    meta.summary = meta.summary.trim().to_owned();
    meta
}

pub(crate) fn live_directory(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

pub(crate) fn route_legacy(
    meta: &LegacyMeta,
    global: &Path,
    router: Option<&ProjectDirectory<'_>>,
) -> PathBuf {
    if let Some(router) = router
        && !meta.workspace.trim().is_empty()
        && live_directory(Path::new(meta.workspace.trim()))
        && let Some(dest) = router(Path::new(meta.workspace.trim()))
        && !dest.as_os_str().is_empty()
    {
        return dest;
    }
    global.to_path_buf()
}

pub(crate) fn route_project(
    workspace: &str,
    router: Option<&ProjectDirectory<'_>>,
) -> Option<PathBuf> {
    let router = router?;
    let workspace = workspace.trim();
    if workspace.is_empty() || !live_directory(Path::new(workspace)) {
        return None;
    }
    router(Path::new(workspace)).filter(|dest| !dest.as_os_str().is_empty())
}

pub(crate) fn destination_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

pub(crate) fn same_directory(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => normalized_absolute(left) == normalized_absolute(right),
    }
}

pub(crate) fn normalized_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other.as_os_str()),
        }
    }
    normal
}

pub(crate) fn explicit_marker(src: &Path) -> String {
    let digest = Sha256::digest(normalized_absolute(src).to_string_lossy().as_bytes());
    let key: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(".legacy-imported.explicit.{key}")
}

pub(crate) fn marker_exists(dest: &Path, marker: &str) -> bool {
    !dest.as_os_str().to_string_lossy().trim().is_empty() && fs::metadata(dest.join(marker)).is_ok()
}

pub(crate) fn stamp_markers(dest: &Path, routed: &str, jsonl: &str) -> io::Result<()> {
    if dest.as_os_str().to_string_lossy().trim().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    let mut seen = Vec::new();
    for marker in [routed, jsonl].into_iter().chain(FLAT_MARKERS) {
        if marker.trim().is_empty() || seen.contains(&marker) {
            continue;
        }
        seen.push(marker);
        let path = dest.join(marker);
        if OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .is_err()
        {
            continue;
        }
        let _ = set_modified(&path, SystemTime::now());
    }
    Ok(())
}

pub(crate) fn set_modified(path: &Path, at: SystemTime) -> io::Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .set_times(FileTimes::new().set_modified(at))
}

pub(crate) fn is_message_format(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut bytes = [0_u8; 64];
    let count = file.read(&mut bytes).unwrap_or_default();
    String::from_utf8_lossy(&bytes[..count])
        .trim_start_matches([' ', '\t', '\r', '\n'])
        .starts_with("{\"role\":")
}

pub(crate) fn native_event_log(path: &Path) -> bool {
    let transcript = session_path_for_event_log(path);
    probe_session_event_log(&transcript).is_ok_and(|probe| probe.native && probe.size > 0)
}

pub(crate) fn native_session(path: &Path) -> bool {
    probe_session_event_log(path).is_ok_and(|probe| probe.native && probe.size > 0)
}

pub(crate) fn save_messages(messages: Vec<Message>, dest: &Path) -> Result<(), OperationError> {
    save_if_absent(messages, dest)
}

fn save_if_absent(messages: Vec<Message>, dest: &Path) -> Result<(), OperationError> {
    let session = Session::new("");
    for message in messages {
        session.add(message);
    }
    match session.save_if_absent(dest.to_string_lossy().as_ref()) {
        Ok(()) => Ok(()),
        Err(SaveError::Exists) => Err(OperationError::Exists),
        Err(error) => Err(OperationError::Save(error)),
    }
}

pub(crate) fn copy_session_file(source: &Path, dest: &Path) -> Result<(), OperationError> {
    if native_session(source) {
        let session = load_session(source.to_string_lossy().as_ref())?;
        return match session.save_if_absent(dest.to_string_lossy().as_ref()) {
            Ok(()) => Ok(()),
            Err(SaveError::Exists) => Err(OperationError::Exists),
            Err(error) => Err(OperationError::Save(error)),
        };
    }
    transform_and_copy_jsonl(source, dest)
}

pub(crate) fn transform_and_copy_jsonl(source: &Path, dest: &Path) -> Result<(), OperationError> {
    source_metadata(source)?;
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = temp_path(parent, ".session");
    let result = write_transformed(source, &tmp).and_then(|()| publish_no_replace(&tmp, dest));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_transformed(source: &Path, tmp: &Path) -> Result<(), OperationError> {
    let input = File::open(source)?;
    let mut output = OpenOptions::new().create_new(true).write(true).open(tmp)?;
    let decoder = Deserializer::from_reader(BufReader::new(input)).into_iter::<Box<RawValue>>();
    for raw in decoder {
        let Ok(raw) = raw else {
            break;
        };
        if let Some(transformed) = flatten_nested_calls(&raw) {
            output.write_all(&transformed)?;
        } else {
            output.write_all(raw.get().as_bytes())?;
        }
        output.write_all(b"\n")?;
    }
    output.flush()?;
    Ok(())
}

fn flatten_nested_calls(raw: &RawValue) -> Option<Vec<u8>> {
    let mut record: BTreeMap<String, Box<RawValue>> = serde_json::from_str(raw.get()).ok()?;
    let role: String = serde_json::from_str(record.get("role")?.get()).ok()?;
    if role != "assistant" {
        return None;
    }
    let calls =
        serde_json::from_str::<Vec<LegacyJsonlToolCall>>(record.get("tool_calls")?.get()).ok()?;
    if calls.is_empty() {
        return None;
    }
    let flat: Vec<serde_json::Value> = calls
        .into_iter()
        .map(|call| {
            let function = call.function?;
            serde_json::to_value(ToolCall {
                id: call.id,
                name: function.name,
                arguments: function.arguments,
                thought_signature: function.thought_signature,
                ..Default::default()
            })
            .ok()
        })
        .collect::<Option<_>>()?;
    let json = serde_json::to_vec(&flat).ok()?;
    record.insert(
        "tool_calls".to_owned(),
        RawValue::from_string(String::from_utf8(json).ok()?).ok()?,
    );
    serde_json::to_vec(&record).ok()
}

fn temp_path(parent: &Path, prefix: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        "{prefix}.{}.{stamp}.{sequence}.tmp",
        std::process::id()
    ))
}

fn publish_no_replace(tmp: &Path, dest: &Path) -> Result<(), OperationError> {
    match fs::hard_link(tmp, dest) {
        Ok(()) => {
            fs::remove_file(tmp)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(OperationError::Exists),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn reconstruct_session(path: &Path) -> Result<Vec<Message>, OperationError> {
    source_metadata(path)?;
    let input = File::open(path)?;
    let mut messages = Vec::new();
    let mut tool_names = BTreeMap::new();
    for event in Deserializer::from_reader(BufReader::new(input)).into_iter::<LegacyEvent>() {
        let Ok(event) = event else {
            break;
        };
        match event.type_.as_str() {
            "user.message" if !event.text.is_empty() => messages.push(Message {
                role: Role::USER,
                content: event.text,
                ..Default::default()
            }),
            "model.final" => {
                let calls: Vec<ToolCall> = event
                    .tool_calls
                    .into_iter()
                    .map(|call| {
                        tool_names.insert(call.id.clone(), call.function.name.clone());
                        ToolCall {
                            id: call.id,
                            name: call.function.name,
                            arguments: call.function.arguments,
                            thought_signature: call.function.thought_signature,
                            ..Default::default()
                        }
                    })
                    .collect();
                messages.push(Message {
                    role: Role::ASSISTANT,
                    content: event.content,
                    reasoning_content: event.reasoning_content,
                    tool_calls: calls,
                    ..Default::default()
                });
            }
            "tool.result" => {
                let name = tool_names.get(&event.call_id).cloned().unwrap_or_default();
                messages.push(Message {
                    role: Role::TOOL,
                    tool_call_id: event.call_id,
                    name,
                    content: event.output,
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    Ok(messages)
}

pub(crate) fn record_imported_title(dest: &Path, base: &str, summary: &str) {
    let summary = summary.trim();
    if summary.is_empty() {
        return;
    }
    let path = dest.join(".titles.json");
    let mut titles = read_file_utf8(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok())
        .unwrap_or_default();
    let key = format!("{base}.jsonl");
    if titles.get(&key).is_some_and(|title| !title.is_empty()) {
        return;
    }
    titles.insert(key, summary.to_owned());
    let Ok(bytes) = serde_json::to_vec_pretty(&titles) else {
        return;
    };
    let _ = atomic_write_file(&path, &bytes, 0o644);
}

pub(crate) fn copy_branch_meta(source: &Path, dest: &Path) -> Result<(), OperationError> {
    let source_meta = session_meta(source);
    let bytes = match fs::read(&source_meta) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    atomic_write_file(&session_meta(dest), &bytes, 0o644)?;
    Ok(())
}

pub(crate) fn complete_branch_meta(source: &Path, dest: &Path) -> Result<bool, OperationError> {
    if load_branch_meta(dest)?.is_some() {
        return Ok(false);
    }
    copy_branch_meta(source, dest)?;
    Ok(session_meta(dest).exists())
}

pub(crate) fn copy_subagent_artifacts(
    source_dir: &Path,
    dest_dir: &Path,
    parent: &str,
) -> Result<(), OperationError> {
    if same_directory(source_dir, dest_dir) {
        return Ok(());
    }
    let artifacts = list_subagents_by_parent(source_dir, parent)?;
    let mut first_error = None;
    for artifact in artifacts {
        for source in [&artifact.session_path, &artifact.meta_path] {
            if let Err(error) = copy_file_if_exists(
                source,
                &dest_dir
                    .join("subagents")
                    .join(source.file_name().unwrap_or_default()),
            ) {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(crate) fn copy_file_if_exists(source: &Path, dest: &Path) -> Result<(), OperationError> {
    let metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() || destination_exists(dest) {
        return Ok(());
    }
    let bytes = fs::read(source)?;
    atomic_write_file(dest, &bytes, 0o644)?;
    if let Ok(at) = metadata.modified() {
        let _ = set_modified(dest, at);
    }
    Ok(())
}

pub(crate) fn jsonl_destination(
    source_dir: &Path,
    source: &Path,
    base: &str,
    global: &Path,
    router: Option<&ProjectDirectory<'_>>,
) -> (PathBuf, String, bool) {
    if let Ok(Some(meta)) = load_branch_meta(source) {
        let summary = meta.topic_title.trim().to_owned();
        let scope = if meta.scope.trim().is_empty() {
            "global"
        } else {
            meta.scope.trim()
        };
        if scope == "project"
            && let Some(dest) = route_project(&meta.workspace_root, router)
        {
            return (dest, summary, true);
        }
        if !meta.scope.trim().is_empty() {
            return (global.to_path_buf(), summary, scope == "global");
        }
    }
    let legacy = read_legacy_meta(source_dir, base);
    (route_legacy(&legacy, global, router), legacy.summary, false)
}

pub(crate) fn stranded_destination(
    source_dir: &Path,
    source: &Path,
    base: &str,
    global: &Path,
    router: Option<&ProjectDirectory<'_>>,
) -> Option<(PathBuf, String)> {
    if let Ok(Some(meta)) = load_branch_meta(source) {
        let scope = if meta.scope.trim().is_empty() {
            "global"
        } else {
            meta.scope.trim()
        };
        if scope == "project"
            && let Some(dest) = route_project(&meta.workspace_root, router)
            && !same_directory(&dest, global)
        {
            return Some((dest, meta.topic_title.trim().to_owned()));
        }
        if !meta.scope.trim().is_empty() {
            return None;
        }
    }
    let legacy = read_legacy_meta(source_dir, base);
    let dest = route_legacy(&legacy, global, router);
    if same_directory(&dest, global) {
        None
    } else {
        Some((dest, legacy.summary))
    }
}
