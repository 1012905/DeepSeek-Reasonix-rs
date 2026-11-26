//! Stable run identity, SHA-256 rendering, and bounded previews.

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use super::types::DEFAULT_PREVIEW_RUNES;

pub fn process_run_id() -> &'static str {
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(new_random_id)
}

pub(crate) fn new_random_id() -> String {
    random_id_with(|bytes| getrandom::fill(bytes).is_ok())
}

pub(super) fn random_id_with(mut random: impl FnMut(&mut [u8; 16]) -> bool) -> String {
    let mut bytes = [0u8; 16];
    if !random(&mut bytes) {
        return fallback_random_id(&bytes);
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

pub(super) fn fallback_random_id(bytes: &[u8; 16]) -> String {
    let digest = Sha256::digest(format!("fallback-{:p}", bytes).as_bytes());
    hex_lower(&digest[..16])
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex_lower(&Sha256::digest(data))
}

pub fn preview_text(text: &str, max_runes: i64) -> String {
    preview_text_bytes(text.as_bytes(), max_runes)
}

pub fn preview_text_bytes(text: &[u8], max_runes: i64) -> String {
    let max_runes = if max_runes <= 0 {
        DEFAULT_PREVIEW_RUNES
    } else {
        max_runes
    };
    let text = trim_space_bytes(text);
    if text.is_empty() {
        return String::new();
    }
    let capacity = usize::try_from(max_runes)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
        .min(text.len());
    let mut output = String::with_capacity(capacity);
    let mut index = 0usize;
    let mut previous_space = false;
    let mut count = 0i64;
    while index < text.len() {
        let (ch, size) = first_utf8_char(&text[index..]);
        index += size;
        if ch == '\u{fffd}' && size == 1 {
            continue;
        }
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
            if index < text.len() {
                output.push('…');
            }
            break;
        }
    }
    output.trim().to_owned()
}

fn trim_space_bytes(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    while start < bytes.len() {
        let (ch, size) = first_utf8_char(&bytes[start..]);
        if !ch.is_whitespace() {
            break;
        }
        start += size;
    }
    let mut end = bytes.len();
    while end > start {
        let (ch, size) = last_utf8_char(&bytes[start..end]);
        if !ch.is_whitespace() {
            break;
        }
        end -= size;
    }
    &bytes[start..end]
}

fn first_utf8_char(bytes: &[u8]) -> (char, usize) {
    for size in 1..=4.min(bytes.len()) {
        let Ok(text) = std::str::from_utf8(&bytes[..size]) else {
            continue;
        };
        let Some(ch) = text.chars().next() else {
            continue;
        };
        if ch.len_utf8() == size {
            return (ch, size);
        }
    }
    ('\u{fffd}', 1)
}

fn last_utf8_char(bytes: &[u8]) -> (char, usize) {
    for size in 1..=4.min(bytes.len()) {
        let start = bytes.len() - size;
        let Ok(text) = std::str::from_utf8(&bytes[start..]) else {
            continue;
        };
        let Some(ch) = text.chars().next() else {
            continue;
        };
        if ch.len_utf8() == size {
            return (ch, size);
        }
    }
    ('\u{fffd}', 1)
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
