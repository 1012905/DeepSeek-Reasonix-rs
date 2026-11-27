use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::*;

#[test]
fn freeze_refs_requires_a_nonblank_workspace() {
    for workspace in ["", " \t\r\n "] {
        assert!(matches!(
            freeze_refs(workspace, &[]),
            Err(RefError::MissingWorkspace)
        ));
    }
}

#[test]
fn freeze_refs_reports_workspace_open_failure() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    assert!(matches!(
        freeze_refs(missing.to_str().unwrap(), &[]),
        Err(RefError::OpenRoot { .. })
    ));
}

#[test]
fn freeze_refs_trims_paths_and_deduplicates_only_exact_spellings() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("one"), b"one").unwrap();
    fs::write(temp.path().join("two"), b"two").unwrap();
    let refs = freeze_refs(
        &format!("  {}  ", temp.path().display()),
        &[
            " one ".to_owned(),
            "one".to_owned(),
            "./one".to_owned(),
            " \t ".to_owned(),
            "two".to_owned(),
        ],
    )
    .unwrap();
    assert_eq!(
        refs.iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>(),
        ["one", "one", "two"]
    );
    assert_eq!(refs[0].content, b"one");
    assert_eq!(refs[1].content, b"one");
    assert_eq!(refs[2].content, b"two");
}

#[test]
fn freeze_refs_projects_absolute_paths_to_workspace_relative_slashes() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested/file.txt"), b"body").unwrap();
    let absolute = temp.path().join("nested/./file.txt");
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &[absolute.to_string_lossy().into_owned()],
    )
    .unwrap();
    assert_eq!(refs[0].path, "nested/file.txt");
    assert_eq!(refs[0].display_path, "nested/file.txt");
    assert_eq!(refs[0].content, b"body");
}

#[test]
fn freeze_refs_keeps_escape_failures_as_diagnostic_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().parent().unwrap().join("outside-ref.txt");
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &[
            "../outside-ref.txt".to_owned(),
            outside.to_string_lossy().into_owned(),
        ],
    )
    .unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].path, "../outside-ref.txt");
    assert_eq!(refs[0].display_path, "../outside-ref.txt");
    assert!(refs[0].content.starts_with(b"/* ref freeze failed: "));
    assert!(String::from_utf8_lossy(&refs[0].content).contains("path outside workspace"));
    assert!(refs[0].content_sha.is_empty());
    assert!(!refs[0].truncated);
}

#[test]
fn freeze_refs_snapshots_file_bytes_checksum_and_identity_fields() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("body.bin"), b"a\0b\xff").unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["body.bin".to_owned()]).unwrap();
    assert_eq!(refs[0].kind, "frozen");
    assert_eq!(refs[0].content, b"a\0b\xff");
    assert_eq!(
        refs[0].content_sha,
        sha256_hex(b"a\0b\xff"),
        "content hash must cover the frozen bytes"
    );
    assert!(!refs[0].truncated);
}

#[test]
fn freeze_refs_truncates_files_only_above_four_mib() {
    let temp = tempfile::tempdir().unwrap();
    let max = usize::try_from(DEFAULT_MAX_ITEM_BYTES).unwrap();
    fs::write(temp.path().join("exact"), vec![b'x'; max]).unwrap();
    fs::write(temp.path().join("over"), vec![b'y'; max + 17]).unwrap();
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &["exact".to_owned(), "over".to_owned()],
    )
    .unwrap();
    assert_eq!(refs[0].content.len(), max);
    assert!(!refs[0].truncated);
    assert_eq!(refs[1].content, vec![b'y'; max]);
    assert!(refs[1].truncated);
    assert_eq!(refs[1].content_sha, sha256_hex(&refs[1].content));
}

#[cfg(unix)]
#[test]
fn freeze_refs_refuses_symlink_targets_and_nested_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    symlink(outside.path(), temp.path().join("link")).unwrap();
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &["link".to_owned(), "link/secret".to_owned()],
    )
    .unwrap();
    for item in refs {
        let diagnostic = String::from_utf8_lossy(&item.content);
        assert!(
            diagnostic.starts_with("/* ref freeze failed: "),
            "{diagnostic}"
        );
        assert!(!diagnostic.contains("secret\n```"), "target bytes leaked");
    }
}

#[cfg(windows)]
#[test]
fn freeze_refs_refuses_junction_targets_and_nested_junction_escape() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    create_junction(&temp.path().join("link"), outside.path());
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &["link".to_owned(), "link/secret".to_owned()],
    )
    .unwrap();
    for item in refs {
        let diagnostic = String::from_utf8_lossy(&item.content);
        assert!(
            diagnostic.starts_with("/* ref freeze failed: "),
            "{diagnostic}"
        );
        assert!(!diagnostic.contains("secret\n```"), "target bytes leaked");
    }
}

