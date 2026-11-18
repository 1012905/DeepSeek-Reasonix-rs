//! Session writer identity and digest rendering (save.go 1074-1101).
//!
//! Revisions are stamped with the process that wrote them so leases,
//! indexes, and conflict diagnostics can tell writers apart.

/// Formats a 32-byte digest as lowercase hex (save.go 1074: digestString).
pub fn digest_string(digest: &[u8; 32]) -> String {
    hex_lower(digest)
}

/// The per-process writer identity (Go: package var sessionWriterID,
/// save.go 72). It is built once per process and reused for every save.
pub fn session_writer_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(new_session_writer_id)
}

/// Builds "host-pid-nonce" where host is the sanitized hostname and nonce
/// is 8 random bytes in hex (save.go 1082). A missing hostname becomes
/// "unknown-host"; an entropy failure falls back to host-pid-UnixNano
/// rather than failing a save.
pub fn new_session_writer_id() -> String {
    let raw_host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_default();
    let mut host = raw_host.trim().to_string();
    if host.is_empty() {
        host = "unknown-host".to_string();
    }
    host = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let pid = std::process::id();
    let mut nonce = [0u8; 8];
    if getrandom::fill(&mut nonce).is_ok() {
        format!("{host}-{pid}-{}", hex_lower(&nonce))
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        format!("{host}-{pid}-{now}")
    }
}

/// Lowercase hex without per-byte formatting.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_string_is_64_lowercase_hex() {
        assert_eq!(digest_string(&[0u8; 32]), "0".repeat(64));
        assert_eq!(digest_string(&[0xff; 32]), "ff".repeat(32));
    }

    #[test]
    fn new_writer_id_shape() {
        let id = new_session_writer_id();
        let parts: Vec<&str> = id.split('-').collect();
        assert!(parts.len() >= 3, "{id}");
        let nonce = parts[parts.len() - 1];
        assert_eq!(nonce.len(), 16, "{id}");
        assert!(
            nonce
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{id}"
        );
        let pid: u32 = parts[parts.len() - 2].parse().expect("pid segment");
        assert_eq!(pid, std::process::id());
    }

    #[test]
    fn session_writer_id_is_stable_per_process() {
        assert_eq!(session_writer_id(), session_writer_id());
        assert!(!session_writer_id().is_empty());
    }
}
