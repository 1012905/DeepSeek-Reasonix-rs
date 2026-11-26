use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

fn blob_fixture() -> (TempDir, BlobStore) {
    let temp = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(temp.path().join("blobs"));
    (temp, blobs)
}

#[test]
fn digest_matches_sha256_and_put_uses_two_level_fanout() {
    let (_temp, blobs) = blob_fixture();
    assert_eq!(
        digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let reference = blobs.put(b"abc").unwrap();
    assert_eq!(reference, digest(b"abc"));
    assert_eq!(
        blobs.path(&reference),
        blobs
            .dir()
            .join("ba/78/ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert_eq!(blobs.get(&reference).unwrap(), b"abc");
    assert!(blobs.has(&reference));
    assert_eq!(blobs.put(b"abc").unwrap(), reference);
    assert_eq!(blobs.size().unwrap(), 3);
}

#[test]
fn blob_read_rejects_invalid_reference_and_corruption_and_put_repairs_it() {
    let (_temp, blobs) = blob_fixture();
    assert!(matches!(
        blobs.get("ABC"),
        Err(BlobError::InvalidReference(_))
    ));
    assert!(matches!(
        blobs.get("short"),
        Err(BlobError::InvalidReference(_))
    ));
    let reference = blobs.put(b"before").unwrap();
    fs::write(blobs.path(&reference), b"corrupt").unwrap();
    assert!(matches!(
        blobs.get(&reference),
        Err(BlobError::DigestMismatch { .. })
    ));
    assert!(!blobs.has(&reference));
    assert_eq!(blobs.put(b"before").unwrap(), reference);
    assert_eq!(blobs.get(&reference).unwrap(), b"before");
}

#[test]
fn remove_ignores_missing_and_invalid_references() {
    let (_temp, blobs) = blob_fixture();
    let reference = blobs.put(b"remove me").unwrap();
    blobs.remove(&reference).unwrap();
    blobs.remove(&reference).unwrap();
    blobs.remove("not-a-reference").unwrap();
    assert!(!blobs.has(&reference));
}

#[test]
fn prune_removes_only_dead_valid_blob_files() {
    let (_temp, blobs) = blob_fixture();
    let live = blobs.put(b"live").unwrap();
    let dead = blobs.put(b"dead").unwrap();
    let invalid = "NOT-A-SHA256-REFERENCE";
    fs::write(blobs.dir().join(invalid), b"keep invalid").unwrap();
    let directory_ref = "a".repeat(64);
    fs::create_dir_all(blobs.dir().join(&directory_ref)).unwrap();
    fs::write(blobs.dir().join(&directory_ref).join("nested"), b"keep").unwrap();

    blobs.prune(&HashSet::from([live.clone()])).unwrap();
    assert!(blobs.has(&live));
    assert!(!blobs.has(&dead));
    assert!(blobs.dir().join(invalid).exists());
    assert!(blobs.dir().join(directory_ref).join("nested").exists());
    assert_eq!(
        blobs.size().unwrap(),
        (b"live".len() + b"keep invalid".len() + b"keep".len()) as u64
    );
}

#[test]
fn empty_blob_store_is_unavailable_and_missing_directory_size_is_zero() {
    let blobs = BlobStore::new(PathBuf::new());
    assert!(matches!(blobs.put(b"x"), Err(BlobError::Unavailable)));
    assert!(matches!(
        blobs.get(&digest(b"x")),
        Err(BlobError::Unavailable)
    ));
    assert!(!blobs.has(&digest(b"x")));
    blobs.remove("invalid").unwrap();
    blobs.prune(&HashSet::new()).unwrap();
    assert_eq!(blobs.size().unwrap(), 0);

    let temp = tempfile::tempdir().unwrap();
    let missing = BlobStore::new(temp.path().join("missing"));
    assert_eq!(missing.size().unwrap(), 0);
    missing.prune(&HashSet::new()).unwrap();
}

#[test]
fn safe_path_confines_relative_and_absolute_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        safe_path(&root, Path::new("a/../b.txt")).unwrap(),
        root.join("b.txt")
    );
    assert_eq!(
        safe_path(&root, &root.join("b.txt")).unwrap(),
        root.join("b.txt")
    );
    assert!(matches!(
        safe_path(&root, Path::new("../outside.txt")),
        Err(PathError::OutsideWorkspace { .. })
    ));
    assert_eq!(
        normalize_rel_path(&root, &root.join("a/./b.txt")),
        "a/b.txt"
    );
}

#[test]
fn capture_path_records_missing_empty_and_oversized_states() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        max_bytes: 3,
    };
    let missing = capture_path(&root.join("missing"), &opts).unwrap();
    assert!(!missing.existed);
    assert!(missing.content.is_none());

    fs::write(root.join("empty"), b"").unwrap();
    let empty = capture_path(&root.join("empty"), &opts).unwrap();
    assert!(empty.existed);
    assert_eq!(empty.content, Some(Vec::new()));
    assert_eq!(empty.sha256, digest(b""));
    assert_eq!(empty.encoding, Kind::Utf8);

    fs::write(root.join("large"), b"four").unwrap();
    assert!(matches!(
        capture_path(&root.join("large"), &opts),
        Err(CaptureError::Oversized {
            size: 4,
            max_bytes: 3,
            ..
        })
    ));
}

