//! ACP-039: additional-directory limits, three frontend codes, seven root vet
//! codes, the `{code, path?}` refusal shape, real-path dedup, open-time
//! comparison, and the disabled factory mapping.

use reasonix_acp::dirs_vet::{
    CODE_DISABLED, CODE_SESSION_OPEN, CODE_TOO_MANY, ExtraRootVetter, ROOT_COVERED_ALREADY,
    ROOT_NOT_DIRECTORY, ROOT_NOT_OWNED, ROOT_OVERLAPS_WRITE, ROOT_PROTECTED, ROOT_SKILL_STORE,
    ROOT_TOO_BROAD, RootRefusal, disabled_refusal, open_dirs_refusal, same_dirs,
    vet_additional_dirs,
};

struct FakeVetter {
    refusals: Vec<(&'static str, &'static str)>,
}

impl ExtraRootVetter for FakeVetter {
    fn vet_extra_root(
        &self,
        _cwd: &str,
        dir: &str,
        _existing: &[String],
    ) -> Result<String, RootRefusal> {
        for (needle, code) in &self.refusals {
            if dir.contains(needle) {
                return Err(RootRefusal {
                    code: (*code).to_owned(),
                });
            }
        }
        Ok(format!("real:{dir}"))
    }
}

#[test]
fn empty_list_passes_without_calling_the_vetter() {
    let vetter = FakeVetter { refusals: vec![] };
    assert!(vet_additional_dirs("/ws", &[], &vetter).unwrap().is_empty());
}

#[test]
fn more_than_thirty_two_refuses_whole_with_too_many_code() {
    let vetter = FakeVetter { refusals: vec![] };
    let dirs: Vec<String> = (0..=32).map(|i| format!("/d/{i}")).collect();
    let err = vet_additional_dirs("/ws", &dirs, &vetter).unwrap_err();
    assert_eq!(err.code, CODE_TOO_MANY);
    assert!(err.path.is_none());
    assert_eq!(err.message, "too many additional directories");
}

#[test]
fn over_long_path_reuses_the_not_directory_root_code_and_carries_the_folder() {
    let vetter = FakeVetter { refusals: vec![] };
    let long = "x".repeat(4097);
    let err = vet_additional_dirs("/ws", std::slice::from_ref(&long), &vetter).unwrap_err();
    assert_eq!(err.code, ROOT_NOT_DIRECTORY);
    assert_eq!(err.path, Some(long));
    assert_eq!(err.message, "additional directory path is too long");
}

#[test]
fn seven_root_refusal_codes_are_exposed_with_the_folder_path() {
    let cases = [
        ("secret", ROOT_PROTECTED),
        ("primary", ROOT_COVERED_ALREADY),
        ("broad", ROOT_TOO_BROAD),
        ("skills", ROOT_SKILL_STORE),
        ("owned", ROOT_NOT_OWNED),
        ("overlap", ROOT_OVERLAPS_WRITE),
    ];
    for (needle, code) in cases {
        let vetter = FakeVetter {
            refusals: vec![(needle, code)],
        };
        let dir = format!("/ws/{needle}-folder");
        let err = vet_additional_dirs("/ws", std::slice::from_ref(&dir), &vetter).unwrap_err();
        assert_eq!(err.code, code, "{code}");
        assert_eq!(err.path, Some(dir));
        assert_eq!(err.message, "this folder cannot be added");
    }
}

#[test]
fn resolved_real_paths_are_deduplicated_within_the_session() {
    let vetter = FakeVetter { refusals: vec![] };
    let out = vet_additional_dirs(
        "/ws",
        &["/sibling".to_owned(), "/sibling/".to_owned()],
        &vetter,
    )
    .unwrap();
    assert_eq!(
        out,
        vec!["real:/sibling".to_owned(), "real:/sibling/".to_owned()]
    );
}

#[test]
fn same_dirs_compares_sorted_regardless_of_order() {
    assert!(same_dirs(
        &["/a".to_owned(), "/b".to_owned()],
        &["/b".to_owned(), "/a".to_owned()]
    ));
    assert!(!same_dirs(
        &["/a".to_owned(), "/b".to_owned()],
        &["/a".to_owned()]
    ));
    assert!(!same_dirs(&["/a".to_owned()], &["/b".to_owned()]));
}

#[test]
fn open_mismatch_refuses_with_session_open_code_and_verbatim_message() {
    let persisted = vec!["/sibling".to_owned()];
    assert!(open_dirs_refusal("session/resume", &persisted, &persisted).is_none());
    let err = open_dirs_refusal("session/resume", &["/other".to_owned()], &persisted).unwrap();
    assert_eq!(err.code, CODE_SESSION_OPEN);
    assert_eq!(
        err.message,
        "session/resume: the session is open with other additional directories; close it first"
    );
}

#[test]
fn disabled_factory_refusal_carries_off_code_and_sentinel_message() {
    let err = disabled_refusal("session/new");
    assert_eq!(err.code, CODE_DISABLED);
    assert_eq!(
        err.message,
        "session/new: this agent writes only inside the session's cwd"
    );
}
