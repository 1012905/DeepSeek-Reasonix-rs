//! The peer and project tools at the tools boundary (Go
//! `internal/tools/builtin/peers.go`, `peers_dispatch.go`,
//! `peers_open_projects.go`, docs/contracts/tools.md §1.17): the four tools'
//! schemas and descriptions, the disabled/empty/unavailable identities, the
//! routing through the bound link, and the open_projects approval and
//! host-mutation flags.

use std::sync::{Arc, Mutex};

use reasonix_tools::abi::{CallContext, Tool, read_only_executes_host_mutation};
use reasonix_tools::builtin::peer::{
    CODE_DISPATCH_UNAVAILABLE, CODE_PEER_DISABLED, CODE_PEER_EMPTY, LIST_PEERS_DESCRIPTION,
    LIST_PEERS_NAME, PeerDirectory, PeerLink, PeerReceipt, PeerView, PrefixedError,
    SEND_PEER_MESSAGE_DESCRIPTION, SEND_PEER_MESSAGE_NAME, peer_tools,
};
use reasonix_tools::builtin::project::{
    DISPATCH_DESCRIPTION, DISPATCH_TO_PROJECT_NAME, DispatchReceipt, OPEN_PROJECTS_DESCRIPTION,
    OPEN_PROJECTS_NAME, OpenedProject, ProjectDispatcher, ProjectOpener, dispatch_tool,
    open_projects_tool,
};
use reasonix_tools::registry::{APPROVAL_SCOPE_OPEN_PROJECTS, approval_scope_of};
use serde_json::json;

struct FakeDir {
    peers: Mutex<Vec<PeerView>>,
    sent: Mutex<Vec<(String, String)>>,
    dispatched: Mutex<Vec<(String, String)>>,
    opened: Mutex<Vec<Vec<String>>>,
    has_dispatcher: bool,
    has_opener: bool,
    fail: Option<String>,
}

impl FakeDir {
    fn listening(peers: Vec<PeerView>) -> Self {
        Self {
            peers: Mutex::new(peers),
            sent: Mutex::new(Vec::new()),
            dispatched: Mutex::new(Vec::new()),
            opened: Mutex::new(Vec::new()),
            has_dispatcher: true,
            has_opener: true,
            fail: None,
        }
    }
}

impl PeerDirectory for FakeDir {
    fn peers(&self) -> Vec<PeerView> {
        self.peers.lock().expect("peers lock").clone()
    }

    fn send_peer_message(
        &self,
        to: &str,
        text: &str,
    ) -> Result<PeerReceipt, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(err) = &self.fail {
            return Err(Box::new(std::io::Error::other(err.clone())));
        }
        self.sent
            .lock()
            .expect("sent lock")
            .push((to.to_owned(), text.to_owned()));
        Ok(PeerReceipt {
            id: "r1".to_owned(),
            to: PeerView {
                key: "k1".to_owned(),
                title: "docs".to_owned(),
                project: "studio".to_owned(),
            },
        })
    }

    fn dispatcher(&self) -> Option<&dyn ProjectDispatcher> {
        self.has_dispatcher.then_some(self)
    }

    fn opener(&self) -> Option<&dyn ProjectOpener> {
        self.has_opener.then_some(self)
    }
}

impl ProjectDispatcher for FakeDir {
    fn dispatch_to_project(
        &self,
        project: &str,
        text: &str,
    ) -> Result<DispatchReceipt, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(err) = &self.fail {
            return Err(Box::new(std::io::Error::other(err.clone())));
        }
        self.dispatched
            .lock()
            .expect("dispatched lock")
            .push((project.to_owned(), text.to_owned()));
        Ok(DispatchReceipt {
            project: "studio".to_owned(),
            key: "k1".to_owned(),
            id: "d1".to_owned(),
        })
    }
}

