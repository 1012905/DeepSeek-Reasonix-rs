use std::collections::BTreeMap;
use std::fs::{self, DirEntry};
use std::io;
use std::path::Path;
use std::time::SystemTime;

use super::native::rehome_flat_import;
use super::types::{LegacyReport, OperationError, ProjectDirectory, SkipReason};
use super::util::{
    backup_base, complete_branch_meta, copy_branch_meta, copy_session_file,
    copy_subagent_artifacts, destination_exists, event_base, is_backup_name, is_event_name,
    is_flat_subagent, is_message_format, is_transcript_name, jsonl_destination, marker_exists,
    native_event_log, native_session, read_entries, read_legacy_meta, reconstruct_session,
    record_imported_title, route_legacy, same_directory, save_messages, set_modified,
    source_metadata, stamp_markers, stranded_destination, transcript_base,
    transform_and_copy_jsonl,
};

#[derive(Debug, Default)]
pub(crate) struct PassOutcome {
    pub imported: usize,
    pub incomplete: bool,
}

impl PassOutcome {
    pub fn merge(&mut self, other: Self) {
        self.imported += other.imported;
        self.incomplete |= other.incomplete;
    }
}

pub(crate) fn run_import(
    src: &Path,
    global: &Path,
    routed: &str,
    jsonl_marker: &str,
    explicit: bool,
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> Result<usize, super::types::LegacyMigrationError> {
    if !explicit && marker_exists(global, routed) && marker_exists(global, jsonl_marker) {
        return Ok(rehome_stranded(src, global, routed, router));
    }
    let entries = match read_entries(src) {
        Ok(entries) => entries,
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(0);
            }
            let reason = source_error_reason(&error);
            if explicit {
                report.skip(src, reason);
                return Ok(0);
            }
            return Err(super::types::LegacySourceError {
                skipped: super::types::SkippedSession {
                    name: src
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path: src.to_path_buf(),
                    reason,
                },
                source: error,
            }
            .into());
        }
    };
    let has_events = legacy_event_bases(src, &entries);
    let mut outcome = import_events(src, global, &entries, router, report);
    if explicit || !marker_exists(global, jsonl_marker) {
        outcome.merge(import_jsonl(
            src,
            global,
            &entries,
            &has_events,
            router,
            report,
        ));
        outcome.merge(import_backups(
            src,
            global,
            &entries,
            &has_events,
            router,
            report,
        ));
    }
    outcome.merge(import_project_subdirs(
        src, global, &entries, router, report,
    ));
    if outcome.incomplete {
        return Ok(outcome.imported);
    }
    let _ = stamp_markers(global, routed, jsonl_marker);
    Ok(outcome.imported)
}

fn source_error_reason(error: &io::Error) -> SkipReason {
    if error.kind() == io::ErrorKind::PermissionDenied {
        SkipReason::Permission
    } else {
        SkipReason::Corrupt
    }
}

fn legacy_event_bases(src: &Path, entries: &[DirEntry]) -> BTreeMap<String, bool> {
    let mut bases = BTreeMap::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir())
            && is_event_name(&name)
            && !native_event_log(&src.join(&name))
        {
            bases.insert(event_base(&name).to_owned(), true);
        }
    }
    bases
}

fn import_events(
    src: &Path,
    global: &Path,
    entries: &[DirEntry],
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry_is_dir(entry, &src.join(&name), report) || !is_event_name(&name) {
            continue;
        }
        let source = src.join(&name);
        if native_event_log(&source) {
            continue;
        }
        let base = event_base(&name);
        let meta = read_legacy_meta(src, base);
        let dest_dir = route_legacy(&meta, global, router);
        let dest = dest_dir.join(format!("{base}.jsonl"));
        let events_info = source_metadata(&source).ok();
        let old = global.join(format!("{base}.jsonl"));
        let may_retire = !same_file_path(&old, &src.join(format!("{base}.jsonl")));
        let (count, deferred) = rehome_flat_import(
            &old,
            &dest,
            &dest_dir,
            base,
            &meta.summary,
            events_info.as_ref(),
            may_retire,
        );
        outcome.imported += count;
        outcome.incomplete |= deferred;
        if count > 0 || deferred || destination_exists(&dest) {
            continue;
        }
        let sidecar = src.join(format!("{base}.jsonl"));
        if sidecar_is_current(&sidecar, events_info.as_ref())
            && transform_and_copy_jsonl(&sidecar, &dest).is_ok()
        {
            preserve_mtime(&dest, events_info.as_ref());
            record_imported_title(&dest_dir, base, &meta.summary);
            outcome.imported += 1;
            continue;
        }
        let messages = match reconstruct_session(&source) {
            Ok(messages) => messages,
            Err(error) => {
                report.skip_error(&source, &error);
                continue;
            }
        };
        if messages.is_empty() {
            if events_info.as_ref().is_some_and(|info| info.len() > 0) {
                report.skip(&source, SkipReason::UnreadableFormat);
            }
            continue;
        }
        match save_messages(messages, &dest) {
            Ok(()) => {
                preserve_mtime(&dest, events_info.as_ref());
                record_imported_title(&dest_dir, base, &meta.summary);
                outcome.imported += 1;
            }
            Err(OperationError::Exists) => {}
            Err(error) => {
                report.skip_write(&source, &error);
                outcome.incomplete = true;
            }
        }
    }
    outcome
}

