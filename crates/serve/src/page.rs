//! Built-page discovery and file-first serving (Go `page.go`): the page a
//! host serves wins over the kernel for every path that names a real file in
//! it, and every other path reaches the kernel. Content types come from a
//! fixed table — the Windows registry lookup can lie about the SVG logo.

use std::io;
use std::path::{Path, PathBuf};

use bytes::Bytes;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;

/// The type of every kind of file the built page ships (page.go:64-69).
pub(crate) const PAGE_CONTENT_TYPES: &[(&str, &str)] = &[
    (".html", "text/html; charset=utf-8"),
    (".js", "text/javascript; charset=utf-8"),
    (".mjs", "text/javascript; charset=utf-8"),
    (".css", "text/css; charset=utf-8"),
    (".json", "application/json"),
    (".map", "application/json"),
    (".svg", "image/svg+xml"),
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
    (".ico", "image/x-icon"),
    (".woff", "font/woff"),
    (".woff2", "font/woff2"),
    (".ttf", "font/ttf"),
    (".otf", "font/otf"),
    (".wasm", "application/wasm"),
    (".webmanifest", "application/manifest+json"),
    (".txt", "text/plain; charset=utf-8"),
];

/// Why page discovery failed (Go `FindPage`'s error return).
#[derive(Debug, thiserror::Error)]
pub enum FindPageError {
    /// An explicit directory holding no index.html is a launch that would
    /// open on nothing, so it fails here rather than at the first paint.
    #[error("no index.html under {0}")]
    ExplicitDirMissingIndex(PathBuf),
    /// The executable could not be located, so nothing beside it can be.
    #[error("the executable could not be located: {0}")]
    ExeUnavailable(#[from] io::Error),
}

/// Resolves the built page a host serves (Go `FindPage`): an explicit
/// directory must contain index.html; else beside the executable (following
/// a symlink, where a package manager linked the binary onto the PATH); else
/// the source-tree dist directories; `None` — the host serves the kernel
/// alone, and says so in its own words.
pub fn find_page(dir: Option<&Path>) -> Result<Option<PathBuf>, FindPageError> {
    if let Some(dir) = dir {
        if !has_index(dir) {
            return Err(FindPageError::ExplicitDirMissingIndex(dir.to_path_buf()));
        }
        return Ok(Some(dir.to_path_buf()));
    }
    let exe = std::env::current_exe()?;
    if let Some(page) = page_beside(&exe) {
        return Ok(Some(page));
    }
    for candidate in ["../desktop-tauri/dist", "desktop-tauri/dist"] {
        if has_index(Path::new(candidate)) {
            return Ok(Some(PathBuf::from(candidate)));
        }
    }
    Ok(None)
}

/// Go `pageBeside`: looks next to the executable as invoked, then next to
/// where a link to it points.
fn page_beside(exe: &Path) -> Option<PathBuf> {
    let mut dirs = vec![exe.parent()?.to_path_buf()];
    if let Ok(real) = std::fs::canonicalize(exe)
        && let Some(real_dir) = real.parent()
        && dirs[0] != real_dir
    {
        dirs.push(real_dir.to_path_buf());
    }
    dirs.iter()
        .map(|dir| dir.join("frontend-next").join("dist"))
        .find(|candidate| has_index(candidate))
}

fn has_index(dir: &Path) -> bool {
    dir.join("index.html")
        .metadata()
        .is_ok_and(|st| !st.is_dir())
}

/// The content type for a page file's extension, `None` for anything the
/// table does not name.
pub(crate) fn page_content_type(name: &str) -> Option<&'static str> {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_ascii_lowercase()))?;
    PAGE_CONTENT_TYPES
        .iter()
        .find(|(known, _)| *known == ext)
        .map(|(_, kind)| *kind)
}

/// Go `withPage` as middleware: serves the files the built page brought with
/// it and leaves every other path to the kernel. Which paths are the page's
/// is asked of the page itself, never of a list of kernel routes — a path
/// naming no file is the page's own routing.
pub(crate) async fn with_page_middleware(
    State(state): State<crate::router::RouterState>,
    request: axum::extract::Request<Body>,
    next: Next,
) -> Response {
    let Some(page_dir) = state.page.clone() else {
        return next.run(request).await;
    };
    // Go trims both slashes, then lets fs.Stat decide: an empty name is the
    // kernel's root, and a name fs.ValidPath would reject (a `.` or `..`
    // element, an internal empty element) is the kernel's business too.
    let name = request.uri().path().trim_matches('/');
    if name.is_empty() || !is_valid_page_name(name) {
        return next.run(request).await;
    }
    let target = page_dir.join(name);
    let Ok(meta) = std::fs::metadata(&target) else {
        return next.run(request).await;
    };
    if meta.is_dir() {
        return next.run(request).await;
    }
    match read_page_file(&target) {
        Some(bytes) => file_response(request.method(), name, bytes),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error\n").into_response(),
    }
}