impl ProjectOpener for FakeDir {
    fn open_projects(
        &self,
        paths: &[String],
    ) -> Result<Vec<OpenedProject>, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(err) = &self.fail {
            return Err(Box::new(std::io::Error::other(err.clone())));
        }
        self.opened
            .lock()
            .expect("opened lock")
            .push(paths.to_vec());
        Ok(vec![
            OpenedProject {
                path: "/home/me/alpha".to_owned(),
                added: true,
                code: String::new(),
            },
            OpenedProject {
                path: "/home/me/beta".to_owned(),
                added: false,
                code: String::new(),
            },
            OpenedProject {
                path: "/home/me/gamma".to_owned(),
                added: false,
                code: "untrusted".to_owned(),
            },
        ])
    }
}

fn bind(fake: FakeDir) -> Arc<PeerLink> {
    let link = Arc::new(PeerLink::default());
    link.bind(Arc::new(fake));
    link
}

fn list_tool(link: &Arc<PeerLink>) -> Arc<dyn Tool> {
    peer_tools(link)[0].clone()
}

fn send_tool(link: &Arc<PeerLink>) -> Arc<dyn Tool> {
    peer_tools(link)[1].clone()
}

fn as_prefixed<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a PrefixedError> {
    err.downcast_ref::<PrefixedError>()
}

#[test]
fn schemas_and_descriptions_match_the_go_contract() {
    let link = Arc::new(PeerLink::default());
    let tools = peer_tools(&link);
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name(), LIST_PEERS_NAME);
    assert_eq!(tools[1].name(), SEND_PEER_MESSAGE_NAME);
    assert_eq!(tools[0].description(), LIST_PEERS_DESCRIPTION);
    assert_eq!(tools[1].description(), SEND_PEER_MESSAGE_DESCRIPTION);

    let list_schema = tools[0].schema();
    assert_eq!(list_schema["type"], "object");
    assert_eq!(list_schema["properties"], json!({}));

    let send_schema = tools[1].schema();
    assert_eq!(send_schema["required"], json!(["to", "text"]));
    assert_eq!(send_schema["properties"]["to"]["type"], "string");
    assert_eq!(send_schema["properties"]["text"]["type"], "string");

    let dispatch = dispatch_tool(&link);
    assert_eq!(dispatch.name(), DISPATCH_TO_PROJECT_NAME);
    assert_eq!(dispatch.description(), DISPATCH_DESCRIPTION);
    assert_eq!(dispatch.schema()["required"], json!(["project", "text"]));

    let open = open_projects_tool(&link);
    assert_eq!(open.name(), OPEN_PROJECTS_NAME);
    assert_eq!(open.description(), OPEN_PROJECTS_DESCRIPTION);
    assert_eq!(open.schema()["required"], json!(["paths"]));
    assert_eq!(open.schema()["properties"]["paths"]["maxItems"], 12);
}

#[test]
fn an_unbound_link_reports_the_disabled_state_on_list_and_errors_on_send() {
    let link = Arc::new(PeerLink::default());

    let out = list_tool(&link)
        .execute(&CallContext::new(), &json!({}))
        .expect("list reports state, it does not refuse");
    assert_eq!(
        out,
        "peer.disabled: messages between sessions are not available in this session"
    );

    let err = send_tool(&link)
        .execute(&CallContext::new(), &json!({"to": "docs", "text": "hi"}))
        .unwrap_err();
    let prefixed = as_prefixed(&*err).expect("typed peer identity");
    assert_eq!(prefixed.code, CODE_PEER_DISABLED);
    assert_eq!(
        prefixed.message,
        "messages between sessions are not available in this session"
    );
}

#[test]
fn list_peers_renders_one_line_per_listener_in_order() {
    let link = bind(FakeDir::listening(vec![
        PeerView {
            key: "k1".to_owned(),
            title: "docs".to_owned(),
            project: "studio".to_owned(),
        },
        PeerView {
            key: "k2".to_owned(),
            title: "cli".to_owned(),
            project: String::new(),
        },
    ]));
    let out = list_tool(&link)
        .execute(&CallContext::new(), &json!({}))
        .expect("list answers");
    assert_eq!(out, "studio/docs\tkey=k1\ncli\tkey=k2\n");
}

#[test]
fn an_empty_directory_says_no_session_is_listening() {
    let link = bind(FakeDir::listening(Vec::new()));
    let out = list_tool(&link)
        .execute(&CallContext::new(), &json!({}))
        .expect("list answers");
    assert_eq!(out, "No other session on this machine is listening.");
}