#[test]
fn capture_path_detects_encoding_and_decodes_inline_text() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    fs::write(root.join("utf16"), [0xff, 0xfe, b'h', 0, b'i', 0]).unwrap();
    let fp = capture_path(
        &root.join("utf16"),
        &CaptureOptions {
            workspace_root: root,
            read_content: true,
            ..CaptureOptions::default()
        },
    )
    .unwrap();
    assert_eq!(fp.encoding, Kind::Utf16Le);
    assert_eq!(fp.sha256, digest(&[0xff, 0xfe, b'h', 0, b'i', 0]));
    let (encoding, decoded) = fileencoding::detect_and_decode(fp.content.as_deref().unwrap());
    assert_eq!(encoding, Kind::Utf16Le);
    assert_eq!(decoded.as_ref(), b"hi");
}

#[test]
fn capture_path_rejects_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir(root.join("dir")).unwrap();
    assert!(matches!(
        capture_path(
            &root.join("dir"),
            &CaptureOptions {
                workspace_root: root,
                read_content: true,
                ..CaptureOptions::default()
            }
        ),
        Err(CaptureError::Directory { .. })
    ));
}

#[cfg(unix)]
#[test]
fn safe_path_rejects_symlink_components() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join("link")).unwrap();
    assert!(matches!(
        safe_path(&root, Path::new("link/file.txt")),
        Err(PathError::Symlink { .. })
    ));
    assert_eq!(
        capture_path(
            Path::new("link/file.txt"),
            &CaptureOptions {
                workspace_root: root,
                read_content: true,
                ..CaptureOptions::default()
            }
        )
        .unwrap_err()
        .gap()
        .reason,
        GAP_SYMLINK
    );
}

#[test]
fn compare_identity_distinguishes_manual_mode_and_existence_changes() {
    let current = Fingerprint {
        existed: true,
        mode: 0o644,
        sha256: digest(b"current"),
        ..Fingerprint::default()
    };
    assert_eq!(
        compare_identity(&current, "", None, 0),
        Some(CONFLICT_COVERAGE_LEGACY)
    );
    assert_eq!(
        compare_identity(&current, &digest(b"current"), Some(true), 0o644),
        None
    );
    assert_eq!(
        compare_identity(&current, &digest(b"owned"), Some(true), 0o600),
        Some(CONFLICT_MODE_CHANGE)
    );
    assert_eq!(
        compare_identity(&current, &digest(b"other"), Some(true), 0o644),
        Some(CONFLICT_MANUAL_EDIT)
    );
    assert_eq!(
        compare_identity(&current, "", Some(false), 0),
        Some(CONFLICT_DELETED_RECREATE)
    );
}

#[test]
fn safe_path_rejects_a_sibling_sharing_the_workspace_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let sibling = temp.path().join("workspace-other");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    fs::write(sibling.join("secret.txt"), "keep").unwrap();

    assert!(matches!(
        safe_path(&root, &sibling.join("secret.txt")),
        Err(PathError::OutsideWorkspace { .. })
    ));
    let error = capture_path(
        &sibling.join("secret.txt"),
        &CaptureOptions {
            workspace_root: root,
            read_content: true,
            ..CaptureOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.gap().reason, GAP_OUTSIDE_WORKSPACE);
}

#[cfg(unix)]
#[test]
fn normalize_rel_path_preserves_literal_backslashes_on_unix() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert_eq!(normalize_rel_path(root, Path::new("a\\b.txt")), "a\\b.txt");
    assert_ne!(
        normalize_rel_path(root, Path::new("a\\b.txt")),
        normalize_rel_path(root, Path::new("a/b.txt"))
    );
}

#[cfg(unix)]
#[test]
fn safe_path_rejects_symlink_components_without_a_workspace_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    let link = temp.path().join("link");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, &link).unwrap();
    assert!(matches!(
        safe_path(Path::new(""), &link.join("new.txt")),
        Err(PathError::Symlink { .. })
    ));
}

#[cfg(unix)]
#[test]
fn capture_path_classifies_unix_directories_as_hardlinks() {
    use std::os::unix::fs::MetadataExt;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let directory = root.join("dir");
    fs::create_dir(&directory).unwrap();
    let nlink = fs::metadata(&directory).unwrap().nlink();
    assert!(nlink > 1);
    let error = fingerprint_path(&root, &directory).unwrap_err();
    match &error {
        CaptureError::Hardlink { nlink: actual, .. } => assert_eq!(*actual, nlink),
        other => panic!("expected hardlink error, got {other:?}"),
    }
    assert_eq!(error.gap().reason, GAP_HARDLINK);
}

#[cfg(unix)]
#[test]
fn capture_path_rejects_symlinks_even_when_targets_stay_in_workspace() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    fs::write(root.join("target"), b"inside").unwrap();
    symlink(Path::new("target"), root.join("link")).unwrap();
    assert!(matches!(
        fingerprint_path(&root, &root.join("link")),
        Err(CaptureError::Path(PathError::Symlink { .. }))
    ));
}

#[cfg(unix)]
#[test]
fn capture_path_rejects_absolute_symlinks_without_a_workspace_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("link");
    fs::write(&target, b"outside").unwrap();
    symlink(&target, &link).unwrap();
    assert!(matches!(
        capture_path(
            &link,
            &CaptureOptions {
                read_content: true,
                ..CaptureOptions::default()
            }
        ),
        Err(CaptureError::Path(PathError::Symlink { .. }))
    ));
}

#[cfg(windows)]
#[test]
fn capture_path_rejects_windows_junctions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir(&target).unwrap();
    let link = root
        .canonicalize()
        .unwrap()
        .join(link.file_name().expect("link name"));
    let status = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(target.canonicalize().unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(matches!(
        fingerprint_path(&root, &link),
        Err(CaptureError::Path(PathError::Symlink { .. }))
    ));
}
