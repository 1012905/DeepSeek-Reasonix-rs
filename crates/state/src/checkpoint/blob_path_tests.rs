use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

#[cfg(windows)]
use super::path_clean::lexical_clean;
use super::*;

fn blob_fixture() -> (TempDir, BlobStore) {
    let temp = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(temp.path().join("blobs"));
    (temp, blobs)
}

fn assert_original_gap(path: &Path, opts: &CaptureOptions, reason: &str) {
    let error = capture_path(path, opts).unwrap_err();
    let CaptureError::AtPath {
        path: original,
        source: _,
    } = &error
    else {
        panic!("expected original-path wrapper, got {error:?}");
    };
    assert_eq!(original.as_path(), path, "path {path:?}");
    let gap = error.gap();
    assert_eq!(gap.path, path.to_string_lossy(), "path {path:?}");
    assert_eq!(gap.reason, reason, "path {path:?}");
    assert_eq!(
        error.is_outside_workspace(),
        reason == GAP_OUTSIDE_WORKSPACE
    );
}

fn assert_at_path(
    error: &CaptureError,
    path: &Path,
    predicate: impl FnOnce(&CaptureError) -> bool,
) {
    let CaptureError::AtPath {
        path: original,
        source,
    } = error
    else {
        panic!("expected original-path wrapper, got {error:?}");
    };
    assert_eq!(original.as_path(), path);
    assert!(predicate(source), "{source:?}");
}

fn is_symlink_at_path(error: &CaptureError) -> bool {
    matches!(
        error,
        CaptureError::AtPath { source, .. }
            if matches!(
                source.as_ref(),
                CaptureError::Path(PathError::Symlink { .. })
            )
    )
}

fn assert_original_path_forms(
    relative: &Path,
    absolute: &Path,
    opts: &CaptureOptions,
    reason: &str,
) {
    assert_original_gap(relative, opts, reason);
    assert_original_gap(absolute, opts, reason);
}

#[test]
fn capture_outside_workspace_gap_preserves_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let outside = temp.path().join("outside.txt");
    fs::create_dir(&root).unwrap();
    fs::write(&outside, b"outside").unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    assert_original_path_forms(
        Path::new("../outside.txt"),
        &outside,
        &opts,
        GAP_OUTSIDE_WORKSPACE,
    );
}

#[test]
fn capture_relative_without_root_gap_preserves_original_path() {
    let error = capture_path(
        Path::new("relative.txt"),
        &CaptureOptions {
            read_content: true,
            ..CaptureOptions::default()
        },
    )
    .unwrap_err();
    assert_at_path(&error, Path::new("relative.txt"), |source| {
        matches!(source, CaptureError::RelativeWithoutRoot(_))
    });
    let gap = error.gap();
    assert_eq!(gap.path, "relative.txt");
    assert_eq!(gap.reason, GAP_UNREADABLE);
}

#[test]
fn capture_error_gap_at_overrides_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let error = capture_path(
        Path::new("../outside.txt"),
        &CaptureOptions {
            workspace_root: root,
            read_content: true,
            ..CaptureOptions::default()
        },
    )
    .unwrap_err();
    let gap = error.gap_at(Path::new("caller/override.txt"));
    assert_eq!(gap.path, "caller/override.txt");
    assert_eq!(gap.reason, GAP_OUTSIDE_WORKSPACE);
}

#[test]
fn capture_oversized_gap_preserves_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let absolute = root.join("large.txt");
    fs::write(&absolute, b"four").unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        max_bytes: 3,
    };
    assert_original_path_forms(Path::new("large.txt"), &absolute, &opts, GAP_OVERSIZED);
}

#[test]
fn capture_directory_gap_preserves_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let absolute = root.join("dir");
    fs::create_dir(&absolute).unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    #[cfg(unix)]
    let reason = {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(&absolute).unwrap().nlink() > 1 {
            GAP_HARDLINK
        } else {
            GAP_CAPTURE_FAILED
        }
    };
    #[cfg(not(unix))]
    let reason = GAP_CAPTURE_FAILED;
    assert_original_path_forms(Path::new("dir"), &absolute, &opts, reason);
}