#[cfg(unix)]
#[test]
fn freeze_refs_allows_a_symlinked_configured_workspace_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("body"), b"inside").unwrap();
    let alias = temp.path().join("alias");
    symlink(&workspace, &alias).unwrap();
    let refs = freeze_refs(alias.to_str().unwrap(), &["body".to_owned()]).unwrap();
    assert_eq!(refs[0].content, b"inside");
}

#[test]
fn freeze_refs_lists_directories_deterministically_and_skips_noise_and_links() {
    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    fs::create_dir_all(tree.join("nested/deeper")).unwrap();
    fs::create_dir_all(tree.join("build")).unwrap();
    fs::write(tree.join("z.txt"), b"z").unwrap();
    fs::write(tree.join("a.txt"), b"a").unwrap();
    fs::write(tree.join(".DS_Store"), b"noise").unwrap();
    fs::write(tree.join("Thumbs.db"), b"noise").unwrap();
    fs::write(tree.join("build/kept.txt"), b"kept").unwrap();
    fs::write(tree.join("nested/deeper/child.txt"), b"child").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    create_directory_link(&tree.join("linked"), outside.path());
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["tree".to_owned()]).unwrap();
    assert_eq!(
        refs[0].content,
        b"a.txt\nnested/\nnested/deeper/\nnested/deeper/child.txt\nz.txt"
    );
    assert!(!refs[0].truncated);
    assert_eq!(refs[0].content_sha, sha256_hex(&refs[0].content));
}

#[test]
fn freeze_refs_caps_directory_entries_at_one_hundred_with_legacy_marker() {
    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for index in 0..101 {
        fs::write(tree.join(format!("f{index:03}")), b"").unwrap();
    }
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["tree".to_owned()]).unwrap();
    let mut expected = (0..100)
        .map(|index| format!("f{index:03}"))
        .collect::<Vec<_>>();
    expected.push("…[truncated; directory has more entries]…".to_owned());
    assert_eq!(refs[0].content, expected.join("\n").into_bytes());
    assert!(!refs[0].truncated);
}

#[test]
fn freeze_refs_caps_the_enumerated_prefix_before_sorting() {
    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for index in (0..101).rev() {
        fs::write(tree.join(format!("f{index:03}")), b"").unwrap();
    }
    let dir = cap_std::fs::Dir::open_ambient_dir(&tree, cap_std::ambient_authority()).unwrap();
    let mut enumerated = dir
        .read_dir(".")
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    enumerated.truncate(MAX_FROZEN_DIR_ENTRIES);
    enumerated.sort();
    enumerated.push("…[truncated; directory has more entries]…".to_owned());

    let refs = freeze_refs(temp.path().to_str().unwrap(), &["tree".to_owned()]).unwrap();
    assert_eq!(refs[0].content, enumerated.join("\n").into_bytes());
}

#[test]
fn freeze_refs_preserves_empty_directory_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("empty")).unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["empty".to_owned()]).unwrap();
    assert!(refs[0].content.is_empty());
    assert_eq!(refs[0].content_sha, sha256_hex(b""));
    assert!(!refs[0].truncated);
}

#[test]
fn freeze_refs_keeps_missing_paths_as_diagnostic_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["missing".to_owned()]).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, "missing");
    assert!(String::from_utf8_lossy(&refs[0].content).starts_with("/* ref freeze failed: "));
}

#[test]
fn freeze_refs_does_not_add_a_marker_for_exactly_one_hundred_entries() {
    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for index in 0..100 {
        fs::write(tree.join(format!("f{index:03}")), b"").unwrap();
    }
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["tree".to_owned()]).unwrap();
    let content = String::from_utf8(refs[0].content.clone()).unwrap();
    assert_eq!(content.lines().count(), 100);
    assert!(!content.contains("truncated; directory has more entries"));
}

#[test]
fn freeze_refs_supports_the_workspace_root_itself() {
    let temp = tempfile::tempdir().unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &[".".to_owned()]).unwrap();
    assert_eq!(refs[0].path, ".");
    assert!(refs[0].content.is_empty());
    assert_eq!(refs[0].content_sha, sha256_hex(b""));
}

#[cfg(windows)]
#[test]
fn freeze_refs_rejects_windows_reserved_local_components() {
    let temp = tempfile::tempdir().unwrap();
    let refs = freeze_refs(
        temp.path().to_str().unwrap(),
        &["CON".to_owned(), "nested/AUX.txt".to_owned()],
    )
    .unwrap();
    for item in refs {
        assert!(String::from_utf8_lossy(&item.content).contains("path outside workspace"));
    }
}

#[cfg(unix)]
#[test]
fn freeze_refs_preserves_literal_backslashes_on_unix() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a\\b"), b"literal").unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["a\\b".to_owned()]).unwrap();
    assert_eq!(refs[0].path, "a\\b");
    assert_eq!(refs[0].content, b"literal");
}

