//! Golden parity with the Go encoding package over a shared hex corpus.
//!
//! The corpus and expected outputs were produced by the reference Go
//! implementation (.parity-gocopy/cmd/encprobe over corpus-encoding.json);
//! cases marked final exercise DetectAndDecode (decoded bytes), the rest
//! exercise DetectFragment (raw whole-character prefix).

use std::fs;
use std::path::Path;

use reasonix_core::fileencoding::{detect_and_decode, detect_fragment};

fn hex_decode(s: &str) -> Vec<u8> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(cleaned.len() % 2, 0, "odd hex length: {cleaned}");
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn load(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fileencoding")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("fixture readable")).expect("valid JSON")
}

#[test]
fn fileencoding_matches_reference_corpus() {
    let corpus = load("corpus.json");
    let expected = load("expected.json");
    let corpus = corpus.as_array().expect("corpus array");
    let expected = expected.as_array().expect("expected array");
    assert_eq!(corpus.len(), expected.len(), "corpus/expected size drift");
    for (case, exp) in corpus.iter().zip(expected.iter()) {
        let name = case["name"].as_str().expect("name");
        let data = hex_decode(case["hex"].as_str().expect("hex"));
        let exp_kind = exp["kind"].as_i64().expect("kind");
        let exp_out = hex_decode(exp["outHex"].as_str().expect("outHex"));
        if case["final"].as_bool().expect("final") {
            let (kind, out) = detect_and_decode(&data);
            assert_eq!(kind as i64, exp_kind, "kind mismatch for {name}");
            assert_eq!(
                out.as_ref(),
                exp_out.as_slice(),
                "decode mismatch for {name}"
            );
        } else {
            let (kind, prefix) = detect_fragment(&data);
            assert_eq!(kind as i64, exp_kind, "kind mismatch for {name}");
            assert_eq!(prefix, exp_out.as_slice(), "prefix mismatch for {name}");
        }
    }
}