fn sidecar_is_current(sidecar: &Path, events_info: Option<&fs::Metadata>) -> bool {
    let Ok(sidecar_info) = source_metadata(sidecar) else {
        return false;
    };
    if !is_message_format(sidecar) {
        return false;
    }
    match (
        sidecar_info.modified(),
        events_info.and_then(|info| info.modified().ok()),
    ) {
        (Ok(sidecar_time), Some(event_time)) => sidecar_time >= event_time,
        _ => true,
    }
}

fn import_jsonl(
    src: &Path,
    global: &Path,
    entries: &[DirEntry],
    has_events: &BTreeMap<String, bool>,
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry_is_dir(entry, &src.join(&name), report) || !is_transcript_name(&name) {
            continue;
        }
        let base = transcript_base(&name);
        if has_events.contains_key(base) || is_flat_subagent(base) {
            continue;
        }
        let source = src.join(&name);
        if !is_message_format(&source) {
            if entry.metadata().is_ok_and(|info| info.len() > 0) {
                report.skip(&source, SkipReason::UnreadableFormat);
            }
            continue;
        }
        let (dest_dir, summary, branch_owned) =
            jsonl_destination(src, &source, base, global, router);
        let dest = dest_dir.join(&name);
        if destination_exists(&dest) {
            if branch_owned {
                complete_existing(&source, &dest, src, &dest_dir, base, &summary, &mut outcome);
            }
            continue;
        }
        let source_info = source_metadata(&source).ok();
        if let Err(error) = copy_session_file(&source, &dest) {
            report.skip_write(&source, &error);
            outcome.incomplete = true;
            continue;
        }
        preserve_mtime(&dest, source_info.as_ref());
        if branch_owned {
            if copy_branch_meta(&source, &dest).is_err() {
                outcome.incomplete = true;
                continue;
            }
            if copy_subagent_artifacts(src, &dest_dir, base).is_err() {
                outcome.incomplete = true;
            }
        }
        record_imported_title(&dest_dir, base, &summary);
        outcome.imported += 1;
    }
    outcome
}

fn complete_existing(
    source: &Path,
    dest: &Path,
    src: &Path,
    dest_dir: &Path,
    base: &str,
    summary: &str,
    outcome: &mut PassOutcome,
) {
    if copy_subagent_artifacts(src, dest_dir, base).is_err() {
        outcome.incomplete = true;
    }
    match complete_branch_meta(source, dest) {
        Ok(true) => {
            record_imported_title(dest_dir, base, summary);
            outcome.imported += 1;
        }
        Ok(false) => {}
        Err(_) => {
            outcome.incomplete = true;
        }
    }
}

fn import_backups(
    src: &Path,
    global: &Path,
    entries: &[DirEntry],
    has_events: &BTreeMap<String, bool>,
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry_is_dir(entry, &src.join(&name), report) || !is_backup_name(&name) {
            continue;
        }
        let base = backup_base(&name);
        if has_events.contains_key(base)
            || fs::symlink_metadata(src.join(format!("{base}.jsonl"))).is_ok()
        {
            continue;
        }
        let source = src.join(&name);
        if !is_message_format(&source) {
            report.skip(&source, SkipReason::UnreadableFormat);
            continue;
        }
        let meta = read_legacy_meta(src, base);
        let dest_dir = route_legacy(&meta, global, router);
        let dest = dest_dir.join(format!("{base}.jsonl"));
        if destination_exists(&dest) {
            continue;
        }
        let source_info = source_metadata(&source).ok();
        if let Err(error) = transform_and_copy_jsonl(&source, &dest) {
            report.skip_write(&source, &error);
            outcome.incomplete = true;
            continue;
        }
        preserve_mtime(&dest, source_info.as_ref());
        record_imported_title(&dest_dir, base, &meta.summary);
        outcome.imported += 1;
    }
    outcome
}

fn import_project_subdirs(
    src: &Path,
    global: &Path,
    entries: &[DirEntry],
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "subagents" || !entry_is_dir(entry, &src.join(&name), report) {
            continue;
        }
        let sub = src.join(&name);
        let sub_entries = match read_entries(&sub) {
            Ok(entries) => entries,
            Err(error) => {
                report.skip(&sub, source_error_reason(&error));
                continue;
            }
        };
        let has_sessions = sub_entries.iter().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !entry.file_type().is_ok_and(|kind| kind.is_dir())
                && (is_transcript_name(&name) || is_event_name(&name))
        });
        if has_sessions {
            outcome.merge(import_subdirectory(
                &sub,
                global,
                &sub_entries,
                router,
                report,
            ));
        }
    }
    outcome
}

