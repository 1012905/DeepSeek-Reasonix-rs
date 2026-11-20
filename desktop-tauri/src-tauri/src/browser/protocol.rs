// The browser the kernel drives when it runs inside this window. The kernel
// speaks the debugging protocol to what it takes for a browser; this answers
// the browser-level half itself — targets are views this process owns — and
// hands each page's commands to that page's own debugger.
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

pub const PAGE_SESSION: &str = "page:";

#[derive(Clone)]
pub struct Hooks {
    pub post: mpsc::Sender<Value>,
    pub notify: mpsc::Sender<Notify>,
    pub conn: String,
    pub target: String,
}

impl Hooks {
    pub fn event(&self, method: &str, params: Value) {
        let _ = self.post.send(json!({
            "conn": self.conn,
            "message": { "method": method, "params": params, "sessionId": format!("{PAGE_SESSION}{}", self.target) },
        }));
    }

    pub fn closed(&self) {
        let _ = self.notify.send(Notify::Closed {
            conn: self.conn.clone(),
            target: self.target.clone(),
        });
    }

    pub fn popup(&self, url: &str) {
        let _ = self.notify.send(Notify::Popup {
            conn: self.conn.clone(),
            opener: self.target.clone(),
            url: url.to_string(),
        });
    }

    pub fn download(&self, url: &str, filename: &str, frame: &str) {
        let _ = self.post.send(json!({
            "conn": self.conn,
            "message": { "method": "Browser.downloadWillBegin", "params": {
                "frameId": frame, "url": url, "suggestedFilename": filename,
                "guid": format!("{}-{}", self.target, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)),
            } },
        }));
    }
}

pub enum Notify {
    Closed {
        conn: String,
        target: String,
    },
    Popup {
        conn: String,
        opener: String,
        url: String,
    },
}

pub enum Incoming {
    Frame(Value),
    Notify(Notify),
    Drop,
}

/// What the window can do with a page; the protocol decides only what is said.
pub trait Views: Send + Sync {
    fn create(&self, target: &str, partition: &str, url: &str, hooks: Hooks) -> Result<(), String>;
    fn send(&self, target: &str, method: &str, params: Value) -> Result<Value, String>;
    fn close(&self, target: &str);
    fn main_frame(&self, target: &str) -> String;
    fn activate(&self, target: &str);
}

struct Conn {
    partition: String,
    targets: HashSet<String>,
}

pub struct Protocol {
    conns: HashMap<String, Conn>,
    views: Arc<dyn Views>,
    post: mpsc::Sender<Value>,
    notify: mpsc::Sender<Notify>,
}

impl Protocol {
    pub fn new(
        views: Arc<dyn Views>,
        post: mpsc::Sender<Value>,
        notify: mpsc::Sender<Notify>,
    ) -> Self {
        Self {
            conns: HashMap::new(),
            views,
            post,
            notify,
        }
    }

    pub fn receive(&mut self, frame: Value) {
        let Some(conn_id) = frame
            .get("conn")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        if let Some(partition) = frame.get("open").and_then(Value::as_str) {
            self.conns.insert(
                conn_id,
                Conn {
                    partition: partition.to_string(),
                    targets: HashSet::new(),
                },
            );
            return;
        }
        let Some(conn) = self.conns.get(&conn_id) else {
            return;
        };
        if frame.get("close").is_some() {
            let targets: Vec<String> = conn.targets.iter().cloned().collect();
            for target in targets {
                self.views.close(&target);
            }
            self.conns.remove(&conn_id);
            return;
        }
        if let Some(message) = frame.get("message") {
            let partition = conn.partition.clone();
            let known: HashSet<String> = conn.targets.clone();
            self.command(&conn_id, &partition, &known, message.clone());
        }
    }

    /// The stream ended: the kernel has already ended its connections, and
    /// their pages go with them.
    pub fn drop_all(&mut self) {
        let conns: Vec<(String, Vec<String>)> = self
            .conns
            .drain()
            .map(|(id, conn)| (id, conn.targets.into_iter().collect()))
            .collect();
        for (_, targets) in conns {
            for target in targets {
                self.views.close(&target);
            }
        }
    }

    pub fn notify(&mut self, note: Notify) {
        match note {
            Notify::Closed { conn, target } => {
                let Some(conn_state) = self.conns.get_mut(&conn) else {
                    return;
                };
                if !conn_state.targets.remove(&target) {
                    return;
                }
                let _ = self.post.send(json!({
                    "conn": conn,
                    "message": { "method": "Target.targetDestroyed", "params": { "targetId": target } },
                }));
            }
            Notify::Popup { conn, opener, url } => {
                let Some(partition) = self.conns.get(&conn).map(|c| c.partition.clone()) else {
                    return;
                };
                if let Ok(target) = self.open(&conn, &partition, &url) {
                    let _ = self.post.send(json!({
                        "conn": conn,
                        "message": { "method": "Target.targetCreated", "params": {
                            "targetInfo": { "targetId": target, "type": "page", "openerId": opener, "url": url },
                        } },
                    }));
                }
            }
        }
    }

