use std::fs;
use std::io::Cursor;

use serde_json::json;
use tempfile::TempDir;

use super::content::ContentPool;
use super::frames::read_frame;
use super::session::{ListingCache, Manifest, Transcript, list, open, open_in};
use super::{Damage, Limit, Unsupported, V4Error};
use crate::sessionstore::sessionpreview::session_preview_from_messages;
use crate::store::naming::v4_sessions_root;

fn manifest(id: &str) -> Manifest {
    Manifest {
        schema_version: 4,
        codec: super::CODEC.to_string(),
        storage_revision: 3,
        content_root: "../.content-v1".to_string(),
        session_id: id.to_string(),
        created_at: "2026-09-25T12:00:00Z".parse().expect("time"),
    }
}

#[test]
fn manifest_equality_compares_every_field_and_the_instant() {
    let original = manifest(crate::sessionstore::sessionv4::test_support::ID);
    let mut other = original.clone();
    other.created_at = "2026-09-25T14:00:00+02:00".parse().expect("time");
    assert!(original.equal(&other));
    other.storage_revision = 2;
    assert!(!original.equal(&other));
    other = original.clone();
    other.content_root = ".content-v1".to_string();
    assert!(!original.equal(&other));
    other = original.clone();
    other.codec = "other".to_string();
    assert!(!original.equal(&other));
    other = original.clone();
    other.schema_version = 3;
    assert!(!original.equal(&other));
    other = original.clone();
    other.session_id = "other".to_string();
    assert!(!original.equal(&other));
    other = original.clone();
    other.created_at = "2026-09-25T12:00:00.000000001Z".parse().expect("time");
    assert!(!original.equal(&other));
}

#[test]
fn open_rejects_each_wrong_manifest_field_with_a_typed_class() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let valid = manifest(crate::sessionstore::sessionv4::test_support::ID);
    let cases = [
        ("schemaVersion", json!(3)),
        ("codec", json!("other")),
        ("storageRevision", json!(4)),
        ("sessionId", json!("other")),
    ];
    for (field, value) in cases {
        let mut raw = serde_json::to_value(&valid).expect("json");
        raw[field] = value;
        fs::write(fixture.dir.join("manifest.json"), raw.to_string()).expect("write");
        let error = open(&fixture.dir).expect_err("wrong manifest");
        if field == "sessionId" {
            assert!(matches!(error, V4Error::Damaged(Damage::ManifestIdentity)));
        } else {
            assert!(matches!(error, V4Error::Unsupported(_)));
        }
    }
}

#[test]
fn manifest_size_cap_accepts_the_boundary_and_rejects_one_more() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let raw = serde_json::to_vec(&manifest(crate::sessionstore::sessionv4::test_support::ID))
        .expect("json");
    let padding = super::MAX_MANIFEST_BYTES - raw.len();
    let padded = format!(
        "{}{}",
        String::from_utf8(raw).expect("utf8"),
        " ".repeat(padding)
    );
    fs::write(fixture.dir.join("manifest.json"), &padded).expect("write");
    assert!(open(&fixture.dir).is_ok());
    fs::write(fixture.dir.join("manifest.json"), format!("{padded} ")).expect("write");
    assert!(matches!(
        open(&fixture.dir),
        Err(V4Error::TooLarge(Limit::Manifest(_)))
    ));
}

#[test]
fn frame_reader_distinguishes_torn_tails_from_damage_and_oversize() {
    let raw = br#"{"recordType":"batch/begin"}"#;
    let valid = crate::sessionstore::sessionv4::test_support::frame(raw);
    assert_eq!(
        read_frame(&mut Cursor::new(&valid))
            .expect("frame")
            .expect("complete"),
        raw
    );
    for cut in [1, 7, valid.len() - 1] {
        assert!(
            read_frame(&mut Cursor::new(&valid[..cut]))
                .expect("torn")
                .is_none()
        );
    }
    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        read_frame(&mut Cursor::new(bad_magic)),
        Err(V4Error::Damaged(Damage::FrameMagic))
    ));
    let mut zero = valid.clone();
    zero[8..12].copy_from_slice(&0u32.to_be_bytes());
    assert!(matches!(
        read_frame(&mut Cursor::new(zero)),
        Err(V4Error::Damaged(Damage::FrameZero))
    ));
    let mut oversized = valid.clone();
    oversized[4..8].copy_from_slice(&((super::MAX_FRAME_BYTES + 1) as u32).to_be_bytes());
    assert!(matches!(
        read_frame(&mut Cursor::new(oversized)),
        Err(V4Error::TooLarge(Limit::Frame { .. }))
    ));
    let mut invalid_zstd = valid;
    invalid_zstd[12..16].fill(0);
    assert!(matches!(
        read_frame(&mut Cursor::new(invalid_zstd)),
        Err(V4Error::Damaged(Damage::FrameDecode(_)))
    ));
    let mut wrong_raw_len = crate::sessionstore::sessionv4::test_support::frame(raw);
    let actual = u32::from_be_bytes(wrong_raw_len[8..12].try_into().expect("bytes"));
    wrong_raw_len[8..12].copy_from_slice(&(actual + 1).to_be_bytes());
    assert!(matches!(
        read_frame(&mut Cursor::new(wrong_raw_len)),
        Err(V4Error::Damaged(Damage::FrameLength { .. }))
    ));
}

