use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::*;

pub struct Fixture {
    pub _temp: TempDir,
    pub root: TempDir,
    pub dir: PathBuf,
    pub store: Store,
}

pub fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = temp.path().join("session.ckpt");
    let store = Store::new(Some(&dir), root.path());
    Fixture {
        _temp: temp,
        root,
        dir,
        store,
    }
}

pub fn write_bytes(root: &Path, path: &str, content: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

pub fn write(root: &Path, path: &str, content: &str) {
    write_bytes(root, path, content.as_bytes());
}

pub fn read(root: &Path, path: &str) -> Vec<u8> {
    fs::read(root.join(path)).unwrap()
}

pub fn live_mode() -> u32 {
    #[cfg(windows)]
    {
        0o666
    }
    #[cfg(not(windows))]
    {
        0o644
    }
}

pub fn checkpoint(turn: i64, files: Vec<FileSnap>) -> Checkpoint {
    let coverage = if files.is_empty() {
        Coverage::None
    } else {
        Coverage::Complete
    };
    Checkpoint {
        schema_version: SCHEMA_V2,
        turn,
        time: CreatedAt::now(),
        coverage,
        files,
        ..Checkpoint::default()
    }
}

pub fn owned_file(
    path: &str,
    before: Option<&str>,
    after_existed: Option<bool>,
    after_sha: &str,
    after_mode: u32,
) -> FileSnap {
    FileSnap {
        path: path.to_owned(),
        content: before.map(str::to_owned),
        sha256: before
            .map(|content| digest(content.as_bytes()))
            .unwrap_or_default(),
        mode: live_mode(),
        after_existed,
        after_sha256: after_sha.to_owned(),
        after_mode,
        ..FileSnap::default()
    }
}

pub fn load(root: &Path, dir: &Path, checkpoints: &[Checkpoint]) -> Store {
    fs::create_dir_all(dir).unwrap();
    for checkpoint in checkpoints {
        write_json_atomic(&turn_path(dir, checkpoint.turn, false), checkpoint).unwrap();
    }
    Store::new(Some(dir), root)
}

pub fn manifest_path(dir: &Path, id: &str) -> PathBuf {
    transaction_dir(dir).join(format!("{id}.json"))
}

pub fn transaction_paths(dir: &Path) -> Vec<PathBuf> {
    let directory = transaction_dir(dir);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Vec::new();
        }
        Err(error) => panic!("read transaction directory {directory:?}: {error}"),
    };
    let mut paths: Vec<_> = entries
        .map(|entry| entry.expect("read transaction directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
}

pub fn single_manifest(dir: &Path) -> (TransactionManifest, PathBuf) {
    let paths = transaction_paths(dir);
    assert_eq!(
        paths.len(),
        1,
        "expected exactly one transaction manifest in {dir:?}, found {paths:?}"
    );
    let path = paths.into_iter().next().expect("single manifest");
    let value: TransactionManifest = read_json_file(&path).unwrap();
    assert_eq!(manifest_path(dir, &value.id), path);
    (value, path)
}

pub fn assert_no_manifest(dir: &Path) {
    let paths = transaction_paths(dir);
    assert!(
        paths.is_empty(),
        "expected no transaction manifests in {dir:?}, found {paths:?}"
    );
}

pub fn save_manifest(dir: &Path, value: &TransactionManifest) {
    write_json_atomic(&manifest_path(dir, &value.id), value).unwrap();
}

pub fn plan(store: &Store, scope: RewindScope) -> RewindPlan {
    store.prepare_rewind(0, scope, 7, 0, scope != RewindScope::Code)
}

pub fn active_writer() -> ActiveWriter {
    ActiveWriter {
        id: "writer-1".into(),
        turn: 1,
        started_at: CreatedAt::now(),
        kind: "background".into(),
    }
}

pub fn modify_add_delete_fixture() -> Fixture {
    let mut fixture = fixture();
    write(fixture.root.path(), "a.txt", "a-after");
    write(fixture.root.path(), "b.txt", "b-created");
    let checkpoints = [checkpoint(
        0,
        vec![
            owned_file(
                "a.txt",
                Some("a-before"),
                Some(true),
                &digest(b"a-after"),
                live_mode(),
            ),
            owned_file(
                "b.txt",
                None,
                Some(true),
                &digest(b"b-created"),
                live_mode(),
            ),
            owned_file("c.txt", Some("c-before"), Some(false), "", 0),
        ],
    )];
    fixture.store = load(fixture.root.path(), &fixture.dir, &checkpoints);
    fixture
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationCall {
    ApplyConversationTruncate { boundary: i64, forward: Vec<u8> },
    RestoreConversation(Vec<u8>),
    TruncateCheckpoints(i64),
    RestoreCheckpoints(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct MockApplier {
    pub calls: Mutex<Vec<ConversationCall>>,
    pub failures: Mutex<Vec<ConversationPhase>>,
}

impl MockApplier {
    pub fn fail_at(&self, phase: ConversationPhase) {
        self.failures.lock().unwrap().push(phase);
    }

    fn result(&self, phase: ConversationPhase) -> Result<(), ConversationError> {
        if self.failures.lock().unwrap().contains(&phase) {
            Err(ConversationError::injected(phase))
        } else {
            Ok(())
        }
    }
}

impl ConversationApplier for MockApplier {
    fn apply_conversation_truncate(
        &self,
        boundary: i64,
        forward: &[u8],
    ) -> Result<(), ConversationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ConversationCall::ApplyConversationTruncate {
                boundary,
                forward: forward.to_vec(),
            });
        self.result(ConversationPhase::ApplyConversationTruncate)
    }

    fn restore_conversation(&self, forward: &[u8]) -> Result<(), ConversationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ConversationCall::RestoreConversation(forward.to_vec()));
        self.result(ConversationPhase::RestoreConversation)
    }

    fn truncate_checkpoints(&self, from_turn: i64) -> Result<(), ConversationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ConversationCall::TruncateCheckpoints(from_turn));
        self.result(ConversationPhase::TruncateCheckpoints)
    }

    fn restore_checkpoints(&self, backup: &[u8]) -> Result<(), ConversationError> {
        self.calls
            .lock()
            .unwrap()
            .push(ConversationCall::RestoreCheckpoints(backup.to_vec()));
        self.result(ConversationPhase::RestoreCheckpoints)
    }
}