    fn reply(&self, conn: &str, id: &Value, result: Value) {
        let _ = self
            .post
            .send(json!({ "conn": conn, "message": { "id": id, "result": result } }));
    }

    fn refuse(&self, conn: &str, id: &Value, message: String, code: i64) {
        let _ = self.post.send(json!({ "conn": conn, "message": { "id": id, "error": { "code": code, "message": message } } }));
    }

    fn command(&mut self, conn_id: &str, partition: &str, known: &HashSet<String>, raw: Value) {
        let msg: Value = match raw {
            Value::String(text) => match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) => return,
            },
            value => value,
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        if let Some(session) = msg.get("sessionId").and_then(Value::as_str) {
            let target = session.strip_prefix(PAGE_SESSION).unwrap_or("").to_string();
            if !known.contains(&target) {
                return self.refuse(
                    conn_id,
                    &id,
                    format!("no page for session {session}"),
                    -32000,
                );
            }
            if method == "Page.bringToFront" {
                self.views.activate(&target);
                return self.reply(conn_id, &id, json!({}));
            }
            let views = self.views.clone();
            let post = self.post.clone();
            let conn = conn_id.to_string();
            std::thread::spawn(move || {
                let answer = views.send(&target, &method, params);
                let message = match answer {
                    Ok(result) => json!({ "id": id, "result": result }),
                    Err(err) => json!({ "id": id, "error": { "code": -32000, "message": err } }),
                };
                let _ = post.send(json!({ "conn": conn, "message": message }));
            });
            return;
        }
        match method.as_str() {
            "Browser.getVersion" => self.reply(
                conn_id,
                &id,
                json!({ "protocolVersion": "1.3", "product": "Reasonix Studio" }),
            ),
            "Target.setDiscoverTargets" | "Browser.setDownloadBehavior" => {
                self.reply(conn_id, &id, json!({}))
            }
            "Target.createTarget" => {
                let url = params
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("about:blank");
                match self.open(conn_id, partition, url) {
                    Ok(target) => self.reply(conn_id, &id, json!({ "targetId": target })),
                    Err(err) => self.refuse(conn_id, &id, err, -32000),
                }
            }
            "Target.attachToTarget" => {
                let target = params.get("targetId").and_then(Value::as_str).unwrap_or("");
                if !known.contains(target) {
                    return self.refuse(conn_id, &id, format!("no target {target}"), -32000);
                }
                self.reply(
                    conn_id,
                    &id,
                    json!({ "sessionId": format!("{PAGE_SESSION}{target}") }),
                );
            }
            "Target.closeTarget" => {
                let target = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let had = self
                    .conns
                    .get_mut(conn_id)
                    .is_some_and(|c| c.targets.remove(&target));
                if had {
                    self.views.close(&target);
                }
                self.reply(conn_id, &id, json!({ "success": had }));
            }
            "Browser.close" => {
                if let Some(conn) = self.conns.get_mut(conn_id) {
                    let targets: Vec<String> = conn.targets.drain().collect();
                    for target in targets {
                        self.views.close(&target);
                    }
                }
                self.reply(conn_id, &id, json!({}));
            }
            _ => self.refuse(
                conn_id,
                &id,
                format!("{method} is not available in this window"),
                -32601,
            ),
        }
    }

    fn open(&mut self, conn_id: &str, partition: &str, url: &str) -> Result<String, String> {
        let target = uuid::Uuid::new_v4().to_string();
        let hooks = Hooks {
            post: self.post.clone(),
            notify: self.notify.clone(),
            conn: conn_id.to_string(),
            target: target.clone(),
        };
        self.views.create(&target, partition, url, hooks)?;
        if let Some(conn) = self.conns.get_mut(conn_id) {
            conn.targets.insert(target.clone());
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeViews {
        created: Mutex<Vec<(String, String, String)>>,
        sent: Mutex<Vec<(String, String)>>,
        closed: Mutex<Vec<String>>,
    }

    impl Views for FakeViews {
        fn create(
            &self,
            target: &str,
            partition: &str,
            url: &str,
            _hooks: Hooks,
        ) -> Result<(), String> {
            self.created
                .lock()
                .expect("lock")
                .push((target.into(), partition.into(), url.into()));
            Ok(())
        }
        fn send(&self, target: &str, method: &str, _params: Value) -> Result<Value, String> {
            self.sent
                .lock()
                .expect("lock")
                .push((target.into(), method.into()));
            Ok(json!({ "ok": true }))
        }
        fn close(&self, target: &str) {
            self.closed.lock().expect("lock").push(target.into());
        }
        fn main_frame(&self, _target: &str) -> String {
            "frame-1".into()
        }
        fn activate(&self, _target: &str) {}
    }

    fn rig() -> (
        Protocol,
        Arc<FakeViews>,
        mpsc::Receiver<Value>,
        mpsc::Receiver<Notify>,
    ) {
        let views = Arc::new(FakeViews::default());
        let (post_tx, post_rx) = mpsc::channel();
        let (notify_tx, notify_rx) = mpsc::channel();
        let protocol = Protocol::new(views.clone(), post_tx, notify_tx);
        (protocol, views, post_rx, notify_rx)
    }

    fn drained(rx: &mpsc::Receiver<Value>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(value) = rx.try_recv() {
            out.push(value);
        }
        out
    }

    #[test]
    fn a_created_target_attaches_under_a_page_session() {
        let (mut protocol, views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 1, "method": "Target.createTarget", "params": { "url": "https://example.com" } } }));
        let frames = drained(&post);
        let target = frames[0]["message"]["result"]["targetId"]
            .as_str()
            .expect("id")
            .to_string();
        assert_eq!(views.created.lock().expect("lock")[0].1, "p1");
        protocol.receive(json!({ "conn": "c1", "message": { "id": 2, "method": "Target.attachToTarget", "params": { "targetId": target } } }));
        let frames = drained(&post);
        assert_eq!(
            frames[0]["message"]["result"]["sessionId"],
            format!("page:{target}")
        );
    }

    #[test]
    fn browser_level_methods_answer_without_a_page() {
        let (mut protocol, _views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(
            json!({ "conn": "c1", "message": { "id": 7, "method": "Browser.getVersion" } }),
        );
        let frames = drained(&post);
        assert_eq!(frames[0]["message"]["result"]["product"], "Reasonix Studio");
        protocol
            .receive(json!({ "conn": "c1", "message": { "id": 8, "method": "Page.navigate" } }));
        let frames = drained(&post);
        assert_eq!(frames[0]["message"]["error"]["code"], -32601);
    }

    #[test]
    fn a_page_command_reaches_the_view_and_answers() {
        let (mut protocol, views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 1, "method": "Target.createTarget", "params": {} } }));
        let target = drained(&post)[0]["message"]["result"]["targetId"]
            .as_str()
            .expect("id")
            .to_string();
        protocol.receive(json!({ "conn": "c1", "message": { "id": 2, "sessionId": format!("page:{target}"), "method": "Runtime.evaluate", "params": { "expression": "1" } } }));
        for _ in 0..100 {
            if !views.sent.lock().expect("lock").is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(views.sent.lock().expect("lock")[0].1, "Runtime.evaluate");
        let frames = drained(&post);
        assert_eq!(frames[0]["message"]["result"]["ok"], true);
    }

    #[test]
    fn an_unknown_session_is_refused() {
        let (mut protocol, _views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 3, "sessionId": "page:ghost", "method": "Page.reload" } }));
        let frames = drained(&post);
        assert!(
            frames[0]["message"]["error"]["message"]
                .as_str()
                .expect("msg")
                .contains("no page for session")
        );
    }

    #[test]
    fn closing_a_connection_buries_its_pages() {
        let (mut protocol, views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 1, "method": "Target.createTarget", "params": {} } }));
        let target = drained(&post)[0]["message"]["result"]["targetId"]
            .as_str()
            .expect("id")
            .to_string();
        protocol.receive(json!({ "conn": "c1", "close": true }));
        assert_eq!(views.closed.lock().expect("lock")[0], target);
        protocol.receive(
            json!({ "conn": "c1", "message": { "id": 2, "method": "Browser.getVersion" } }),
        );
        assert!(drained(&post).is_empty());
    }

    #[test]
    fn a_drop_buries_every_connection() {
        let (mut protocol, views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c2", "open": "p2" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 1, "method": "Target.createTarget", "params": {} } }));
        protocol.receive(json!({ "conn": "c2", "message": { "id": 1, "method": "Target.createTarget", "params": {} } }));
        let _ = drained(&post);
        protocol.drop_all();
        assert_eq!(views.closed.lock().expect("lock").len(), 2);
    }

    #[test]
    fn a_death_reported_once_is_told_once() {
        let (mut protocol, _views, post, _notify) = rig();
        protocol.receive(json!({ "conn": "c1", "open": "p1" }));
        protocol.receive(json!({ "conn": "c1", "message": { "id": 1, "method": "Target.createTarget", "params": {} } }));
        let target = drained(&post)[0]["message"]["result"]["targetId"]
            .as_str()
            .expect("id")
            .to_string();
        protocol.notify(Notify::Closed {
            conn: "c1".into(),
            target: target.clone(),
        });
        protocol.notify(Notify::Closed {
            conn: "c1".into(),
            target,
        });
        let frames = drained(&post);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["message"]["method"], "Target.targetDestroyed");
    }
}