#[test]
fn transcript_folds_upsert_retract_replace_title_and_config() {
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fixture.batch(
        vec![
            crate::sessionstore::sessionv4::test_support::message("a", "user", "one"),
            crate::sessionstore::sessionv4::test_support::message("b", "assistant", "two"),
            json!({"kind":"message/complete","payload":{"message":{"id":"a","role":"user","content":"ignored"}}}),
            json!({"kind":"message/upsert","payload":{"message":{"id":"a","role":"user","content":"updated"}}}),
            json!({"kind":"message/retract","payload":{"messageIds":["b"]}}),
            crate::sessionstore::sessionv4::test_support::message("c", "user", "three"),
            json!({"kind":"session/title","payload":{"title":"  Name  "}}),
            json!({"kind":"session/config","payload":{"modelRef":" model-a "}}),
            json!({"kind":"session/config","payload":{"modelRef":"  "}}),
        ],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let transcript = open(&fixture.dir)
        .expect("open")
        .transcript()
        .expect("transcript");
    assert_eq!(transcript.messages.len(), 2);
    assert_eq!(transcript.messages[0].content, "updated");
    assert_eq!(transcript.messages[1].content, "three");
    assert_eq!(transcript.title, "Name");
    assert_eq!(transcript.model_ref, "model-a");
    fixture.batch(
        vec![json!({"kind":"history/replace","payload":{"messages":[
            {"id":"x","role":"user","content":"first"},
            {"id":"x","role":"user","content":"duplicate"},
            {"role":"assistant","content":"anonymous"}
        ]}})],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let transcript = open(&fixture.dir)
        .expect("open")
        .transcript()
        .expect("transcript");
    assert_eq!(transcript.messages.len(), 2);
    assert_eq!(transcript.messages[0].content, "first");
    fixture.batch(
        vec![json!({"kind":"message/retract","payload":{"messageIds":[""]}})],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    assert_eq!(
        open(&fixture.dir)
            .expect("open")
            .transcript()
            .expect("transcript")
            .messages
            .len(),
        1
    );
}

#[test]
fn payloads_decode_inline_from_pool_and_from_v1_image_inputs() {
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let payload = json!({"message":{"id":"pooled","role":"user","content":"large"}});
    let reference = fixture.object(
        &serde_json::to_vec(&payload).expect("json"),
        "application/json",
    );
    fixture.batch(
        vec![json!({"kind":"message/complete","payloadRef":reference})],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let image = fixture.object(b"image bytes", "image/png");
    fixture.batch(
        vec![json!({"kind":"message/upsert","payload":{"message":{
            "id":"images","role":"user","content":"pictures","host_authored":"legacy-invalid",
            "origin":"host","image_inputs":[
                {"kind":"attachment","attachment":{"content":image}},
                {"kind":"url","url":"https://example.test/a.png"}
            ]
        }}})],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let transcript = open(&fixture.dir)
        .expect("open")
        .transcript()
        .expect("transcript");
    assert_eq!(transcript.messages[0].content, "large");
    assert!(transcript.messages[1].host_authored);
    assert_eq!(
        transcript.messages[1].images[0],
        "data:image/png;base64,aW1hZ2UgYnl0ZXM="
    );
    assert_eq!(
        transcript.messages[1].images[1],
        "https://example.test/a.png"
    );
}

#[test]
fn content_pool_validates_digest_size_and_manifest_confinement() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let session = open(&fixture.dir).expect("open");
    let reference = fixture.object(b"abc", "text/plain");
    let pool = ContentPool::for_session(&session);
    assert_eq!(
        pool.read(&serde_json::from_value(reference.clone()).expect("ref"))
            .expect("read"),
        b"abc"
    );
    let mut wrong_size = reference.clone();
    wrong_size["bytes"] = json!(4);
    assert!(matches!(
        pool.read(&serde_json::from_value(wrong_size).expect("ref")),
        Err(V4Error::Damaged(Damage::ContentSize { .. }))
    ));
    let digest = reference["digest"].as_str().expect("digest").to_string();
    let object = fixture
        .root
        .join(".content-v1/objects")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(&digest);
    fs::write(object, b"abd").expect("corrupt object");
    assert!(matches!(
        pool.read(&serde_json::from_value(reference.clone()).expect("ref")),
        Err(V4Error::Damaged(Damage::ContentDigest))
    ));
    let mut traversal = reference.clone();
    traversal["digest"] = json!("../../outside");
    assert!(matches!(
        pool.read(&serde_json::from_value(traversal).expect("ref")),
        Err(V4Error::Damaged(Damage::ContentRef))
    ));
    let outside = fixture
        .temp
        .path()
        .join("outside/objects")
        .join(&digest[..2])
        .join(&digest[2..4]);
    fs::create_dir_all(&outside).expect("outside dirs");
    fs::write(outside.join(&digest), b"abc").expect("outside object");
    fs::remove_file(
        fixture
            .root
            .join(".content-v1/objects")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(&digest),
    )
    .expect("remove pooled object");
    let mut raw = serde_json::to_value(manifest(crate::sessionstore::sessionv4::test_support::ID))
        .expect("json");
    raw["contentRoot"] = json!("../../outside");
    fs::write(fixture.dir.join("manifest.json"), raw.to_string()).expect("write");
    assert!(matches!(
        ContentPool::for_session(&open(&fixture.dir).expect("open"))
            .read(&serde_json::from_value(reference).expect("ref")),
        Err(V4Error::Damaged(Damage::ContentObject { .. }))
    ));
}

#[test]
fn batch_checks_sequence_count_commit_and_checksum() {
    for case in 0..4 {
        let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
            crate::sessionstore::sessionv4::test_support::ID,
        );
        let first = json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/begin","commitId":"c","firstSeq":1,"eventCount":1});
        let event = json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/event","event":{"id":"e","seq":1,"kind":"message/complete","payload":{"message":{"id":"m","role":"user","content":"x"}}}});
        let end = json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/end","commitId":"c","sha256":"00".repeat(32)});
        let records = match case {
            0 => vec![first.clone(), event.clone(), end],
            1 => vec![
                json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/begin","commitId":"c","firstSeq":2,"eventCount":1}),
                event,
                end,
            ],
            2 => vec![
                first,
                json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/event","event":{"id":"e","seq":2,"kind":"message/complete","payload":{}}}),
                end,
            ],
            _ => vec![
                first,
                event,
                json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"batch/end","commitId":"other","sha256":"00".repeat(32)}),
            ],
        };
        crate::sessionstore::sessionv4::test_support::write_frames(&fixture.dir, &records);
        assert!(matches!(
            open(&fixture.dir).expect("open").transcript(),
            Err(V4Error::Damaged(_))
        ));
    }
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fixture.batch(
        vec![crate::sessionstore::sessionv4::test_support::message(
            "a",
            "user",
            "committed",
        )],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    fixture.batch(
        vec![crate::sessionstore::sessionv4::test_support::message(
            "b", "user", "torn",
        )],
        crate::sessionstore::sessionv4::test_support::End::Torn,
    );
    assert_eq!(
        open(&fixture.dir)
            .expect("open")
            .transcript()
            .expect("transcript")
            .messages
            .len(),
        1
    );
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fixture.batch(
        vec![crate::sessionstore::sessionv4::test_support::message(
            "a", "user", "x",
        )],
        crate::sessionstore::sessionv4::test_support::End::BadSum,
    );
    assert!(matches!(
        open(&fixture.dir).expect("open").transcript(),
        Err(V4Error::Damaged(Damage::Checksum))
    ));
}

#[test]
fn unknown_record_is_unsupported_not_damage() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    crate::sessionstore::sessionv4::test_support::write_frames(
        &fixture.dir,
        &[json!({"schemaVersion":4,"codec":super::CODEC,"recordType":"future/record"})],
    );
    assert!(matches!(
        open(&fixture.dir).expect("open").transcript(),
        Err(V4Error::Unsupported(Unsupported::RecordType(_)))
    ));
}

#[test]
fn listing_skips_hidden_invalid_and_non_directory_entries() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fs::create_dir_all(fixture.root.join(".hidden")).expect("dir");
    fs::create_dir_all(fixture.root.join("bad")).expect("dir");
    fs::write(fixture.root.join("bad/manifest.json"), "{}").expect("write");
    fs::write(fixture.root.join("file"), "{}").expect("write");
    assert_eq!(list(&fixture.root).len(), 1);
    assert!(open_in(&fixture.root, "..").is_err());
}

