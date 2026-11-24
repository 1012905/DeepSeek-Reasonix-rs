//! The web face at the tools boundary (Go `internal/tools/builtin/webfetch.go`,
//! `webfetch_page.go`, `webimage.go`): tool cards, URL vetting, page rendering,
//! image-target validation, the digest, and the seam-absent refusal.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    FetchPage, IMPORT_WEB_IMAGE_TOOL_NAME, ImageImportArgs, ImageReceipt, ImportWebImageTool,
    WEB_FETCH_TOOL_NAME, WebFetchTool, WebHost, content_type_short, render_fetch_page, sha256_hex,
    validate_image_target, vet_fetch_url,
};
use serde_json::json;

struct FakeWeb {
    page: Option<FetchPage>,
    receipt: Option<ImageReceipt>,
}

impl WebHost for FakeWeb {
    fn fetch_page(
        &self,
        _url: &str,
    ) -> Result<FetchPage, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.page.clone().unwrap_or_default())
    }

    fn import_image(
        &self,
        _args: &ImageImportArgs,
    ) -> Result<ImageReceipt, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.receipt.clone().unwrap_or_default())
    }
}

fn ctx_with(host: Arc<FakeWeb>) -> CallContext {
    let bound: Arc<Arc<dyn WebHost>> = Arc::new(host as Arc<dyn WebHost>);
    CallContext::new().with_value(bound)
}

#[test]
fn web_fetch_card_matches_frozen_catalog() {
    let tool = WebFetchTool;
    assert_eq!(tool.name(), WEB_FETCH_TOOL_NAME);
    assert!(tool.description().contains("HTTP(S) text"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn import_web_image_card_matches_frozen_catalog() {
    let tool = ImportWebImageTool;
    assert_eq!(tool.name(), IMPORT_WEB_IMAGE_TOOL_NAME);
    assert!(tool.description().contains("Validates PNG/JPEG/GIF/WebP"));
    assert!(tool.schema().is_object());
    assert!(!tool.read_only());
}

#[test]
fn vet_fetch_url_rejects_local_credentials_and_schemes() {
    assert!(vet_fetch_url("").is_err());
    assert!(vet_fetch_url("ftp://example.com/").is_err());
    assert!(vet_fetch_url("https://user:pw@example.com/").is_err());
    assert!(vet_fetch_url("http://127.0.0.1/").is_err());
    assert!(vet_fetch_url("http://localhost/").is_err());
    assert!(vet_fetch_url("http://169.254.169.254/").is_err());
    assert!(vet_fetch_url("https://example.com/").is_ok());
}

#[test]
fn render_fetch_page_empty_and_filled() {
    let empty = FetchPage {
        status: 404,
        status_text: "404 Not Found".into(),
        ..Default::default()
    };
    assert_eq!(
        render_fetch_page(&empty),
        "(empty body — status 404 Not Found)"
    );
    let full = FetchPage {
        status: 200,
        status_text: "200 OK".into(),
        content_type: "text/html; charset=utf-8".into(),
        bytes: 12,
        text: "hello".into(),
    };
    assert_eq!(
        render_fetch_page(&full),
        "status 200 OK · text/html · 12 bytes\n\nhello"
    );
    assert_eq!(
        content_type_short("application/json; charset=utf-8"),
        "application/json"
    );
}

#[test]
fn image_target_validation_blocks_traversal_and_bad_alt() {
    assert!(validate_image_target("assets/a.png", "Alt text").is_ok());
    assert!(validate_image_target("", "Alt").is_err());
    assert!(validate_image_target("assets/a.png", "").is_err());
    assert!(validate_image_target("../escape.png", "Alt").is_err());
    assert!(validate_image_target("a/b?.png", "Alt").is_err());
}

#[test]
fn sha256_hex_matches_known_vector() {
    // sha256("abc") — a standard NIST vector, proving the workspace sha2 crate
    // (not a hand-written hash) backs the digest.
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn web_fetch_passes_vetted_page_through_the_bound_host() {
    let host = Arc::new(FakeWeb {
        page: Some(FetchPage {
            status: 200,
            status_text: "200 OK".into(),
            content_type: "text/plain".into(),
            bytes: 5,
            text: "hello".into(),
        }),
        receipt: None,
    });
    let out = WebFetchTool
        .execute(&ctx_with(host), &json!({"url": "https://example.com/"}))
        .expect("fetch renders through the seam");
    assert!(out.starts_with("status 200 OK · text/plain · 5 bytes"));
}

#[test]
fn web_fetch_without_host_fails_closed() {
    let err = WebFetchTool
        .execute(&CallContext::new(), &json!({"url": "https://example.com/"}))
        .expect_err("seam absent refuses");
    assert!(err.to_string().contains("network fetch backend"));
}

#[test]
fn import_web_image_validates_then_returns_receipt_json() {
    let host = Arc::new(FakeWeb {
        page: None,
        receipt: Some(ImageReceipt {
            path: "assets/a.png".into(),
            alt: "Alt".into(),
            source: "example.com".into(),
            image_url: "https://example.com/a.png".into(),
            sha256: sha256_hex(b"abc"),
            width: 800,
            height: 600,
        }),
    });
    let args = json!({
        "url": "https://example.com/",
        "image_url": "https://example.com/a.png",
        "path": "assets/a.png",
        "alt": "Alt",
    });
    let out = ImportWebImageTool
        .execute(&ctx_with(host), &args)
        .expect("import renders receipt");
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("receipt is JSON");
    assert_eq!(parsed["image"]["path"], "assets/a.png");
    assert_eq!(parsed["width"], 800);
}

#[test]
fn import_web_image_rejects_internal_source_before_the_seam() {
    let err = ImportWebImageTool
        .execute(
            &CallContext::new(),
            &json!({
                "url": "http://127.0.0.1/",
                "image_url": "http://127.0.0.1/a.png",
                "path": "assets/a.png",
                "alt": "Alt",
            }),
        )
        .expect_err("internal source refused before host");
    assert!(err.to_string().contains("internal address"));
}