#[cfg(unix)]
#[test]
fn freeze_refs_fails_closed_on_non_utf8_directory_entry_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join(OsString::from_vec(vec![0xff])), b"secret").unwrap();
    let refs = freeze_refs(temp.path().to_str().unwrap(), &["tree".to_owned()]).unwrap();
    let diagnostic = String::from_utf8_lossy(&refs[0].content);
    assert!(diagnostic.starts_with("/* ref freeze failed: "));
    assert!(diagnostic.contains("not valid UTF-8"));
    assert!(!diagnostic.contains('\u{fffd}'));
}

#[test]
fn file_snapshot_comparison_detects_size_and_mtime_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("body");
    let dir =
        cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
    fs::write(&path, b"abc").unwrap();
    let before = dir.open("body").unwrap().metadata().unwrap();
    assert!(super::refs::same_file_state(&before, &before));

    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(4)
        .unwrap();
    let resized = dir.open("body").unwrap().metadata().unwrap();
    assert!(!super::refs::same_file_state(&before, &resized));

    let modified = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let retimed = dir.open("body").unwrap().metadata().unwrap();
    assert!(!super::refs::same_file_state(&resized, &retimed));
}

#[test]
fn apply_frozen_refs_renders_the_exact_sorted_legacy_block() {
    let bodies = BTreeMap::from([
        ("z/file".to_owned(), b"last".to_vec()),
        ("a/file".to_owned(), b"first".to_vec()),
    ]);
    assert_eq!(apply_frozen_refs("submit", &bodies), b"submit\n\n<!-- frozen inbox references (enqueue-time snapshot) -->\n\n### @a/file\n```\nfirst\n```\n\n### @z/file\n```\nlast\n```\n");
    assert_eq!(apply_frozen_refs("submit", &BTreeMap::new()), b"submit");
}

#[test]
fn materialize_refs_checks_stored_bytes_without_live_filesystem_access() {
    let body = b"stored body";
    let refs = vec![RefSnapshot {
        kind: "frozen".to_owned(),
        path: "missing/live/path".to_owned(),
        display_path: " alias ".to_owned(),
        content: body.to_vec(),
        content_sha: sha256_hex(body),
        ..RefSnapshot::default()
    }];
    let (note, bodies) = materialize_refs(&refs);
    assert!(note.is_empty());
    assert_eq!(bodies.get("alias"), Some(&body.to_vec()));
}

#[test]
fn materialize_refs_stops_on_clean_git_or_checksum_mismatch_and_keeps_prior_bodies() {
    let first = RefSnapshot {
        path: "first".to_owned(),
        content: b"first".to_vec(),
        ..RefSnapshot::default()
    };
    let clean = RefSnapshot {
        kind: "clean_git".to_owned(),
        path: "clean".to_owned(),
        ..RefSnapshot::default()
    };
    let (note, bodies) = materialize_refs(&[first.clone(), clean]);
    assert_eq!(note, "legacy clean-git reference clean requires refresh");
    assert_eq!(bodies.get("first"), Some(&b"first".to_vec()));

    let corrupt = RefSnapshot {
        path: "corrupt".to_owned(),
        content: b"changed".to_vec(),
        content_sha: sha256_hex(b"original"),
        ..RefSnapshot::default()
    };
    let (note, bodies) = materialize_refs(&[first, corrupt]);
    assert_eq!(note, "checksum mismatch for frozen ref corrupt");
    assert_eq!(bodies.get("first"), Some(&b"first".to_vec()));
    assert!(!bodies.contains_key("corrupt"));
}

#[test]
fn materialize_refs_preserves_non_utf8_body_bytes() {
    let refs = vec![RefSnapshot {
        path: "binary".to_owned(),
        content: vec![0xff, 0x00, 0xfe],
        ..RefSnapshot::default()
    }];
    let (note, mut bodies) = materialize_refs(&refs);
    assert!(note.is_empty());
    assert_eq!(bodies.get("binary"), Some(&vec![0xff, 0x00, 0xfe]));
    let mut input = BTreeMap::new();
    input.insert("binary".to_owned(), bodies.remove("binary").unwrap());
    assert_eq!(apply_frozen_refs("submit", &input), b"submit\n\n<!-- frozen inbox references (enqueue-time snapshot) -->\n\n### @binary\n```\n\xff\0\xfe\n```\n");
}

#[cfg(windows)]
fn create_junction(link: &Path, target: &Path) {
    let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned());
    let output = std::process::Command::new(comspec)
        .args(["/D", "/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J failed: {output:?}");
}

fn create_directory_link(link: &Path, target: &Path) {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
    }
    #[cfg(windows)]
    {
        create_junction(link, target);
    }
}
