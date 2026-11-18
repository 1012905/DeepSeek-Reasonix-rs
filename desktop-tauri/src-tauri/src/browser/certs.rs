// A page's certificate is refused unless a pair the person trusted this run
// names it. On Windows, WebView2 raises the refusal natively, so the shell
// answers it on the main thread: a trusted pair proceeds, anything else is
// cancelled and handed to the window's page, which alone may trust it. Other
// platforms raise no such event yet, so no certificate is ever offered to trust.
use sha2::{Digest, Sha256};
use url::{Host, Url};

/// The certificate a load was refused for, as the window's page reads it.
#[derive(Clone)]
pub struct CertInfo {
    pub host: String,
    pub url: String,
    pub error: String,
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    /// Seconds since the epoch.
    pub expiry: f64,
}

pub fn trust_key(host: &str, fingerprint: &str) -> String {
    format!("{host} {fingerprint}")
}

/// The SHA-256 of the DER certificate, colon-separated hex, the form the
/// platform's own certificate object reports.
fn fingerprint(pem: &str) -> String {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let Ok(der) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.trim())
    else {
        return String::new();
    };
    Sha256::digest(&der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn error_name(status: i32) -> &'static str {
    match status {
        1 => "ERR_CERT_COMMON_NAME_INVALID",
        2 => "ERR_CERT_DATE_INVALID",
        3 => "ERR_SSL_CLIENT_AUTH_CERT_BAD_FORMAT",
        4 => "ERR_CERT_REVOKED",
        5 => "ERR_CERT_INVALID",
        _ => "ERR_FAILED",
    }
}

/// The host as a URL spells it, port and all: the pair is bound to exactly the
/// address the certificate was refused for.
fn host_of(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return String::new();
    };
    let host = match url.host() {
        Some(Host::Ipv6(addr)) => format!("[{addr}]"),
        Some(Host::Ipv4(addr)) => addr.to_string(),
        Some(Host::Domain(domain)) => domain.to_string(),
        None => return String::new(),
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

#[cfg(windows)]
mod sys {
    use super::{CertInfo, error_name, fingerprint, host_of, trust_key};
    use std::cell::RefCell;
    use std::collections::HashSet;
    use tauri::{Webview, WebviewWindow};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, COREWEBVIEW2_WEB_ERROR_STATUS,
        ICoreWebView2_14,
    };
    use webview2_com::ServerCertificateErrorDetectedEventHandler;
    use windows::core::{Interface, PWSTR};

    // The pairs trusted this run live on the main thread, where every refusal is
    // answered; a restart forgets them, so a new run asks again.
    thread_local! {
        static TRUSTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    }

    /// Records a pair the person chose to proceed past. The window's page asks
    /// for this; neither the kernel nor the agent driving the browser can.
    pub fn trust(window: &WebviewWindow, host: &str, fingerprint: &str) {
        let key = trust_key(host, fingerprint);
        let _ = window.run_on_main_thread(move || {
            TRUSTED.with(|trusted| {
                trusted.borrow_mut().insert(key);
            });
        });
    }

    fn is_trusted(host: &str, fingerprint: &str) -> bool {
        TRUSTED.with(|trusted| trusted.borrow().contains(&trust_key(host, fingerprint)))
    }

    fn out_string(read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
        let mut value = PWSTR::null();
        if read(&mut value).is_err() {
            return String::new();
        }
        // SAFETY: the callee filled value with a NUL-terminated wide string it
        // owns for this call; reading it here copies before it is freed.
        unsafe { value.to_string() }.unwrap_or_default()
    }

    /// Watches for the certificate a page's load was refused for. A trusted pair
    /// proceeds at once; anything else is cancelled and reported to the page.
    pub fn watch_certificates(
        webview: &Webview,
        on_cert: impl Fn(CertInfo) + Send + Sync + 'static,
    ) -> Result<(), String> {
        webview
            .with_webview(move |platform| {
                let _ = (|| -> windows::core::Result<()> {
                    // SAFETY: the core is live on the main thread; the event
                    // source keeps the handler alive through its COM reference.
                    unsafe {
                        let core = platform.controller().CoreWebView2()?;
                        let core14 = core.cast::<ICoreWebView2_14>()?;
                        let handler = ServerCertificateErrorDetectedEventHandler::create(Box::new(
                            move |_sender, args| {
                                let Some(args) = args else {
                                    return Ok(());
                                };
                                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                                args.ErrorStatus(&mut status)?;
                                let url = out_string(|out| args.RequestUri(out));
                                let cert = args.ServerCertificate()?;
                                let subject = out_string(|out| cert.Subject(out));
                                let issuer = out_string(|out| cert.Issuer(out));
                                let mut expiry = 0f64;
                                cert.ValidTo(&mut expiry)?;
                                let pem = out_string(|out| cert.ToPemEncoding(out));
                                let fp = fingerprint(&pem);
                                let host = host_of(&url);
                                if is_trusted(&host, &fp) {
                                    args.SetAction(
                                        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
                                    )?;
                                    return Ok(());
                                }
                                args.SetAction(
                                    COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL,
                                )?;
                                on_cert(CertInfo {
                                    host,
                                    url,
                                    error: error_name(status.0).to_string(),
                                    fingerprint: fp,
                                    subject,
                                    issuer,
                                    expiry,
                                });
                                Ok(())
                            },
                        ));
                        let mut token = 0i64;
                        core14.add_ServerCertificateErrorDetected(&handler, &mut token)?;
                    }
                    Ok(())
                })();
            })
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod sys {
    use super::CertInfo;
    use tauri::{Webview, WebviewWindow};

    pub fn trust(_window: &WebviewWindow, _host: &str, _fingerprint: &str) {}

    pub fn watch_certificates(
        _webview: &Webview,
        _on_cert: impl Fn(CertInfo) + Send + Sync + 'static,
    ) -> Result<(), String> {
        Ok(())
    }
}

pub use sys::{trust, watch_certificates};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_is_the_host_and_the_fingerprint_together() {
        assert_eq!(trust_key("example.com", "AB:CD"), "example.com AB:CD");
        assert_ne!(
            trust_key("example.com", "AB:CD"),
            trust_key("example.com", "AB:CE")
        );
    }

    #[test]
    fn the_fingerprint_is_colon_separated_sha256_of_the_der() {
        let pem = "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n";
        let expected = Sha256::digest(b"abc")
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(fingerprint(pem), expected);
        assert_eq!(fingerprint("not a certificate"), "");
    }

    #[test]
    fn certificate_statuses_keep_their_net_error_names() {
        assert_eq!(error_name(1), "ERR_CERT_COMMON_NAME_INVALID");
        assert_eq!(error_name(2), "ERR_CERT_DATE_INVALID");
        assert_eq!(error_name(4), "ERR_CERT_REVOKED");
        assert_eq!(error_name(5), "ERR_CERT_INVALID");
        assert_eq!(error_name(0), "ERR_FAILED");
    }

    #[test]
    fn the_host_carries_the_port_the_url_spells() {
        assert_eq!(host_of("https://example.com/a"), "example.com");
        assert_eq!(host_of("https://example.com:8443/a"), "example.com:8443");
        assert_eq!(host_of("https://example.com:443/a"), "example.com");
        assert_eq!(host_of("http://127.0.0.1:8080/"), "127.0.0.1:8080");
        assert_eq!(host_of("https://[::1]:9000/"), "[::1]:9000");
        assert_eq!(host_of("not a url"), "");
    }
}
