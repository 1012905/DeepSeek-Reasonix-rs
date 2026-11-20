//! The one handshake line the shell parses before anything loads
//! (docs/contracts/host.md §2.1): compact JSON, field order version/origin/
//! token, one trailing newline.

use serde::Serialize;
use std::io::Write;

pub const HANDSHAKE_VERSION: i32 = 1;

#[derive(Debug, Serialize, PartialEq)]
pub struct Handshake {
    pub version: i32,
    pub origin: String,
    pub token: String,
}

pub fn announce<W: Write>(w: &mut W, b: &Handshake) -> std::io::Result<()> {
    let line = serde_json::to_string(b).map_err(|e| std::io::Error::other(e.to_string()))?;
    writeln!(w, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_line_has_frozen_field_order_and_trailing_newline() {
        let h = Handshake {
            version: HANDSHAKE_VERSION,
            origin: "http://127.0.0.1:5273".to_string(),
            token: "ab".repeat(32),
        };
        let mut buf = Vec::new();
        announce(&mut buf, &h).unwrap();
        let line = String::from_utf8(buf).unwrap();
        assert_eq!(
            line,
            "{\"version\":1,\"origin\":\"http://127.0.0.1:5273\",\"token\":\"".to_string()
                + &"ab".repeat(32)
                + "\"}\n"
        );
        assert_eq!(line.lines().count(), 1);
    }
}
