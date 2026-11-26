use std::path::Path;

use super::path_clean::lexical_clean;

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

#[cfg(windows)]
fn encoded_units(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().collect()
}

#[cfg(not(windows))]
fn encoded_units(path: &Path) -> Vec<u8> {
    path.as_os_str().as_encoded_bytes().to_vec()
}

#[cfg(not(windows))]
fn assert_clean(input: &str, expected: &str) {
    assert_eq!(
        encoded_units(&lexical_clean(Path::new(input))),
        encoded_units(Path::new(expected)),
        "input {input:?}"
    );
}

#[cfg(windows)]
#[test]
fn windows_lazy_buffer_regressions_match_go_1_26_11() {
    let mut mismatches = Vec::new();
    for (input, expected) in [
        ("/", r"\"),
        (r"\\server\..\share\file", r"\\server\..\share\file"),
        ("/??/a", "/??/a."),
        ("C:..", "C:.."),
        (r"C:..\foo\..", "C:.."),
        ("./b:/../c", r".\c"),
        (r"é:/\a/..\\../\..//.", r".\.."),
        (r"é:share\\?//../../\..", r".\.."),
    ] {
        let actual = lexical_clean(Path::new(input));
        if actual.as_os_str() != Path::new(expected).as_os_str() {
            mismatches.push(format!("{input:?}: got {actual:?}, expected {expected:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[cfg(windows)]
#[test]
fn windows_clean_preserves_unpaired_surrogates() {
    let input = std::ffi::OsString::from_wide(&[0xd800, b':' as u16, b'/' as u16, b'a' as u16]);
    let expected = std::ffi::OsString::from_wide(&[
        b'.' as u16,
        b'\\' as u16,
        0xd800,
        b':' as u16,
        b'\\' as u16,
        b'a' as u16,
    ]);
    assert_eq!(
        lexical_clean(Path::new(&input)).as_os_str(),
        Path::new(&expected).as_os_str()
    );
}

#[cfg(windows)]
fn decode_units(encoded: &str) -> Vec<u16> {
    assert_eq!(encoded.len() % 4, 0);
    encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|byte| u8::from_str_radix(std::str::from_utf8(byte).unwrap(), 16).unwrap())
        .collect::<Vec<_>>()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect()
}

#[cfg(windows)]
#[test]
fn windows_clean_matches_go_1_26_11_generated_oracle_corpus() {
    const CORPUS: &str = include_str!("testdata/path_clean_windows.tsv");
    let mut count = 0;
    let mut mismatch_count = 0;
    let mut mismatches = Vec::new();
    for line in CORPUS.lines() {
        let (input, expected) = line.split_once('\t').unwrap();
        let input = std::ffi::OsString::from_wide(&decode_units(input));
        let expected = decode_units(expected);
        let actual = encoded_units(&lexical_clean(Path::new(&input)));
        count += 1;
        if actual != expected {
            mismatch_count += 1;
        }
        if actual != expected && mismatches.len() < 12 {
            mismatches.push(format!(
                "input {}: got {}, expected {}",
                input.to_string_lossy(),
                actual
                    .iter()
                    .map(|unit| format!("{unit:04x}"))
                    .collect::<String>(),
                expected
                    .iter()
                    .map(|unit| format!("{unit:04x}"))
                    .collect::<String>()
            ));
        }
    }
    assert_eq!(count, 10_267);
    assert!(
        mismatches.is_empty(),
        "{mismatch_count} of {count} corpus paths differ:\n{}",
        mismatches.join("\n")
    );
}

#[cfg(not(windows))]
#[test]
fn unix_clean_matches_go_table() {
    for (input, expected) in [
        ("", "."),
        ("abc//def/../ghi", "abc/ghi"),
        ("../../abc", "../../abc"),
        ("/abc/def/../../..", "/"),
        ("//abc//", "/abc"),
        ("a/b/c/..", "a/b"),
    ] {
        assert_clean(input, expected);
    }
}