/// Go `fs.ValidPath`: slash-separated, no empty, `.` or `..` elements.
fn is_valid_page_name(name: &str) -> bool {
    name.split('/')
        .all(|el| !el.is_empty() && el != "." && el != "..")
}

fn read_page_file(target: &Path) -> Option<Bytes> {
    std::fs::read(target).ok().map(Bytes::from)
}

fn file_response(method: &Method, name: &str, bytes: Bytes) -> Response {
    if method != Method::GET && method != Method::HEAD {
        // Go http.FileServer's shape: 405 with the methods it does accept.
        let mut response = (
            StatusCode::METHOD_NOT_ALLOWED,
            [(axum::http::header::ALLOW, "GET, HEAD")],
            "Method Not Allowed\n",
        )
            .into_response();
        response.headers_mut().remove(CONTENT_TYPE);
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        headers.insert(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        for (key, value) in headers.iter() {
            response.headers_mut().insert(key, value.clone());
        }
        return response;
    }
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    if let Some((name_header, kind)) = page_content_type(name).map(|kind| (CONTENT_TYPE, kind))
        && let Ok(value) = HeaderValue::from_str(kind)
    {
        response.headers_mut().insert(name_header, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::tempdir;

    #[test]
    fn explicit_dir_requires_index_html() {
        let tmp = tempdir();
        assert!(matches!(
            find_page(Some(tmp.path())),
            Err(FindPageError::ExplicitDirMissingIndex(_))
        ));
        std::fs::write(tmp.path().join("index.html"), b"<html>").unwrap();
        let found = find_page(Some(tmp.path())).unwrap().unwrap();
        assert_eq!(found, tmp.path());
    }

    #[test]
    fn an_index_that_is_a_directory_is_not_a_page() {
        let tmp = tempdir();
        std::fs::create_dir_all(tmp.path().join("index.html")).unwrap();
        assert!(matches!(
            find_page(Some(tmp.path())),
            Err(FindPageError::ExplicitDirMissingIndex(_))
        ));
    }

    #[test]
    fn no_page_found_is_none_not_an_error() {
        // A fresh cwd tree has no dist directories beside the test binary.
        // Beside-the-exe and the source-tree candidates may still hit in a
        // checkout, so only the shape of the answer is asserted.
        match find_page(None) {
            Ok(found) => {
                if let Some(page) = found {
                    assert!(has_index(&page));
                }
            }
            Err(err) => panic!("find_page(None) must not fail: {err}"),
        }
    }

    #[test]
    fn content_types_come_from_the_fixed_table() {
        assert_eq!(page_content_type("a.svg"), Some("image/svg+xml"));
        assert_eq!(page_content_type("A.SVG"), Some("image/svg+xml"));
        assert_eq!(
            page_content_type("index.html"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            page_content_type("app.webmanifest"),
            Some("application/manifest+json")
        );
        assert_eq!(page_content_type("data.bin"), None);
        assert_eq!(page_content_type("no-extension"), None);
    }

    #[test]
    fn page_names_reject_traversal_elements() {
        assert!(is_valid_page_name("assets/app.js"));
        assert!(is_valid_page_name("index.html"));
        assert!(!is_valid_page_name("../secret"));
        assert!(!is_valid_page_name("a/./b"));
        assert!(!is_valid_page_name("a//b"));
        assert!(!is_valid_page_name("a/"));
    }

    #[tokio::test]
    async fn with_page_serves_a_real_file_and_leaves_the_rest_to_the_kernel() {
        use crate::router::{RouterConfig, build_router};
        use reasonix_session::hub::{Hub, HubPaths};
        use std::sync::Arc;

        let page = tempdir();
        std::fs::create_dir_all(page.path().join("assets")).unwrap();
        std::fs::write(page.path().join("index.html"), b"<html>shell</html>").unwrap();
        std::fs::write(page.path().join("assets/logo.svg"), b"<svg/>").unwrap();

        let tmp = tempdir();
        let state = build_router(
            Arc::new(Hub::new(HubPaths {
                state_root: tmp.path().to_path_buf(),
            })),
            RouterConfig {
                token: "tkn".to_owned(),
                origin: String::new(),
                page_dir: Some(page.path().to_path_buf()),
                ..RouterConfig::default()
            },
        );
        use tower::ServiceExt as _ServiceExt;
        let response = state
            .clone()
            .oneshot(
                axum::extract::Request::builder()
                    .uri("/assets/logo.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("image/svg+xml"))
        );
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"<svg/>");

        // A path naming no file reaches the kernel, which (with no runtime)
        // answers 503 hub.no_runtime_open.
        let response = state
            .clone()
            .oneshot(
                axum::extract::Request::builder()
                    .uri("/assets/absent.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