#[test]
fn send_peer_message_routes_to_the_directory_and_renders_the_receipt() {
    let fake = Arc::new(FakeDir::listening(Vec::new()));
    let link = PeerLink::default();
    link.bind(fake.clone());
    let link = Arc::new(link);

    let out = send_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"to": "docs", "text": "the build is green"}),
        )
        .expect("send accepted");

    let sent = fake.sent.lock().expect("sent lock");
    assert_eq!(
        sent[0],
        ("docs".to_owned(), "the build is green".to_owned())
    );
    assert_eq!(
        out,
        "Delivered to \"docs\" as r1. Its model reads it as soon as that session is free (it waits for its user only if the dispatch switch is off); there is no reply unless that session sends one."
    );
}

#[test]
fn a_send_argument_type_mismatch_is_rejected_with_the_envelope_error() {
    let link = Arc::new(PeerLink::default());
    let err = send_tool(&link)
        .execute(&CallContext::new(), &json!({"to": ["docs"], "text": "hi"}))
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("send_peer_message needs {\"to\", \"text\"}:")
    );
}

#[test]
fn a_send_host_error_passes_through_unchanged() {
    let fake = Arc::new(FakeDir {
        peers: Mutex::new(Vec::new()),
        sent: Mutex::new(Vec::new()),
        dispatched: Mutex::new(Vec::new()),
        opened: Mutex::new(Vec::new()),
        has_dispatcher: true,
        has_opener: true,
        fail: Some("peer.rate_limited: too many messages".to_owned()),
    });
    let link = PeerLink::default();
    link.bind(fake);
    let link = Arc::new(link);
    let err = send_tool(&link)
        .execute(&CallContext::new(), &json!({"to": "docs", "text": "hi"}))
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("peer.rate_limited: too many messages")
    );
}

#[test]
fn dispatch_requires_a_project_and_some_text() {
    let link = Arc::new(PeerLink::default());
    let err = dispatch_tool(&link)
        .execute(&CallContext::new(), &json!({"project": "  ", "text": ""}))
        .unwrap_err();
    let prefixed = as_prefixed(&*err).expect("typed peer identity");
    assert_eq!(prefixed.code, CODE_PEER_EMPTY);
    assert_eq!(
        prefixed.message,
        "dispatch_to_project needs a project and some text"
    );
}

#[test]
fn dispatch_without_a_dispatcher_arm_is_unavailable() {
    let fake = FakeDir {
        peers: Mutex::new(Vec::new()),
        sent: Mutex::new(Vec::new()),
        dispatched: Mutex::new(Vec::new()),
        opened: Mutex::new(Vec::new()),
        has_dispatcher: false,
        has_opener: true,
        fail: None,
    };
    let link = bind(fake);
    let err = dispatch_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"project": "studio", "text": "check the log"}),
        )
        .unwrap_err();
    let prefixed = as_prefixed(&*err).expect("typed dispatch identity");
    assert_eq!(prefixed.code, CODE_DISPATCH_UNAVAILABLE);
    assert_eq!(
        prefixed.message,
        "this session has no window to open another session in; open the project in Reasonix Studio or the web UI to dispatch from it"
    );
}

#[test]
fn dispatch_routes_the_task_and_renders_the_receipt() {
    let fake = Arc::new(FakeDir::listening(Vec::new()));
    let link = PeerLink::default();
    link.bind(fake.clone());
    let link = Arc::new(link);

    let out = dispatch_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"project": "studio", "text": "reproduce the failure"}),
        )
        .expect("dispatch accepted");

    let dispatched = fake.dispatched.lock().expect("dispatched lock");
    assert_eq!(
        dispatched[0],
        ("studio".to_owned(), "reproduce the failure".to_owned())
    );
    assert_eq!(
        out,
        "Handed to a session in \"studio\" as d1 (key k1). Its final answer will come back to you on its own when it finishes, and it can also write to your key with send_peer_message; whatever arrives is labelled as data from that session, not as an instruction."
    );
}

