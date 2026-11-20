//! Typed refusal identity and remedy (tools.md §1.16, §1.29).

use std::fmt;

/// `tool.cancelled_by_user` — the user stopped the call on its own while the
/// turn went on. Whatever the call had already done outside the host is
/// unknown.
pub const CANCELLED_BY_USER: &str = "tool.cancelled_by_user";

/// The setting whose change would lift a refusal. The producer that knows why
/// the call cannot proceed sets it; the model reads it rendered by the host
/// and never parses it out of a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remedy {
    /// `propose` where the model may ask for the change, `open` where only
    /// the user may make it.
    pub op: RemedyOp,
    pub setting: String,
    pub value: Option<serde_json::Value>,
}

/// Who may act on a remedy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemedyOp {
    Propose,
    Open,
}

impl fmt::Display for RemedyOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RemedyOp::Propose => "propose",
            RemedyOp::Open => "open",
        })
    }
}

impl fmt::Display for Remedy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tool:setting op={} id={}", self.op, self.setting)?;
        if self.op == RemedyOp::Propose
            && let Some(v) = &self.value
        {
            write!(f, " value={}", v)?;
        }
        Ok(())
    }
}

/// `Remedy` rendering used by [`Refusal`]'s Display.
pub fn remedy_string(remedy: &Remedy) -> String {
    remedy.to_string()
}

/// One host refusal. `code` is the identity and the only thing a reader may
/// classify on; `message` is presentation and may be reworded or translated
/// without changing what the refusal means. Whoever first knows why the call
/// cannot proceed owns the code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refusal {
    pub code: String,
    pub message: String,
    pub remedy: Option<Remedy>,
}

impl Refusal {
    pub fn code(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            remedy: None,
        }
    }

    /// Attaches the setting whose change lifts this refusal.
    pub fn with_remedy(mut self, op: RemedyOp, setting: impl Into<String>) -> Self {
        self.remedy = Some(Remedy {
            op,
            setting: setting.into(),
            value: None,
        });
        self
    }

    /// A refusal carrying neither identity nor words — what a tool that
    /// cannot say why it is unavailable returns.
    pub fn is_empty(&self) -> bool {
        self.code.is_empty() && self.message.trim().is_empty()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The code travels with the sentence because an identity the model
        // never sees is one it cannot cite back.
        let msg = self.message.trim().to_owned();
        let msg = if self.code.is_empty() {
            msg
        } else if msg.is_empty() {
            format!("(refusal: {})", self.code)
        } else {
            format!("{} (refusal: {})", msg, self.code)
        };
        let msg = match &self.remedy {
            Some(r) => format!("{} (fix: {})", msg, r),
            None => msg,
        };
        f.write_str(msg.trim())
    }
}

impl std::error::Error for Refusal {}

/// A host refusal raised by a tool enforcing its own execution policy — the
/// call never ran. Rendered as a blocked outcome rather than an execution
/// failure, so loop guards count it and frontends tell a refusal apart from
/// a completed call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BlockedError {
    pub message: String,
}

/// Returns a host refusal carrying the model-facing message.
pub fn blocked(message: impl Into<String>) -> BlockedError {
    BlockedError {
        message: message.into(),
    }
}

/// The message behind `err` when it is a host refusal.
pub fn blocked_message<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a str> {
    err.downcast_ref::<BlockedError>()
        .map(|b| b.message.as_str())
}
