//! The peer session tools (Go `internal/tools/builtin/peers.go`): list the
//! other Reasonix sessions on this machine and send one of them text. They
//! exist only in a session that turned messages between sessions on; until
//! the controller binds a directory the tools answer that no session is
//! listening.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{CallContext, CapabilitySet, Tool, ToolResult};
use crate::builtin::project::{ProjectDispatcher, ProjectOpener};

/// `peer.disabled` — messages between sessions are not available in this
/// session.
pub const CODE_PEER_DISABLED: &str = "peer.disabled";
/// `peer.empty` — the dispatch call named neither a project nor any text.
pub const CODE_PEER_EMPTY: &str = "peer.empty";
/// `dispatch.unavailable` — this session has no window to reach another
/// session or project.
pub const CODE_DISPATCH_UNAVAILABLE: &str = "dispatch.unavailable";

pub const DISABLED_MESSAGE: &str = "messages between sessions are not available in this session";

/// A refusal identity Go carries as a plain prefixed error (`code: message`),
/// not a [`crate::refusal::Refusal`]. The code stays the classification a
/// reader matches on; the sentence is presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixedError {
    pub code: String,
    pub message: String,
}

impl PrefixedError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for PrefixedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PrefixedError {}

fn prefixed(code: &str, message: &str) -> ToolResult {
    Err(Box::new(PrefixedError::new(code, message)))
}

/// Another session on this machine that can be written to (Go `tool.PeerView`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeerView {
    pub key: String,
    pub title: String,
    pub project: String,
}

/// Says a message was handed to the receiver's spool, not read (Go
/// `tool.PeerReceipt`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeerReceipt {
    pub id: String,
    pub to: PeerView,
}

/// What the peer tools ask of the session that owns them (Go
/// `tool.PeerDirectory`). The dispatcher/opener arms are optional: the same
/// controller may implement only the message side.
pub trait PeerDirectory: Send + Sync {
    fn peers(&self) -> Vec<PeerView>;
    fn send_peer_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<PeerReceipt, Box<dyn std::error::Error + Send + Sync>>;

    /// The dispatch arm, when this directory's controller can hand a task to
    /// another project's session (Go `tool.ProjectDispatcher` type assertion).
    fn dispatcher(&self) -> Option<&dyn ProjectDispatcher> {
        None
    }

    /// The opener arm, when this directory's controller can add folders to
    /// the project list after a card (Go `tool.ProjectOpener` type assertion).
    fn opener(&self) -> Option<&dyn ProjectOpener> {
        None
    }
}

/// Joins the peer tools, built with the registry, to the controller that
/// exists only afterwards (Go `tool.PeerLink`). Until the controller binds it
/// the tools answer that no session is listening.
#[derive(Default)]
pub struct PeerLink {
    dir: Mutex<Option<Arc<dyn PeerDirectory>>>,
}

impl PeerLink {
    /// Binds the directory the tools call through.
    pub fn bind(&self, dir: Arc<dyn PeerDirectory>) {
        *self.dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    }

    /// The bound directory, if any.
    pub fn directory(&self) -> Option<Arc<dyn PeerDirectory>> {
        self.dir.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// `list_peers`: the other sessions on this machine listening for messages.
pub const LIST_PEERS_NAME: &str = "list_peers";
/// `send_peer_message`.
pub const SEND_PEER_MESSAGE_NAME: &str = "send_peer_message";

/// Go `listPeers.Description()`, verbatim.
pub const LIST_PEERS_DESCRIPTION: &str = "List the other Reasonix sessions on this machine that are listening for messages. Use the name or key it prints as the target of send_peer_message.";
/// Go `sendPeerMessage.Description()`, verbatim.
pub const SEND_PEER_MESSAGE_DESCRIPTION: &str = "Send text to another Reasonix session on this machine, named by list_peers. While the user's dispatch switch is on (the default) its model reads it as soon as that session is free, labelled as coming from you; with the switch off it waits there for its user to approve it. Sending grants no permission and is not a way to run anything there: it is for passing information. Do not include secrets.";

const LIST_PEERS_SCHEMA: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;
const SEND_PEER_MESSAGE_SCHEMA: &str = r#"{"type":"object","properties":{"to":{"type":"string","description":"The session's key, or its title, or project/title, as list_peers prints it"},"text":{"type":"string","description":"What to tell that session"}},"required":["to","text"],"additionalProperties":false}"#;

/// The list tool (Go `listPeers`).
pub struct ListPeersTool {
    link: Arc<PeerLink>,
}

/// The send tool (Go `sendPeerMessage`).
pub struct SendPeerMessageTool {
    link: Arc<PeerLink>,
}

/// Go `PeerTools`: the message-side pair a composition root registers.
pub fn peer_tools(link: &Arc<PeerLink>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ListPeersTool { link: link.clone() }),
        Arc::new(SendPeerMessageTool { link: link.clone() }),
    ]
}

#[derive(Deserialize, Default)]
struct SendArgs {
    #[serde(default)]
    to: String,
    #[serde(default)]
    text: String,
}

impl Tool for ListPeersTool {
    fn name(&self) -> &str {
        LIST_PEERS_NAME
    }

    fn description(&self) -> &str {
        LIST_PEERS_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(LIST_PEERS_SCHEMA).expect("the list_peers schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        let Some(dir) = self.link.directory() else {
            // A missing directory answers with the disabled sentence and no
            // error: this arm reports state, it does not refuse a call.
            return Ok(format!("{CODE_PEER_DISABLED}: {DISABLED_MESSAGE}"));
        };
        let peers = dir.peers();
        if peers.is_empty() {
            return Ok("No other session on this machine is listening.".to_owned());
        }
        let mut b = String::new();
        for p in &peers {
            b.push_str(&format!("{}\tkey={}\n", display_name(p), p.key));
        }
        Ok(b)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn display_name(p: &PeerView) -> String {
    if p.project.is_empty() {
        p.title.clone()
    } else {
        format!("{}/{}", p.project, p.title)
    }
}

impl Tool for SendPeerMessageTool {
    fn name(&self) -> &str {
        SEND_PEER_MESSAGE_NAME
    }

    fn description(&self) -> &str {
        SEND_PEER_MESSAGE_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(SEND_PEER_MESSAGE_SCHEMA)
            .expect("the send_peer_message schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: SendArgs = serde_json::from_value(args.clone()).map_err(|e| {
            std::io::Error::other(format!("send_peer_message needs {{\"to\", \"text\"}}: {e}"))
        })?;
        let Some(dir) = self.link.directory() else {
            return prefixed(CODE_PEER_DISABLED, DISABLED_MESSAGE);
        };
        let receipt = dir.send_peer_message(&parsed.to, &parsed.text)?;
        Ok(format!(
            "Delivered to {:?} as {}. Its model reads it as soon as that session is free (it waits for its user only if the dispatch switch is off); there is no reply unless that session sends one.",
            receipt.to.title, receipt.id
        ))
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
