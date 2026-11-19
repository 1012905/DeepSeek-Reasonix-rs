//! The per-launch credential (docs/contracts/host.md §2.1): 32 CSPRNG bytes,
//! hex-encoded, never persisted and never derived from configuration.

pub const CREDENTIAL_BYTES: usize = 32;

pub fn launch_credential() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; CREDENTIAL_BYTES];
    getrandom::fill(&mut buf)?;
    Ok(hex_encode(&buf))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_is_64_hex_chars() {
        let token = launch_credential().unwrap();
        assert_eq!(token.len(), CREDENTIAL_BYTES * 2);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn credentials_differ_per_mint() {
        assert_ne!(launch_credential().unwrap(), launch_credential().unwrap());
    }
}
