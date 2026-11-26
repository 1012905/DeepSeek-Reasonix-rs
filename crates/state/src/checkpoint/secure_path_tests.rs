use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use super::*;

fn workspace() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    (temp, root)
}

fn assert_outside(error: SecureError) {
    assert!(
        matches!(error, SecureError::Path(PathError::OutsideWorkspace { .. })),
        "unexpected error: {error}"
    );
}

#[test]
fn secure_path_rejects_relative_and_absolute_escapes() {
    let (temp, root) = workspace();
    let sibling = temp.path().join("workspace-other");
    fs::create_dir(&sibling).unwrap();
    let outside = sibling.join("secret.txt");
    fs::write(&outside, b"keep").unwrap();

    assert_outside(write_new_file(&root, Path::new("../escape.txt"), b"no", 0o600).unwrap_err());
    assert!(!temp.path().join("escape.txt").exists());

    let results = [
        open_workspace_file(&root, &outside).map(|_| ()),
        write_new_file(&root, &outside, b"no", 0o600),
        rename(&root, &root.join("source"), &outside),
        remove(&root, &outside),
        chmod(&root, &outside, 0o600),
        path_exists(&root, &outside).map(|_| ()),
        read_file(&root, &outside).map(|_| ()),
    ];
    for result in results {
        assert_outside(result.unwrap_err());
    }
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
}

#[test]
fn secure_path_rejects_relative_target_without_root() {
    let temp = tempfile::tempdir().unwrap();
    let error = open_workspace_file(Path::new(""), Path::new("file.txt")).unwrap_err();
    assert!(matches!(
        error,
        SecureError::RelativeWithoutRoot(ref path) if path == Path::new("file.txt")
    ));

    let error = rename(
        Path::new(""),
        &temp.path().join("from.txt"),
        Path::new("to.txt"),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        SecureError::RelativeWithoutRoot(ref path) if path == Path::new("to.txt")
    ));
}

#[test]
fn secure_path_preserves_not_found_identity() {
    let (_temp, root) = workspace();
    let missing = root.join("missing.txt");

    assert!(
        open_workspace_file(&root, &missing)
            .unwrap_err()
            .is_not_found()
    );
    assert!(read_file(&root, &missing).unwrap_err().is_not_found());
    assert!(remove(&root, &missing).unwrap_err().is_not_found());
    assert!(chmod(&root, &missing, 0o600).unwrap_err().is_not_found());
    assert!(!path_exists(&root, &missing).unwrap());
}

#[test]
fn secure_path_create_new_refuses_existing_file() {
    let (_temp, root) = workspace();
    let path = root.join("existing.txt");
    write_new_file(&root, &path, b"original", 0o600).unwrap();

    let error = write_new_file(&root, &path, b"replacement", 0o600).unwrap_err();
    assert!(matches!(
        error,
        SecureError::Io {
            op: "create",
            source,
            ..
        } if source.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn secure_path_creates_nested_parent_directories() {
    let (_temp, root) = workspace();
    let relative = Path::new("one/two/three.txt");
    write_new_file(&root, relative, b"nested", 0o600).unwrap();

    assert_eq!(read_file(&root, relative).unwrap(), b"nested");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(root.join(relative))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn secure_path_round_trips_rename_chmod_exists_and_read() {
    let (_temp, root) = workspace();
    let source = root.join("source.txt");
    let destination = root.join("destination.txt");
    write_new_file(&root, &source, b"payload", 0o600).unwrap();

    assert!(path_exists(&root, &source).unwrap());
    chmod(&root, &source, 0o400).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&source).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o400);
    }
    #[cfg(windows)]
    assert!(fs::metadata(&source).unwrap().permissions().readonly());

    chmod(&root, &source, 0o600).unwrap();
    #[cfg(windows)]
    assert!(!fs::metadata(&source).unwrap().permissions().readonly());

    rename(&root, &source, &destination).unwrap();
    assert!(!path_exists(&root, &source).unwrap());
    assert!(path_exists(&root, &destination).unwrap());
    assert_eq!(read_file(&root, &destination).unwrap(), b"payload");

    remove(&root, &destination).unwrap();
    assert!(!path_exists(&root, &destination).unwrap());
}

#[cfg(unix)]
#[test]
fn secure_path_rename_creates_destination_directories() {
    let (_temp, root) = workspace();
    let source = root.join("source.txt");
    let destination = root.join("one/two/destination.txt");
    write_new_file(&root, &source, b"rename", 0o600).unwrap();

    rename(&root, &source, &destination).unwrap();
    assert_eq!(read_file(&root, &destination).unwrap(), b"rename");
    assert!(!path_exists(&root, &source).unwrap());
}

#[test]
fn secure_path_supports_absolute_targets_without_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.txt");
    let destination = temp.path().join("destination.txt");
    let unrooted = Path::new("");

    write_new_file(unrooted, &source, b"unrooted", 0o600).unwrap();
    assert_eq!(read_file(unrooted, &source).unwrap(), b"unrooted");
    rename(unrooted, &source, &destination).unwrap();
    assert!(path_exists(unrooted, &destination).unwrap());
    remove(unrooted, &destination).unwrap();
    assert!(!path_exists(unrooted, &destination).unwrap());

    let empty = temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    remove(unrooted, &empty).unwrap();
    assert!(!empty.exists());
}