#[test]
fn listing_and_updated_at_use_log_stamp_and_creation_floor() {
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fixture.batch(
        vec![crate::sessionstore::sessionv4::test_support::message(
            "a", "user", "hello",
        )],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let session = open(&fixture.dir).expect("open");
    let transcript = session.transcript().expect("transcript");
    let (preview, turns) = session_preview_from_messages(&transcript.messages);
    assert_eq!(preview, "hello");
    assert_eq!(turns, 1);
    assert_eq!(
        session.updated_at(),
        session.log_mod_time.max(session.manifest.created_at)
    );
}

#[test]
fn source_catalog_cache_checks_identity_stamp_and_creation_text() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let mut cache = ListingCache::default();
    let session = open(&fixture.dir).expect("open");
    let listing = super::session::Listing {
        title: " Title ".into(),
        preview: "preview".into(),
        turns: 2,
        model_ref: "m".into(),
    };
    cache.put(&session, listing.clone());
    assert_eq!(cache.get(&session).expect("hit").title, "Title");
    let mut changed = session.clone();
    changed.log_size += 1;
    assert!(cache.get(&changed).is_none());
    changed = session.clone();
    changed.manifest.storage_revision = 2;
    assert!(cache.get(&changed).is_none());
}

#[test]
fn source_store_can_be_renamed_after_all_handles_close() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let session = open(&fixture.dir).expect("open");
    let _ = session.transcript().expect("transcript");
    let moved = fixture.root.with_extension("moved");
    fs::rename(&fixture.root, &moved).expect("rename source");
    assert!(
        moved
            .join(crate::sessionstore::sessionv4::test_support::ID)
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn confined_metadata_does_not_open_the_final_entry() {
    let temp = TempDir::new().expect("tempdir");
    let socket =
        std::os::unix::net::UnixListener::bind(temp.path().join("value")).expect("unix socket");
    let store = super::fs::ConfinedRoot::new(temp.path()).expect("confined root");

    let metadata = store
        .metadata(std::path::Path::new("value"))
        .expect("socket metadata");
    assert!(metadata.file_type().is_socket());
    drop(socket);
}

#[test]
fn confined_root_refuses_a_replaced_root_directory() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("root");
    fs::create_dir(&root).expect("root");
    fs::write(root.join("value"), b"original").expect("original");
    let store = super::fs::ConfinedRoot::new(&root).expect("confined root");
    let moved = temp.path().join("moved");
    fs::rename(&root, &moved).expect("move root");
    fs::create_dir(&root).expect("replacement root");
    fs::write(root.join("value"), b"replacement").expect("replacement");

    assert!(store.read(std::path::Path::new("value")).is_err());
}

