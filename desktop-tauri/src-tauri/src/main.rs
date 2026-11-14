// The shell owns OS capability and no business state; the kernel is a sidecar
// this process spawns and holds a lease on — the pipe its stdin runs on, so
// the host's lifetime is exactly the shell's. The handshake is one JSON line
// on stdout naming the loopback origin the host serves the Studio page from
// and the token the shell sets straight into the webview's cookie store as
// the reasonix_token cookie the gate reads (the gate takes the credential
// from the cookie only: a query token would land in request lines). Acts the host asks of the
// shell arrive on the same pipe, one line each, after the handshake.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::webview::Cookie;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const PAGE_PATH: &str = "/";

#[derive(Deserialize)]
struct Handshake {
    version: u32,
    origin: String,
    token: String,
}

fn host_binary() -> PathBuf {
    std::env::var_os("REASONIX_STUDIO_HOST_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin/reasonix-studio-host.exe")
        })
}

fn page_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../desktop/frontend-next/dist")
}

fn crash_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("reasonix-studio-crash");
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn spawn_host() -> Result<Child, String> {
    let bin = host_binary();
    let page = page_dir();
    if !bin.exists() {
        return Err(format!(
            "the kernel host binary is missing at {} — build it: go build -o src-tauri/bin/reasonix-studio-host.exe ../../cmd/reasonix-studio-host",
            bin.display()
        ));
    }
    if !page.join("index.html").exists() {
        return Err(format!(
            "the Studio page build is missing at {} — run: pnpm --dir desktop/frontend-next build",
            page.display()
        ));
    }
    Command::new(&bin)
        .arg("-page")
        .arg(&page)
        .arg("-crash-dir")
        .arg(crash_dir())
        .arg("-studio-app-pid")
        .arg(std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawning {}: {e}", bin.display()))
}

fn read_handshake(stdout: &mut impl BufRead) -> Result<Handshake, String> {
    let mut line = String::new();
    stdout
        .read_line(&mut line)
        .map_err(|e| format!("reading the host handshake: {e}"))?;
    if line.trim().is_empty() {
        return Err("the host ended before its handshake".into());
    }
    let handshake: Handshake = serde_json::from_str(line.trim())
        .map_err(|e| format!("parsing the host handshake: {e}"))?;
    if handshake.version != 1 {
        return Err(format!(
            "unsupported host handshake version {}",
            handshake.version
        ));
    }
    Ok(handshake)
}

// The gate reads the credential from the reasonix_token cookie only — the
// query form would put it in request lines — so the handshake's token is set
// straight into the webview's cookie store, mirroring the Electron shell's
// armCredential: host-only, path /, httpOnly, strict.
fn credential_cookie(origin: &str, token: &str) -> Result<Cookie<'static>, String> {
    let url: tauri::Url = origin
        .parse()
        .map_err(|e| format!("parsing the host origin: {e}"))?;
    Ok(Cookie::build(("reasonix_token".to_string(), token.to_string()))
        .path("/")
        .domain(
            url.host_str()
                .ok_or("the host origin has no host")?
                .to_string(),
        )
        .http_only(true)
        .secure(false)
        .same_site(tauri::webview::cookie::SameSite::Strict)
        .build())
}
// Acting on the acts — relaunch after an update, quit, show — is a later
// phase; for now they are drained so the pipe never fills and blocks the host.
fn drain_acts(stdout: impl BufRead + Send + 'static) {
    std::thread::spawn(move || {
        for line in stdout.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => println!("host act: {line}"),
                Err(_) => break,
                Ok(_) => {}
            }
        }
    });
}

fn run() -> Result<(), String> {
    tauri::Builder::default()
        .setup(|app| {
            let mut kernel = spawn_host()?;
            let bound = {
                let stdout = kernel
                    .stdout
                    .take()
                    .ok_or("the host stdout pipe is missing")?;
                let mut reader = BufReader::new(stdout);
                let bound = read_handshake(&mut reader)?;
                drain_acts(reader);
                bound
            };
            let cookie = credential_cookie(&bound.origin, &bound.token)?;
            let webview = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(
                    "about:blank"
                        .parse()
                        .map_err(|e| format!("blank url: {e}"))?,
                ),
            )
            .title("Reasonix Studio")
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|e| format!("building the window: {e}"))?;
            webview
                .set_cookie(cookie)
                .map_err(|e| format!("setting the auth cookie: {e}"))?;
            let origin: tauri::Url = bound
                .origin
                .parse()
                .map_err(|e| format!("parsing the host origin: {e}"))?;
            let page = origin
                .join(PAGE_PATH)
                .map_err(|e| format!("joining the Studio page path: {e}"))?;
            webview
                .navigate(page)
                .map_err(|e| format!("navigating to the Studio page: {e}"))?;
            app.manage(Mutex::new(kernel));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        panic!("Failed to setup app: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    use std::net::TcpListener;

    #[test]
    fn handshake_parses_the_host_line() {
        let mut line = Cursor::new(
            "{\"version\":1,\"origin\":\"http://127.0.0.1:5281\",\"token\":\"t3\"}\n".to_string(),
        );
        let bound = read_handshake(&mut line).expect("parses");
        assert_eq!(bound.origin, "http://127.0.0.1:5281");
        assert_eq!(bound.token, "t3");
    }

    #[test]
    fn handshake_refuses_unknown_versions() {
        let mut line =
            Cursor::new("{\"version\":2,\"origin\":\"x\",\"token\":\"t\"}\n".to_string());
        assert!(read_handshake(&mut line).is_err());
    }

    #[test]
    fn handshake_refuses_an_ended_pipe() {
        let mut line = Cursor::new(String::new());
        assert!(read_handshake(&mut line).is_err());
    }

    // The credential cookie mirrors the Electron shell's armCredential shape:
    // the handshake token under the gate's own cookie name, host-only for the
    // origin the page is served from.
    #[test]
    fn credential_cookie_carries_the_handshake_token() {
        let cookie = credential_cookie("http://127.0.0.1:5281", "t3").expect("builds");
        assert_eq!(cookie.name(), "reasonix_token");
        assert_eq!(cookie.value(), "t3");
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.domain(), Some("127.0.0.1"));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(
            cookie.same_site(),
            Some(tauri::webview::cookie::SameSite::Strict)
        );
        assert_eq!(cookie.secure(), Some(false));
    }
}