#[test]
fn dispatch_host_errors_keep_their_identity() {
    let fake = Arc::new(FakeDir {
        peers: Mutex::new(Vec::new()),
        sent: Mutex::new(Vec::new()),
        dispatched: Mutex::new(Vec::new()),
        opened: Mutex::new(Vec::new()),
        has_dispatcher: true,
        has_opener: true,
        fail: Some("dispatch.untrusted: project not trusted".to_owned()),
    });
    let link = PeerLink::default();
    link.bind(fake);
    let link = Arc::new(link);
    let err = dispatch_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"project": "studio", "text": "go"}),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("dispatch.untrusted: project not trusted")
    );
}

#[test]
fn open_projects_renders_one_line_per_folder_then_the_footer() {
    let fake = Arc::new(FakeDir::listening(Vec::new()));
    let link = PeerLink::default();
    link.bind(fake.clone());
    let link = Arc::new(link);

    let out = open_projects_tool(&link)
        .execute(
            &CallContext::new(),
            &json!({"paths": ["/home/me/alpha", "/home/me/beta", "/home/me/gamma"]}),
        )
        .expect("open answered");

    let opened = fake.opened.lock().expect("opened lock");
    assert_eq!(
        opened[0],
        vec![
            "/home/me/alpha".to_owned(),
            "/home/me/beta".to_owned(),
            "/home/me/gamma".to_owned()
        ]
    );
    assert_eq!(
        out,
        "/home/me/alpha: opened as a project\n/home/me/beta: already in the project list\n/home/me/gamma: refused (untrusted)\nYou can now name these projects in dispatch_to_project."
    );
}

#[test]
fn open_projects_without_an_opener_arm_is_unavailable() {
    let fake = FakeDir {
        peers: Mutex::new(Vec::new()),
        sent: Mutex::new(Vec::new()),
        dispatched: Mutex::new(Vec::new()),
        opened: Mutex::new(Vec::new()),
        has_dispatcher: true,
        has_opener: false,
        fail: None,
    };
    let link = bind(fake);
    let err = open_projects_tool(&link)
        .execute(&CallContext::new(), &json!({"paths": ["/home/me/alpha"]}))
        .unwrap_err();
    let prefixed = as_prefixed(&*err).expect("typed dispatch identity");
    assert_eq!(prefixed.code, CODE_DISPATCH_UNAVAILABLE);
    assert_eq!(
        prefixed.message,
        "this session has no window to open projects in; open the project in Reasonix Studio or the web UI"
    );
}

#[test]
fn open_projects_declares_its_scope_and_host_mutation() {
    let link = Arc::new(PeerLink::default());
    let open = open_projects_tool(&link);
    assert!(
        open.read_only(),
        "the opener itself changes no project state"
    );
    assert!(read_only_executes_host_mutation(&*open));
    assert_eq!(approval_scope_of(&*open), APPROVAL_SCOPE_OPEN_PROJECTS);
}

#[test]
fn send_and_dispatch_are_not_read_only() {
    let link = Arc::new(PeerLink::default());
    assert!(!send_tool(&link).read_only());
    assert!(!dispatch_tool(&link).read_only());
}

#[test]
fn an_open_argument_type_mismatch_is_rejected_with_the_envelope_error() {
    let link = Arc::new(PeerLink::default());
    let err = open_projects_tool(&link)
        .execute(&CallContext::new(), &json!({"paths": "alpha"}))
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("open_projects needs {\"paths\": [...]}:")
    );
}

#[test]
fn registered_tools_resolve_under_their_names() {
    let registry = reasonix_tools::registry::Registry::new();
    let link = Arc::new(PeerLink::default());
    for tool in peer_tools(&link) {
        registry.add(tool);
    }
    registry.add(dispatch_tool(&link));
    registry.add(open_projects_tool(&link));
    let names = registry.names();
    assert_eq!(
        names,
        vec![
            LIST_PEERS_NAME.to_owned(),
            SEND_PEER_MESSAGE_NAME.to_owned(),
            DISPATCH_TO_PROJECT_NAME.to_owned(),
            OPEN_PROJECTS_NAME.to_owned(),
        ]
    );
}