#[test]
fn reads_after_the_source_moves_return_typed_errors_instead_of_panicking() {
    let mut fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    fixture.batch(
        vec![crate::sessionstore::sessionv4::test_support::message(
            "a", "user", "hello",
        )],
        crate::sessionstore::sessionv4::test_support::End::Ended,
    );
    let reference = fixture.object(b"pooled", "text/plain");
    let session = open(&fixture.dir).expect("open");
    let moved = fixture.root.with_extension("moved");
    fs::rename(&fixture.root, &moved).expect("rename source");
    let pool = ContentPool::for_session(&session);
    assert!(matches!(
        pool.read(&serde_json::from_value(reference).expect("reference")),
        Err(V4Error::Damaged(Damage::ContentObject { .. }))
    ));
    assert!(matches!(session.transcript(), Err(V4Error::Io { .. })));
}

#[test]
fn v4_root_rule_only_matches_a_sessions_directory() {
    let temp = TempDir::new().expect("tempdir");
    assert_eq!(
        v4_sessions_root(&temp.path().join("sessions")),
        temp.path().join("sessions-v4")
    );
    assert!(
        v4_sessions_root(&temp.path().join("other"))
            .as_os_str()
            .is_empty()
    );
}

fn link_dir(target: &std::path::Path, link: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("symlink");
    #[cfg(windows)]
    {
        let target = target.canonicalize().expect("canonical target");
        let link = link
            .parent()
            .expect("link parent")
            .canonicalize()
            .expect("canonical parent")
            .join(link.file_name().expect("link name"));
        let status = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .expect("create junction");
        assert!(status.success());
    }
}

