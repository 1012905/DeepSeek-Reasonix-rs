//! Inline chain-of-thought splitting (Go: `openai/think.go`). MiniMax-M3
//! streams its reasoning inside a leading `<think>…</think>` block instead of
//! populating `reasoning_content`.

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ThinkState {
    #[default]
    Probe,
    Inside,
    Passthrough,
}

/// Peels a leading `<think>…</think>` block out of the content stream into
/// reasoning text. It only arms on a `<think>` at the very start of the turn,
/// so an answer that merely mentions the tag is never hijacked.
#[derive(Debug, Default)]
pub struct ThinkSplitter {
    state: ThinkState,
    buf: String,
}

impl ThinkSplitter {
    /// Feeds one content delta; returns (reasoning, text) pieces.
    pub fn push(&mut self, s: &str) -> (String, String) {
        match self.state {
            ThinkState::Passthrough => (String::new(), s.to_owned()),
            ThinkState::Inside => self.scan_close(s),
            ThinkState::Probe => {
                self.buf.push_str(s);
                let trimmed = self
                    .buf
                    .trim_start_matches([' ', '\t', '\r', '\n'])
                    .to_owned();
                if trimmed.len() < THINK_OPEN.len() {
                    if THINK_OPEN.starts_with(&trimmed) {
                        return (String::new(), String::new());
                    }
                    return (String::new(), self.drain_passthrough());
                }
                if let Some(rest) = trimmed.strip_prefix(THINK_OPEN) {
                    self.state = ThinkState::Inside;
                    self.buf.clear();
                    return self.scan_close(rest);
                }
                (String::new(), self.drain_passthrough())
            }
        }
    }

    /// Emits what is buffered when the stream ends mid-decision: an
    /// unterminated `<think>` block is reasoning; anything else is text.
    pub fn flush(&mut self) -> (String, String) {
        if self.buf.is_empty() {
            return (String::new(), String::new());
        }
        let out = std::mem::take(&mut self.buf);
        if self.state == ThinkState::Inside {
            return (out, String::new());
        }
        (String::new(), out)
    }

    fn scan_close(&mut self, s: &str) -> (String, String) {
        self.buf.push_str(s);
        if let Some(idx) = self.buf.find(THINK_CLOSE) {
            let reasoning = self.buf[..idx].to_owned();
            let rest = self.buf[idx + THINK_CLOSE.len()..]
                .trim_start_matches([' ', '\t', '\r', '\n'])
                .to_owned();
            self.buf.clear();
            self.state = ThinkState::Passthrough;
            return (reasoning, rest);
        }
        let keep = marker_suffix_len(&self.buf, THINK_CLOSE);
        let reasoning = self.buf[..self.buf.len() - keep].to_owned();
        let hold = self.buf[self.buf.len() - keep..].to_owned();
        self.buf = hold;
        (reasoning, String::new())
    }

    fn drain_passthrough(&mut self) -> String {
        self.state = ThinkState::Passthrough;
        std::mem::take(&mut self.buf)
    }
}

/// Length of the longest proper suffix of `s` that is a prefix of `marker` —
/// the tail held back in case the rest of the tag arrives in the next delta.
/// A match is ASCII bytes of the marker itself, so the boundary is safe.
fn marker_suffix_len(s: &str, marker: &str) -> usize {
    let max = marker.len().saturating_sub(1).min(s.len());
    for n in (1..=max).rev() {
        let at = s.len() - n;
        if !s.is_char_boundary(at) {
            continue;
        }
        if marker.as_bytes().starts_with(&s.as_bytes()[at..]) {
            return n;
        }
    }
    0
}