fn import_subdirectory(
    src: &Path,
    global: &Path,
    entries: &[DirEntry],
    router: Option<&ProjectDirectory<'_>>,
    report: &mut LegacyReport,
) -> PassOutcome {
    let mut outcome = PassOutcome::default();
    let has_events: BTreeMap<String, bool> = entries
        .iter()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !entry.file_type().is_ok_and(|kind| kind.is_dir())
                && is_event_name(&name)
                && !native_event_log(&src.join(&name))
        })
        .map(|entry| event_base(&entry.file_name().to_string_lossy()).to_owned())
        .map(|base| (base, true))
        .collect();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry_is_dir(entry, &src.join(&name), report) {
            continue;
        }
        let (base, source, reconstruct) = if is_event_name(&name) {
            if native_event_log(&src.join(&name)) {
                continue;
            }
            let base = event_base(&name).to_owned();
            let sidecar = src.join(format!("{base}.jsonl"));
            let reconstruct = !sidecar_is_current(&sidecar, entry.metadata().ok().as_ref());
            (
                base,
                if reconstruct {
                    src.join(&name)
                } else {
                    sidecar
                },
                reconstruct,
            )
        } else if is_transcript_name(&name) {
            let base = transcript_base(&name).to_owned();
            if has_events.contains_key(&base) {
                continue;
            }
            let source = src.join(&name);
            if !is_message_format(&source) {
                if entry.metadata().is_ok_and(|info| info.len() > 0) {
                    report.skip(&source, SkipReason::UnreadableFormat);
                }
                continue;
            }
            (base, source, false)
        } else {
            continue;
        };
        let meta = read_legacy_meta(src, &base);
        let dest_dir = route_legacy(&meta, global, router);
        let dest = dest_dir.join(format!("{base}.jsonl"));
        if destination_exists(&dest) {
            continue;
        }
        let original_info = entry.metadata().ok();
        let result = if reconstruct {
            let messages = match reconstruct_session(&source) {
                Ok(messages) if messages.is_empty() => continue,
                Ok(messages) => messages,
                Err(error) => {
                    report.skip_error(&source, &error);
                    continue;
                }
            };
            save_messages(messages, &dest)
        } else if native_session(&source) {
            copy_session_file(&source, &dest)
        } else {
            transform_and_copy_jsonl(&source, &dest)
        };
        match result {
            Ok(()) => {
                preserve_mtime(&dest, original_info.as_ref());
                record_imported_title(&dest_dir, &base, &meta.summary);
                outcome.imported += 1;
            }
            Err(OperationError::Exists) => {}
            Err(error) => {
                if !reconstruct && native_session(&source) {
                    report.skip_error(&source, &error);
                } else {
                    report.skip_write(&source, &error);
                }
                outcome.incomplete = true;
            }
        }
    }
    outcome
}

pub(crate) fn rehome_stranded(
    src: &Path,
    global: &Path,
    routed: &str,
    router: Option<&ProjectDirectory<'_>>,
) -> usize {
    let Some(router) = router else {
        return 0;
    };
    let marker = global.join(routed);
    let Ok(watermark) = fs::metadata(&marker).and_then(|info| info.modified()) else {
        return 0;
    };
    let Ok(entries) = read_entries(src) else {
        return 0;
    };
    let mut imported = 0;
    let mut failed = false;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            || !is_transcript_name(&name)
            || is_backup_name(&name)
        {
            continue;
        }
        let base = transcript_base(&name);
        if is_flat_subagent(base) {
            continue;
        }
        let source = src.join(&name);
        let Ok(info) = source_metadata(&source) else {
            continue;
        };
        if !info.modified().is_ok_and(|at| at > watermark) || !is_message_format(&source) {
            continue;
        }
        let Some((dest_dir, summary)) =
            stranded_destination(src, &source, base, global, Some(router))
        else {
            continue;
        };
        if same_directory(&dest_dir, global) {
            continue;
        }
        let dest = dest_dir.join(&name);
        if destination_exists(&dest) {
            if copy_subagent_artifacts(src, &dest_dir, base).is_err() {
                failed = true;
            }
            match complete_branch_meta(&source, &dest) {
                Ok(true) => record_imported_title(&dest_dir, base, &summary),
                Ok(false) => {}
                Err(_) => failed = true,
            }
            continue;
        }
        if transform_and_copy_jsonl(&source, &dest).is_err() {
            failed = true;
            continue;
        }
        preserve_mtime(&dest, Some(&info));
        if copy_branch_meta(&source, &dest).is_err()
            || copy_subagent_artifacts(src, &dest_dir, base).is_err()
        {
            failed = true;
        }
        record_imported_title(&dest_dir, base, &summary);
        imported += 1;
    }
    if !failed {
        let _ = set_modified(&marker, SystemTime::now());
    }
    imported
}

fn entry_is_dir(entry: &DirEntry, path: &Path, report: &mut LegacyReport) -> bool {
    match entry.file_type() {
        Ok(kind) => kind.is_dir(),
        Err(error) => {
            report.skip_error(path, &OperationError::Io(error));
            false
        }
    }
}

fn same_file_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => super::util::normalized_absolute(left) == super::util::normalized_absolute(right),
    }
}

fn preserve_mtime(dest: &Path, info: Option<&fs::Metadata>) {
    if let Some(at) = info.and_then(|info| info.modified().ok()) {
        let _ = set_modified(dest, at);
    }
}