#[cfg(unix)]
#[test]
fn secure_path_rejects_symlink_escapes() {
    use std::os::unix::fs::symlink;

    let (temp, root) = workspace();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.txt"), b"keep").unwrap();
    symlink(&outside, root.join("link")).unwrap();

    let results = [
        open_workspace_file(&root, Path::new("link/secret.txt")).map(|_| ()),
        write_new_file(&root, Path::new("link/new.txt"), b"no", 0o600),
        rename(&root, Path::new("source"), Path::new("link/new.txt")),
        remove(&root, Path::new("link/secret.txt")),
        chmod(&root, Path::new("link/secret.txt"), 0o600),
        path_exists(&root, Path::new("link/secret.txt")).map(|_| ()),
        read_file(&root, Path::new("link/secret.txt")).map(|_| ()),
    ];
    for result in results {
        assert!(matches!(
            result,
            Err(SecureError::Path(PathError::Symlink { .. }))
        ));
    }
    assert_eq!(fs::read(outside.join("secret.txt")).unwrap(), b"keep");
    assert!(!outside.join("new.txt").exists());
}

#[cfg(unix)]
#[test]
fn secure_path_rejects_leaf_symlinks_inside_workspace() {
    use std::os::unix::fs::symlink;

    let (_temp, root) = workspace();
    fs::write(root.join("target.txt"), b"inside").unwrap();
    symlink(Path::new("target.txt"), root.join("link.txt")).unwrap();

    assert!(matches!(
        read_file(&root, Path::new("link.txt")),
        Err(SecureError::Path(PathError::Symlink { .. }))
    ));
    assert!(matches!(
        write_new_file(&root, Path::new("link.txt"), b"no", 0o600),
        Err(SecureError::Path(PathError::Symlink { .. }))
    ));
}

#[cfg(unix)]
#[test]
fn secure_path_accepts_symlinked_workspace_root() {
    use std::os::unix::fs::symlink;

    let (temp, actual) = workspace();
    let alias = temp.path().join("workspace-alias");
    symlink(&actual, &alias).unwrap();

    write_new_file(&alias, Path::new("one/file.txt"), b"alias", 0o600).unwrap();
    assert_eq!(
        read_file(&alias, Path::new("one/file.txt")).unwrap(),
        b"alias"
    );
    assert!(path_exists(&alias, Path::new("one/file.txt")).unwrap());
    chmod(&alias, Path::new("one/file.txt"), 0o400).unwrap();
    rename(&alias, Path::new("one/file.txt"), Path::new("two/file.txt")).unwrap();
    assert_eq!(
        read_file(&actual, Path::new("two/file.txt")).unwrap(),
        b"alias"
    );
    remove(&alias, Path::new("two/file.txt")).unwrap();
    assert!(!actual.join("two/file.txt").exists());
}

#[cfg(unix)]
#[test]
fn secure_path_rejects_workspace_root_as_file_target() {
    let (_temp, root) = workspace();

    assert!(matches!(
        open_workspace_file(&root, &root),
        Err(SecureError::WorkspaceRoot)
    ));
    assert!(matches!(
        write_new_file(&root, &root, b"no", 0o600),
        Err(SecureError::WorkspaceRoot)
    ));
}