#[test]
fn capture_unreadable_gap_preserves_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let absolute = root.join("plain/child.txt");
    fs::write(root.join("plain"), b"not a directory").unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    assert_original_path_forms(
        Path::new("plain/child.txt"),
        &absolute,
        &opts,
        GAP_UNREADABLE,
    );
}

#[cfg(any(unix, windows))]
#[test]
fn capture_hardlink_gap_preserves_original_path_and_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let original = root.join("original.txt");
    let absolute = root.join("link.txt");
    fs::write(&original, b"hardlink").unwrap();
    fs::hard_link(&original, &absolute).unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    assert_original_path_forms(Path::new("link.txt"), &absolute, &opts, GAP_HARDLINK);
    let error = capture_path(Path::new("link.txt"), &opts).unwrap_err();
    assert_at_path(&error, Path::new("link.txt"), |source| {
        matches!(source, CaptureError::Hardlink { nlink: 2, .. })
    });
}

#[test]
fn capture_path_leaves_ordinary_file_nlink_zero() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let path = root.join("ordinary.txt");
    fs::write(&path, b"ordinary").unwrap();
    for read_content in [false, true] {
        let fingerprint = capture_path(
            &path,
            &CaptureOptions {
                workspace_root: root.clone(),
                read_content,
                ..CaptureOptions::default()
            },
        )
        .unwrap();
        assert!(fingerprint.existed);
        assert_eq!(fingerprint.nlink, 0);
    }
    let missing = capture_path(
        &root.join("missing.txt"),
        &CaptureOptions {
            workspace_root: root,
            ..CaptureOptions::default()
        },
    )
    .unwrap();
    assert_eq!(missing.nlink, 0);
}

#[cfg(windows)]
#[test]
fn windows_path_comparison_uses_go_simple_fold() {
    let temp = tempfile::tempdir().unwrap();
    for (index, (left, right)) in [
        ("K", "k"),
        ("K", "\u{212a}"),
        ("S", "s"),
        ("S", "\u{17f}"),
        ("Σ", "σ"),
        ("Σ", "ς"),
        ("ß", "ẞ"),
    ]
    .into_iter()
    .enumerate()
    {
        let root = temp.path().join(format!("root-{index}-{left}"));
        fs::create_dir(&root).unwrap();
        let target = temp.path().join(format!("root-{index}-{right}/file.txt"));
        assert_eq!(
            safe_path(&root, &target).unwrap(),
            target,
            "{left:?} {right:?}"
        );
    }
    for (index, (left, right)) in [
        ("ß", "ss"),
        ("İ", "i"),
        ("İ", "ı"),
        ("ﬁ", "fi"),
        ("é", "e\u{301}"),
    ]
    .into_iter()
    .enumerate()
    {
        let negative_root = temp.path().join(format!("negative-{index}-{left}"));
        fs::create_dir(&negative_root).unwrap();
        let target = temp
            .path()
            .join(format!("negative-{index}-{right}/file.txt"));
        assert!(matches!(
            safe_path(&negative_root, &target),
            Err(PathError::OutsideWorkspace { .. })
        ));
    }
}

#[cfg(windows)]
#[test]
fn windows_path_comparison_fails_closed_for_distinct_invalid_utf16() {
    use std::os::windows::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let left = std::ffi::OsString::from_wide(&[0xd800]);
    let right = std::ffi::OsString::from_wide(&[0xd801]);
    let root = temp.path().join(left);
    let target = temp.path().join(right).join("file.txt");
    assert!(matches!(
        safe_path(&root, &target),
        Err(PathError::OutsideWorkspace { .. })
    ));
}

#[cfg(not(windows))]
#[test]
fn path_comparison_remains_exact_on_non_windows() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("K");
    fs::create_dir(&root).unwrap();
    let target = temp.path().join("k/file.txt");
    assert!(matches!(
        safe_path(&root, &target),
        Err(PathError::OutsideWorkspace { .. })
    ));
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