fn link_confined_dir(target: &std::path::Path, link: &std::path::Path, name: &str) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(std::path::Path::new(name), link).expect("symlink");
    #[cfg(windows)]
    {
        let _ = name;
        link_dir(target, link);
    }
}

#[test]
fn open_in_refuses_external_session_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let outside = TempDir::new().expect("outside");
    let target = outside
        .path()
        .join(crate::sessionstore::sessionv4::test_support::ID);
    fs::rename(&fixture.dir, &target).expect("move session");
    link_dir(&target, &fixture.dir);

    assert!(
        open_in(
            &fixture.root,
            crate::sessionstore::sessionv4::test_support::ID
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn open_in_refuses_relative_external_session_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let target = fixture
        .temp
        .path()
        .join("outside")
        .join(crate::sessionstore::sessionv4::test_support::ID);
    fs::create_dir_all(target.parent().expect("outside parent")).expect("outside");
    fs::rename(&fixture.dir, &target).expect("move session");
    std::os::unix::fs::symlink(
        std::path::Path::new("../outside").join(crate::sessionstore::sessionv4::test_support::ID),
        &fixture.dir,
    )
    .expect("relative external symlink");

    assert!(
        open_in(
            &fixture.root,
            crate::sessionstore::sessionv4::test_support::ID
        )
        .is_err()
    );
}

#[test]
fn open_in_allows_confined_session_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let target = fixture.root.join("real-session");
    fs::rename(&fixture.dir, &target).expect("move session");
    link_confined_dir(&target, &fixture.dir, "real-session");

    let session = open_in(
        &fixture.root,
        crate::sessionstore::sessionv4::test_support::ID,
    )
    .expect("confined session link");
    assert_eq!(
        session.manifest.session_id,
        crate::sessionstore::sessionv4::test_support::ID
    );
}

#[cfg(unix)]
#[test]
fn open_in_allows_absolute_confined_session_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let target = fixture.root.join("real-session");
    fs::rename(&fixture.dir, &target).expect("move session");
    link_dir(&target, &fixture.dir);

    let session = open_in(
        &fixture.root,
        crate::sessionstore::sessionv4::test_support::ID,
    )
    .expect("absolute confined session link");
    assert_eq!(
        session.manifest.session_id,
        crate::sessionstore::sessionv4::test_support::ID
    );
}

#[test]
fn content_pool_refuses_external_object_directory_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let reference = fixture.object(b"pooled", "text/plain");
    let digest = reference["digest"].as_str().expect("digest");
    let source = fixture.root.join(".content-v1/objects").join(&digest[..2]);
    let outside = TempDir::new().expect("outside");
    let target = outside.path().join(&digest[..2]);
    fs::rename(&source, &target).expect("move objects");
    link_dir(&target, &source);
    let session = open(&fixture.dir).expect("open");
    let pool = ContentPool::for_session(&session);

    assert!(matches!(
        pool.read(&serde_json::from_value(reference).expect("reference")),
        Err(V4Error::Damaged(Damage::ContentObject { .. }))
    ));
}

#[test]
fn content_pool_allows_confined_object_directory_links() {
    let fixture = crate::sessionstore::sessionv4::test_support::Fixture::new(
        crate::sessionstore::sessionv4::test_support::ID,
    );
    let reference = fixture.object(b"pooled", "text/plain");
    let digest = reference["digest"].as_str().expect("digest");
    let objects = fixture.root.join(".content-v1/objects");
    let source = objects.join(&digest[..2]);
    let target = objects.join(format!("real-{}", &digest[..2]));
    fs::rename(&source, &target).expect("move objects");
    link_confined_dir(
        &target,
        &source,
        target.file_name().expect("name").to_str().expect("utf8"),
    );
    let session = open(&fixture.dir).expect("open");
    let pool = ContentPool::for_session(&session);

    assert_eq!(
        pool.read(&serde_json::from_value(reference).expect("reference"))
            .expect("confined object"),
        b"pooled"
    );
}

#[allow(dead_code)]
fn _transcript_type_is_public(_: Transcript) {}
