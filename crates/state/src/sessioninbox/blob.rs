//! Go-compatible envelope encoding and replay fingerprints.

use std::collections::BTreeMap;

use reasonix_contract::gojson::marshal;
use serde::Serialize;

use super::Result;
use super::manifest::valid_sha256;
use super::sha256_hex;
use super::types::{Error, PromptEnvelope};

pub(crate) fn encode_envelope(env: &PromptEnvelope) -> Result<(Vec<u8>, String, i64)> {
    let data = marshal(env)?.into_bytes();
    let checksum = sha256_hex(&data);
    let size = i64::try_from(data.len()).map_err(|_| Error::ItemTooLarge)?;
    Ok((data, checksum, size))
}

pub(crate) fn idempotency_request_hash(env: &PromptEnvelope) -> Result<String> {
    if env.fingerprint_version != 0 {
        if env.fingerprint_version != 1 || !valid_sha256(&env.request_fingerprint) {
            return Err(Error::InvalidRequestFingerprint);
        }
        return Ok(env.request_fingerprint.clone());
    }

    let mut stored = env.invocations.clone();
    if stored.is_empty()
        && let Some(invocation) = &env.invocation
    {
        stored.push(invocation.clone());
    }
    stored.sort_by_key(|invocation| invocation.offset);
    let invocations: Vec<StableInvocation<'_>> = stored
        .iter()
        .map(|invocation| StableInvocation {
            name: &invocation.name,
            args: &invocation.args,
            display: &invocation.display,
        })
        .collect();
    let stable = StableEnvelope {
        display_text: &env.display_text,
        raw_text: &env.raw_text,
        submit_text: &env.submit_text,
        invocations: &invocations,
        format: &env.format,
        attachments: &env.attachments,
        attachment_identities: &env.attachment_identities,
        explicit_refs: &env.explicit_refs,
        source: &env.source,
        extra: &env.extra,
    };
    Ok(sha256_hex(marshal(&stable)?.as_bytes()))
}

#[derive(Serialize)]
struct StableInvocation<'a> {
    #[serde(rename = "name", skip_serializing_if = "str::is_empty")]
    name: &'a str,
    #[serde(rename = "args", skip_serializing_if = "BTreeMap::is_empty")]
    args: &'a BTreeMap<String, String>,
    #[serde(rename = "display", skip_serializing_if = "str::is_empty")]
    display: &'a str,
}

#[derive(Serialize)]
struct StableEnvelope<'a> {
    #[serde(rename = "displayText")]
    display_text: &'a str,
    #[serde(rename = "rawText")]
    raw_text: &'a str,
    #[serde(rename = "submitText")]
    submit_text: &'a str,
    #[serde(rename = "invocations", skip_serializing_if = "Vec::is_empty")]
    invocations: &'a Vec<StableInvocation<'a>>,
    #[serde(rename = "format", skip_serializing_if = "str::is_empty")]
    format: &'a str,
    #[serde(rename = "attachments", skip_serializing_if = "Vec::is_empty")]
    attachments: &'a Vec<String>,
    #[serde(rename = "attachmentIdentities", skip_serializing_if = "Vec::is_empty")]
    attachment_identities: &'a Vec<String>,
    #[serde(rename = "explicitRefs", skip_serializing_if = "Vec::is_empty")]
    explicit_refs: &'a Vec<String>,
    #[serde(rename = "source", skip_serializing_if = "str::is_empty")]
    source: &'a str,
    #[serde(rename = "extra", skip_serializing_if = "BTreeMap::is_empty")]
    extra: &'a BTreeMap<String, String>,
}