#[cfg(windows)]
#[test]
fn secure_path_windows_fallback_uses_validate_then_plain_semantics() {
    let (temp, root) = workspace();
    let source = root.join("source.txt");
    let destination = root.join("destination.txt");

    assert!(path_exists(&root, &root).unwrap());
    write_new_file(&root, &source, b"windows", 0o600).unwrap();
    assert_eq!(read_file(&root, &source).unwrap(), b"windows");

    let readonly = root.join("readonly.txt");
    write_new_file(&root, &readonly, b"readonly", 0o400).unwrap();
    assert!(fs::metadata(&readonly).unwrap().permissions().readonly());
    chmod(&root, &readonly, 0o600).unwrap();
    chmod(&root, &source, 0o400).unwrap();
    assert!(fs::metadata(&source).unwrap().permissions().readonly());
    chmod(&root, &source, 0o600).unwrap();
    rename(&root, &source, &destination).unwrap();
    assert!(path_exists(&root, &destination).unwrap());
    remove(&root, &destination).unwrap();
    assert!(!path_exists(&root, &destination).unwrap());

    let empty = root.join("empty");
    fs::create_dir(&empty).unwrap();
    remove(&root, &empty).unwrap();
    assert!(!empty.exists());

    assert_outside(read_file(&root, &temp.path().join("outside.txt")).unwrap_err());
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct JsonFixture {
    name: String,
    values: Vec<u32>,
}

#[test]
fn atomic_json_writes_two_space_pretty_json_and_creates_parents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("one/two/value.json");
    let value = JsonFixture {
        name: "example".into(),
        values: vec![1, 2],
    };

    write_json_atomic(&path, &value).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"name\": \"example\",\n  \"values\": [\n    1,\n    2\n  ]\n}"
    );
    assert_eq!(read_json_file::<JsonFixture>(&path).unwrap(), value);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}

#[test]
fn atomic_json_round_trips_and_replaces_without_temp_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("value.json");
    write_json_atomic(
        &path,
        &JsonFixture {
            name: "before".into(),
            values: vec![1],
        },
    )
    .unwrap();
    let expected = JsonFixture {
        name: "after".into(),
        values: vec![2, 3],
    };
    write_json_atomic(&path, &expected).unwrap();

    assert_eq!(read_json_file::<JsonFixture>(&path).unwrap(), expected);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn atomic_json_uses_go_html_escaping() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("escaped.json");
    write_json_atomic(
        &path,
        &JsonFixture {
            name: "<>&\u{2028}\u{2029}".into(),
            values: vec![],
        },
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"name\": \"\\u003c\\u003e\\u0026\\u2028\\u2029\",\n  \"values\": []\n}"
    );
}

#[test]
fn atomic_json_reports_typed_decode_failure() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid.json");
    fs::write(&path, b"not json").unwrap();

    let error = read_json_file::<JsonFixture>(&path).unwrap_err();
    assert!(matches!(
        error,
        SecureError::JsonDecode {
            path: ref error_path,
            source,
        } if error_path == &path && source.is_syntax()
    ));
}

#[test]
fn atomic_json_reports_typed_read_failure() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing.json");

    let error = read_json_file::<JsonFixture>(&path).unwrap_err();
    assert!(matches!(
        error,
        SecureError::JsonRead {
            path: ref error_path,
            ref source,
        } if error_path == &path && source.kind() == std::io::ErrorKind::NotFound
    ));
    assert!(error.is_not_found());
}

#[test]
fn atomic_json_reports_typed_encode_failure_before_creating_parents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing/value.json");
    let invalid = BTreeMap::from([((1_u32, 2_u32), 3_u32)]);

    let error = write_json_atomic(&path, &invalid).unwrap_err();
    assert!(matches!(error, SecureError::JsonEncode { .. }));
    assert!(!temp.path().join("missing").exists());
}

#[test]
fn atomic_json_reports_typed_parent_creation_failure() {
    let temp = tempfile::tempdir().unwrap();
    let blocker = temp.path().join("blocker");
    fs::write(&blocker, b"file").unwrap();

    let error = write_json_atomic(
        &blocker.join("value.json"),
        &JsonFixture {
            name: "blocked".into(),
            values: vec![],
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        SecureError::Io {
            op: "create JSON directory",
            ..
        }
    ));
}
