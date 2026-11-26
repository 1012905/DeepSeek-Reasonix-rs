//! Stable run identity, SHA-256 rendering, and bounded previews.

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use super::types::DEFAULT_PREVIEW_RUNES;

pub fn process_run_id() -> &'static str {
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(new_random_id)
}

pub(crate) fn new_random_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let digest = Sha256::digest(format!("fallback-{}-{nanos}", std::process::id()).as_bytes());
        bytes.copy_from_slice(&digest[..16]);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex_lower(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_lower(&Sha256::digest(data))
}

pub fn preview_text(text: &str, max_runes: i64) -> String {
    let max_runes = if max_runes <= 0 {
        DEFAULT_PREVIEW_RUNES
    } else {
        max_runes
    };
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let capacity = usize::try_from(max_runes)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
        .min(text.len());
    let mut output = String::with_capacity(capacity);
    let mut chars = text.chars().peekable();
    let mut previous_space = false;
    let mut count = 0i64;
    while let Some(ch) = chars.next() {
        if matches!(ch, '\n' | '\r' | '\t' | ' ') {
            if previous_space || count == 0 {
                continue;
            }
            output.push(' ');
            previous_space = true;
            count += 1;
            if count >= max_runes {
                break;
            }
            continue;
        }
        previous_space = false;
        output.push(ch);
        count += 1;
        if count >= max_runes {
            if chars.peek().is_some() {
                output.push('…');
            }
            break;
        }
    }
    output.trim().to_owned()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
