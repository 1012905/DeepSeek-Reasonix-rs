//! Opaque message-chunk ids (message_ids.go:15-48; run_loop.go:416-419).
//!
//! A live message is one sampling attempt, so a retried attempt earns a new
//! message; a replayed history message is numbered per session. The prefix is
//! drawn once per session open so ids from before a restart cannot collide with
//! new ones. The runtime supplies the random 6-byte prefix; this pure type keeps
//! the rule and a fixed-prefix constructor tests can predict.

/// Prefix length: six random bytes encoded as twelve lowercase hex chars.
pub const PREFIX_HEX_LEN: usize = 12;

/// Per-session message id allocator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageIds {
    prefix: String,
    replay: u64,
}

impl MessageIds {
    /// Builds an allocator around an explicit prefix. The prefix must be twelve
    /// lowercase hex characters; anything else is rejected so a malformed
    /// prefix cannot ride the wire.
    pub fn with_prefix(prefix: &str) -> Option<Self> {
        if prefix.len() != PREFIX_HEX_LEN {
            return None;
        }
        if !prefix
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return None;
        }
        Some(Self {
            prefix: prefix.to_owned(),
            replay: 0,
        })
    }

    /// The session-open prefix, for diagnostics and outbound chunk headers.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Live id for the attempt currently streaming. The attempt id is
    /// `sa-<attempt 1-based>-<process seq>`; the seq breaks same-tick ties on
    /// coarse clocks. Attempt `0` means no active attempt, so no id rides the
    /// chunk (message_ids.go:31-36; run_loop.go:416-419).
    pub fn live(&self, attempt: u64, seq: u64) -> Option<String> {
        if attempt == 0 {
            return None;
        }
        Some(format!("{}-sa-{attempt}-{seq}", self.prefix))
    }

    /// Replayed-history id; the counter is one-based and advances per call
    /// (message_ids.go:39-41).
    pub fn replayed(&mut self) -> String {
        self.replay += 1;
        format!("{}-r{}", self.prefix, self.replay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_id_requires_an_active_attempt() {
        let ids = MessageIds::with_prefix("a1b2c3d4e5f6").unwrap();
        assert_eq!(ids.live(0, 1), None);
        assert_eq!(ids.live(1, 1).unwrap(), "a1b2c3d4e5f6-sa-1-1");
    }

    #[test]
    fn three_id_shapes_match_the_reference_examples() {
        // Matrix A §1.6: `<12hex>-sa-1-1`, `<12hex>-sa-2-3`, `<12hex>-r1`.
        let mut ids = MessageIds::with_prefix("001122334455").unwrap();
        assert_eq!(ids.live(1, 1).unwrap(), "001122334455-sa-1-1");
        assert_eq!(ids.live(2, 3).unwrap(), "001122334455-sa-2-3");
        assert_eq!(ids.replayed(), "001122334455-r1");
    }

    #[test]
    fn replayed_ids_are_consecutively_numbered() {
        let mut ids = MessageIds::with_prefix("001122334455").unwrap();
        assert_eq!(ids.replayed(), "001122334455-r1");
        assert_eq!(ids.replayed(), "001122334455-r2");
        assert_eq!(ids.replayed(), "001122334455-r3");
    }

    #[test]
    fn prefix_must_be_twelve_lowercase_hex() {
        assert!(MessageIds::with_prefix("a1b2c3d4e5f").is_none());
        assert!(MessageIds::with_prefix("a1b2c3d4e5fG").is_none());
        assert!(MessageIds::with_prefix("A1B2C3D4E5F6").is_none());
        assert!(MessageIds::with_prefix("a1b2c3d4e5f6").is_some());
    }
}
