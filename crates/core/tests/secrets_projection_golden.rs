//! Golden corpus comparison for the secrets endpoint projection.
//!
//! fixtures/secrets-projection/corpus.json holds projection inputs and
//! expected.json the outputs recorded from the reference Go
//! implementation (internal/base/secrets, package reasonix). Any
//! divergence is a parity bug: the projection must mask at least as
//! aggressively as the reference and pass through exactly the same
//! harmless values.

use std::collections::BTreeMap;
use std::fs;

use reasonix_core::secrets;
use serde_json::{Value, to_value};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/secrets-projection"
);

fn load(name: &str) -> Value {
    let text = fs::read_to_string(format!("{FIXTURES}/{name}")).expect("fixture readable");
    serde_json::from_str(&text).expect("fixture is valid JSON")
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array fixture")
        .iter()
        .map(|v| v.as_str().expect("string entry").to_string())
        .collect()
}

#[test]
fn secrets_projection_matches_reference_corpus() {
    let corpus = load("corpus.json");
    let expected = load("expected.json");

    // Config maps: order-stable projection per map.
    let maps = corpus["configMaps"].as_array().expect("configMaps");
    let want_maps = expected["configMaps"]
        .as_array()
        .expect("expected configMaps");
    assert_eq!(maps.len(), want_maps.len(), "config map count");
    for (i, raw) in maps.iter().enumerate() {
        let mut fields = BTreeMap::new();
        for (k, v) in raw.as_object().expect("map fixture") {
            fields.insert(k.clone(), v.as_str().expect("string value").to_string());
        }
        let got = to_value(secrets::redact_config_map(Some(&fields))).expect("serializable");
        assert_eq!(
            got,
            want_maps[i],
            "config map #{i} ({raw}) projected to {got}, want {want}",
            want = want_maps[i]
        );
    }

    // Argument vectors.
    let vectors = corpus["argVectors"].as_array().expect("argVectors");
    let want_vectors = expected["argVectors"]
        .as_array()
        .expect("expected argVectors");
    assert_eq!(vectors.len(), want_vectors.len(), "argument vector count");
    for (i, raw) in vectors.iter().enumerate() {
        let args = strings(raw);
        let got = to_value(secrets::redact_args(&args)).expect("serializable");
        assert_eq!(
            got,
            want_vectors[i],
            "argument vector #{i} ({raw}) projected to {got}, want {want}",
            want = want_vectors[i]
        );
    }

    // Endpoints.
    let endpoints = strings(&corpus["rawEndpoints"]);
    let want_endpoints = strings(&expected["rawEndpoints"]);
    assert_eq!(endpoints.len(), want_endpoints.len(), "endpoint count");
    for (i, raw) in endpoints.iter().enumerate() {
        let got = secrets::redact_endpoint(raw);
        assert_eq!(
            got,
            want_endpoints[i],
            "endpoint #{i} ({raw}) projected to {got}, want {want}",
            want = want_endpoints[i]
        );
    }

    // Keyed config values.
    let values = corpus["configValues"].as_array().expect("configValues");
    let want_values = strings(&expected["configValues"]);
    assert_eq!(values.len(), want_values.len(), "config value count");
    for (i, kv) in values.iter().enumerate() {
        let key = kv["key"].as_str().expect("key");
        let value = kv["value"].as_str().expect("value");
        let got = secrets::redact_config_value(key, value);
        assert_eq!(
            got,
            want_values[i],
            "config value #{i} ({key}={value}) projected to {got}, want {want}",
            want = want_values[i]
        );
    }

    // Credential key detection.
    let keys = strings(&corpus["credentialKeys"]);
    let want_keys: Vec<bool> = expected["credentialKeys"]
        .as_array()
        .expect("expected credentialKeys")
        .iter()
        .map(|v| v.as_bool().expect("bool"))
        .collect();
    assert_eq!(keys.len(), want_keys.len(), "credential key count");
    for (i, key) in keys.iter().enumerate() {
        assert_eq!(
            secrets::credential_key(key),
            want_keys[i],
            "credential key #{i} ({key})"
        );
    }

    // Credential value detection.
    let values = strings(&corpus["credentialValues"]);
    let want_values: Vec<bool> = expected["credentialValues"]
        .as_array()
        .expect("expected credentialValues")
        .iter()
        .map(|v| v.as_bool().expect("bool"))
        .collect();
    assert_eq!(values.len(), want_values.len(), "credential value count");
    for (i, value) in values.iter().enumerate() {
        assert_eq!(
            secrets::credential_value(value),
            want_values[i],
            "credential value #{i} ({value})"
        );
    }
}