#[cfg(windows)]
#[test]
fn safe_path_rejects_windows_non_local_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    for path in [
        "a:b",
        "dir/a:b.txt",
        "dir/CON",
        "dir/con ",
        "dir/COM1.log",
        "dir/LPT\u{b9}.txt",
        "dir/CONIN$.cfg",
        "dir/CONOUT$",
        "dir/PRN...",
        "dir/NUL ",
        "dir/CON.txt.",
    ] {
        let result = safe_path(&root, Path::new(path));
        assert!(
            matches!(result, Err(PathError::OutsideWorkspace { .. })),
            "{path:?}: {result:?}"
        );
    }
    for path in ["normal.txt", "COM10", "LPT0", "CONSOLE", "dir/file"] {
        assert_eq!(safe_path(&root, Path::new(path)).unwrap(), root.join(path));
    }
    assert_eq!(safe_path(&root, Path::new(".")).unwrap(), root);
}

#[cfg(windows)]
#[test]
fn safe_path_conservatively_rejects_extension_bearing_device_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("CON.txt"), b"ordinary on Windows 11").unwrap();
    assert!(matches!(
        safe_path(&root, Path::new("CON.txt")),
        Err(PathError::OutsideWorkspace { .. })
    ));
}

#[cfg(windows)]
#[test]
fn normalize_rel_path_falls_back_for_windows_non_local_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir_all(&root).unwrap();
    for path in ["CON.txt", "dir/a:b.txt"] {
        let absolute = root.join(path);
        let expected = absolute.to_string_lossy().replace('\\', "/");
        assert_eq!(normalize_rel_path(&root, &absolute), expected);
    }
    assert_eq!(
        normalize_rel_path(&root, Path::new("normal.txt")),
        "normal.txt"
    );
}

#[cfg(windows)]
#[test]
fn rootless_capture_retains_go_path_semantics() {
    let path = Path::new(r"C:\missing\a:b.txt");
    assert_eq!(safe_path(Path::new(""), path).unwrap(), lexical_clean(path));
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
    let large = root.join("large");
    let error = capture_path(&large, &opts).unwrap_err();
    assert_at_path(&error, &large, |source| {
        matches!(
            source,
            CaptureError::Oversized {
                size: 4,
                max_bytes: 3,
                ..
            }
        )
    });
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
    let directory = root.join("dir");
    fs::create_dir(&directory).unwrap();
    let error = capture_path(
        &directory,
        &CaptureOptions {
            workspace_root: root,
            read_content: true,
            ..CaptureOptions::default()
        },
    )
    .unwrap_err();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let nlink = fs::metadata(&directory).unwrap().nlink();
        if nlink > 1 {
            assert_at_path(
                &error,
                &directory,
                |source| matches!(source, CaptureError::Hardlink { nlink: actual, .. } if *actual == nlink),
            );
        } else {
            assert_at_path(&error, &directory, |source| {
                matches!(source, CaptureError::Directory { .. })
            });
        }
    }
    #[cfg(not(unix))]
    assert_at_path(&error, &directory, |source| {
        matches!(source, CaptureError::Directory { .. })
    });
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
    assert_at_path(
        &error,
        &directory,
        |source| matches!(source, CaptureError::Hardlink { nlink: actual, .. } if *actual == nlink),
    );
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
        Err(error) if is_symlink_at_path(&error)
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
        Err(error) if is_symlink_at_path(&error)
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
        Err(error) if is_symlink_at_path(&error)
    ));
}

#[cfg(unix)]
#[test]
fn capture_symlink_gap_preserves_original_path() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let target = temp.path().join("target");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file.txt"), b"outside").unwrap();
    symlink(&target, root.join("link")).unwrap();
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    assert_original_path_forms(
        Path::new("link/file.txt"),
        &root.join("link/file.txt"),
        &opts,
        GAP_SYMLINK,
    );
}

#[cfg(windows)]
#[test]
fn capture_windows_junction_gap_preserves_original_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let target = root.join("target");
    let link = root.join("link");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file.txt"), b"inside").unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success());
    let opts = CaptureOptions {
        workspace_root: root.clone(),
        read_content: true,
        ..CaptureOptions::default()
    };
    assert_original_path_forms(
        Path::new("link/file.txt"),
        &root.join("link/file.txt"),
        &opts,
        GAP_SYMLINK,
    );
}
