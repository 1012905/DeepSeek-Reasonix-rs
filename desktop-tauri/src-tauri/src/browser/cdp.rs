// The page-level debugging protocol. On Windows, WebView2 speaks CDP natively,
// so the kernel's page commands cross here unchanged; calls run on the thread
// that asked and answers come back on the main thread through a channel. Other
// platforms have no native debugger bridge yet and say so rather than pretend.

pub struct LoginAsk {
    pub id: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub realm: String,
}

#[cfg(windows)]
mod sys {
    use super::LoginAsk;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::{Webview, WebviewWindow};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2_2, ICoreWebView2_10,
        ICoreWebView2BasicAuthenticationRequestedEventArgs, ICoreWebView2Deferral,
    };
    use webview2_com::{
        BasicAuthenticationRequestedEventHandler, CallDevToolsProtocolMethodCompletedHandler,
        DevToolsProtocolEventReceivedEventHandler,
    };
    use windows::core::{HSTRING, Interface, PWSTR};

    const CDP_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn call(webview: &Webview, method: &str, params: &Value) -> Result<Value, String> {
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        let setup_tx = tx.clone();
        let method = method.to_string();
        let body = params.to_string();
        let call_method = method.clone();
        let posted = webview.with_webview(move |platform| {
            let started = (|| -> windows::core::Result<()> {
                // SAFETY: the controller and its core are live on the main thread
                // for this callback; the async call keeps its own references.
                unsafe {
                    let core = platform.controller().CoreWebView2()?;
                    let core2 = core.cast::<ICoreWebView2_2>()?;
                    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                        move |answer, json| {
                            let _ = tx.send(answer.map(|_| json).map_err(|e| e.to_string()));
                            Ok(())
                        },
                    ));
                    core2.CallDevToolsProtocolMethod(
                        &HSTRING::from(call_method),
                        &HSTRING::from(body),
                        &handler,
                    )?;
                }
                Ok(())
            })();
            if let Err(err) = started {
                let _ = setup_tx.send(Err(err.to_string()));
            }
        });
        if let Err(err) = posted {
            return Err(err.to_string());
        }
        let answer = rx
            .recv_timeout(CDP_TIMEOUT)
            .map_err(|_| format!("{method} did not answer"))??;
        if answer.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(&answer).map_err(|e| format!("reading the CDP answer: {e}"))
    }

    pub fn subscribe(
        webview: &Webview,
        event: &str,
        handler: impl Fn(Value) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let event = event.to_string();
        webview
            .with_webview(move |platform| {
                let _ = (|| -> windows::core::Result<()> {
                    // SAFETY: as in call; the receiver keeps the handler alive
                    // through its own COM reference for the webview's lifetime.
                    unsafe {
                        let core = platform.controller().CoreWebView2()?;
                        let core2 = core.cast::<ICoreWebView2_2>()?;
                        let receiver =
                            core2.GetDevToolsProtocolEventReceiver(&HSTRING::from(event))?;
                        let wrapped = DevToolsProtocolEventReceivedEventHandler::create(Box::new(
                            move |_sender, args| {
                                if let Some(args) = args {
                                    let mut json = PWSTR::null();
                                    args.ParameterObjectAsJson(&mut json)?;
                                    if let Ok(text) = json.to_string()
                                        && let Ok(value) = serde_json::from_str::<Value>(&text)
                                    {
                                        handler(value);
                                    }
                                }
                                Ok(())
                            },
                        ));
                        let mut token = 0i64;
                        receiver.add_DevToolsProtocolEventReceived(&wrapped, &mut token)?;
                    }
                    Ok(())
                })();
            })
            .map_err(|e| e.to_string())
    }

    struct PendingLogin {
        args: ICoreWebView2BasicAuthenticationRequestedEventArgs,
        deferral: ICoreWebView2Deferral,
    }

    // The deferral and its event args are STA objects: they are completed on the
    // thread that received the request, which is where every access happens.
    thread_local! {
        static LOGINS: std::cell::RefCell<HashMap<String, PendingLogin>> =
            std::cell::RefCell::new(HashMap::new());
    }

    /// Watches for the page or a proxy asking for a login. The ask goes to the
    /// window's own page; nothing is answered until the person answers.
    pub fn watch_logins(
        webview: &Webview,
        target: &str,
        emit: impl Fn(LoginAsk) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let _ = target;
        webview
            .with_webview(move |platform| {
                let _ = (|| -> windows::core::Result<()> {
                    // SAFETY: the core is live on the main thread; the handler is
                    // kept by the event source's COM reference.
                    unsafe {
                        let core: ICoreWebView2 = platform.controller().CoreWebView2()?;
                        let core10 = core.cast::<ICoreWebView2_10>()?;
                        let handler = BasicAuthenticationRequestedEventHandler::create(Box::new(
                            move |_sender, args| {
                                let Some(args) = args else {
                                    return Ok(());
                                };
                                let mut uri = PWSTR::null();
                                args.Uri(&mut uri)?;
                                let mut challenge = PWSTR::null();
                                args.Challenge(&mut challenge)?;
                                let url = uri.to_string().unwrap_or_default();
                                let realm = challenge.to_string().unwrap_or_default();
                                let deferral = args.GetDeferral()?;
                                let id = uuid::Uuid::new_v4().to_string();
                                let parsed = url::Url::parse(&url).ok();
                                let ask = LoginAsk {
                                    id: id.clone(),
                                    url: url.clone(),
                                    host: parsed
                                        .as_ref()
                                        .and_then(|u| u.host_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    port: parsed
                                        .as_ref()
                                        .and_then(|u| u.port_or_known_default())
                                        .unwrap_or(0),
                                    realm,
                                };
                                LOGINS.with(|logins| {
                                    logins
                                        .borrow_mut()
                                        .insert(id, PendingLogin { args, deferral });
                                });
                                emit(ask);
                                Ok(())
                            },
                        ));
                        let mut token = 0i64;
                        core10.add_BasicAuthenticationRequested(&handler, &mut token)?;
                    }
                    Ok(())
                })();
            })
            .map_err(|e| e.to_string())
    }

    /// Settles one login on the main thread. An empty username cancels it.
    pub fn answer_login(window: &WebviewWindow, id: &str, username: &str, password: &str) {
        let id = id.to_string();
        let username = username.to_string();
        let password = password.to_string();
        let _ = window.run_on_main_thread(move || {
            let pending = LOGINS.with(|logins| logins.borrow_mut().remove(&id));
            let Some(pending) = pending else {
                return;
            };
            // SAFETY: the args and deferral belong to this STA thread, which is
            // where the request arrived and where it is completed now.
            unsafe {
                if username.is_empty() {
                    let _ = pending.args.SetCancel(true);
                } else if let Ok(response) = pending.args.Response() {
                    let _ = response.SetUserName(&HSTRING::from(username));
                    let _ = response.SetPassword(&HSTRING::from(password));
                }
                let _ = pending.deferral.Complete();
            }
        });
    }

    pub fn cancel_all_logins(window: &WebviewWindow) {
        let _ = window.run_on_main_thread(|| {
            let drained: Vec<PendingLogin> =
                LOGINS.with(|logins| logins.borrow_mut().drain().map(|(_, v)| v).collect());
            for pending in drained {
                // SAFETY: as in answer_login.
                unsafe {
                    let _ = pending.args.SetCancel(true);
                    let _ = pending.deferral.Complete();
                }
            }
        });
    }
}

#[cfg(not(windows))]
mod sys {
    use super::LoginAsk;
    use serde_json::Value;
    use tauri::{Webview, WebviewWindow};

    pub fn call(_webview: &Webview, method: &str, _params: &Value) -> Result<Value, String> {
        Err(format!("{method} is not available on this platform"))
    }

    pub fn subscribe(
        _webview: &Webview,
        _event: &str,
        _handler: impl Fn(Value) + Send + Sync + 'static,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn watch_logins(
        _webview: &Webview,
        _target: &str,
        _emit: impl Fn(LoginAsk) + Send + Sync + 'static,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn answer_login(_window: &WebviewWindow, _id: &str, _username: &str, _password: &str) {}

    pub fn cancel_all_logins(_window: &WebviewWindow) {}
}

pub use sys::{answer_login, call, cancel_all_logins, subscribe, watch_logins};
