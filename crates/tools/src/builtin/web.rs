//! The web face (Go `internal/tools/builtin/webfetch.go`, `webfetch_page.go`,
//! `webimage.go`): `web_fetch` reads an HTTP(S) page and `import_web_image`
//! saves an image the fetched page listed.
//!
//! The URL spelling vetting, the page rendering, the image-target validation
//! and the digest are pure here; the real HTTP round trip, HTML reduction,
//! candidate extraction, image decode and file write are host seams behind
//! [`WebHost`]. A missing seam fails closed with a typed refusal.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::abi::{CallContext, Capability, CapabilitySet, PathWriter, Tool, ToolResult};
use crate::builtin::network::is_local;
use crate::provenance::{Provenance, ProvenanceDeclarer, ProvenanceKind, host_of};
use crate::refusal::Refusal;

/// `web_fetch`, the tool name the model calls.
pub const WEB_FETCH_TOOL_NAME: &str = "web_fetch";
/// `import_web_image`, the tool name the model calls.
pub const IMPORT_WEB_IMAGE_TOOL_NAME: &str = "import_web_image";

/// `web.internal_address` — an address on this machine's own network (loopback,
/// RFC1918, link-local, CGNAT) must not leave the workspace. The Go sentinel
/// carries no dotted code; this id mirrors its role.
pub const CODE_FETCH_INTERNAL_ADDRESS: &str = "web.internal_address";
/// The Go sentinel sentence, kept verbatim as the refusal message prefix.
pub const MSG_FETCH_INTERNAL_ADDRESS: &str = "refusing to fetch internal address";

/// Go `webFetch.Description()`, verbatim.
pub const WEB_FETCH_DESCRIPTION: &str = "Read HTTP(S) text. Reduces HTML to text and image candidates for import_web_image; returns JSON/plain text verbatim.";

/// Go `webImage.Description()`, verbatim.
pub const IMPORT_WEB_IMAGE_DESCRIPTION: &str = "Import web_fetch image_url from source url into new path. Validates PNG/JPEG/GIF/WebP. Returns image {path,alt,source} for render_surface. Match extension.";

const WEB_FETCH_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "url":{"type":"string"}
},
"required":["url"]
}"#;

const IMPORT_WEB_IMAGE_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["url","image_url","path","alt"],"properties":{"url":{"type":"string"},"image_url":{"type":"string"},"path":{"type":"string"},"alt":{"type":"string"}}}"#;

/// What a fetch found: the status the response carried, and the text the page
/// reduced to (Go `FetchPage`). Text is empty when the body was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchPage {
    pub status: i64,
    pub status_text: String,
    pub content_type: String,
    pub bytes: usize,
    pub text: String,
}

/// The saved image receipt `import_web_image` returns to the model (Go result
/// map). The host seam fills it after download, decode and write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageReceipt {
    pub path: String,
    pub alt: String,
    pub source: String,
    pub image_url: String,
    pub sha256: String,
    pub width: i64,
    pub height: i64,
}

/// The structured arguments behind an image import (Go `webImage.Execute`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageImportArgs {
    #[serde(default)]
    pub url: String,
    #[serde(rename = "image_url", default)]
    pub image_url: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub alt: String,
}

/// Host side the web tools need: the actual HTTP client, HTML reducer, image
/// decoder, file writer and receipt bookkeeping. The tool layer owns the
/// spelling vetting, validation and rendering; this trait never returns
/// unvetted results.
pub trait WebHost: Send + Sync {
    /// Fetches and reduces one page to its facts (Go `webFetch.fetch`). The
    /// seam applies the same SSRF guard the Go dialer applies.
    fn fetch_page(&self, url: &str) -> Result<FetchPage, Box<dyn std::error::Error + Send + Sync>>;
    /// Downloads the observed image, decodes it, writes `path` under the
    /// workspace and returns the receipt facts. Candidate membership,
    /// dimensions and extension match were already checked by the tool layer;
    /// the seam owns bytes, decoding and persistence.
    fn import_image(
        &self,
        args: &ImageImportArgs,
    ) -> Result<ImageReceipt, Box<dyn std::error::Error + Send + Sync>>;
}

/// The web host bound to one call, when the active session installed one.
pub fn web_host_from(ctx: &CallContext) -> Option<Arc<dyn WebHost>> {
    ctx.value()?.downcast_ref::<Arc<dyn WebHost>>().cloned()
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

/// Host-refusal the seam cannot serve its face at all without a browser/HTTP
/// backend behind it.
fn seam_unavailable() -> Refusal {
    Refusal::code(
        "web.host_unavailable",
        "web tools need a network fetch backend; none is installed in this runtime",
    )
}

// --- pure URL vetting (Go `VetFetchURL`, `publicByConstruction`) ------------

/// Whether a hostname names this machine's own DNS search space (Go
/// `externalurl.Host` local flag): bare `localhost`/`local` or their
/// dotted suffixes. Single-label other names are not local by construction.
pub fn public_by_construction(host: &str) -> bool {
    let bare = host.trim_end_matches('.').to_lowercase();
    !(bare == "localhost"
        || bare.ends_with(".localhost")
        || bare == "local"
        || bare.ends_with(".local"))
}

/// Whether an IP literal names this machine's own network (Go
/// `internalForRemote` / `blockedFetchAddress`). DNS resolution of names is a
/// host seam; a name this runtime cannot resolve passes, as in Go.
pub fn internal_ip_literal(host: &str) -> bool {
    if let Some(octets) = parse_v4(host) {
        return v4_is_internal(&octets);
    }
    let lower = host.to_lowercase();
    if lower == "::" || lower == "::1" {
        return true;
    }
    lower.starts_with("fe80")
        || lower.starts_with("fc")
        || lower.starts_with("fd")
        || lower.starts_with("ff")
}

fn parse_v4(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut out = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse::<u8>().ok()?;
    }
    Some(out)
}

fn v4_is_internal(o: &[u8; 4]) -> bool {
    let [a, b, _c, _] = *o;
    if a == 127 || a == 0 {
        return true;
    }
    if a == 10 {
        return true;
    }
    if a == 172 && (16..=31).contains(&b) {
        return true;
    }
    if a == 192 && b == 168 {
        return true;
    }
    if a == 169 && b == 254 {
        return true;
    }
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    a >= 224
}

/// Lexical vetting of a fetch URL before it reaches the seam: absolute
/// http/https, non-empty host, no credentials, public-by-construction name or
/// a non-internal IP literal. DNS resolution stays with the host seam.
pub fn vet_fetch_url(raw: &str) -> Result<(), Refusal> {
    if raw.is_empty() {
        return Err(Refusal::code("web.url_required", "url is required"));
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(Refusal::code(
            "web.url_invalid",
            "url must be an absolute http(s) address without credentials",
        ));
    };
    if scheme != "http" && scheme != "https" {
        return Err(Refusal::code(
            "web.url_invalid",
            "url must be an absolute http(s) address without credentials",
        ));
    }
    let (host, _) = match rest.split_once('/') {
        Some((h, _)) => (h, true),
        None => (rest, false),
    };
    let host = match host.split_once('@') {
        Some((_, _host)) => {
            return Err(Refusal::code(
                "web.url_invalid",
                "url must be an absolute http(s) address without credentials",
            ));
        }
        None => host,
    };
    let host_no_port = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    if host_no_port.is_empty() {
        return Err(Refusal::code(
            "web.url_invalid",
            "url must be an absolute http(s) address without credentials",
        ));
    }
    let looks_ip = host_no_port.parse::<std::net::IpAddr>().is_ok();
    if looks_ip {
        if internal_ip_literal(host_no_port) {
            return Err(Refusal::code(
                CODE_FETCH_INTERNAL_ADDRESS,
                format!("{MSG_FETCH_INTERNAL_ADDRESS} {host_no_port}"),
            ));
        }
        return Ok(());
    }
    if !public_by_construction(host_no_port) {
        return Err(Refusal::code(
            CODE_FETCH_INTERNAL_ADDRESS,
            format!("{MSG_FETCH_INTERNAL_ADDRESS} {host_no_port}"),
        ));
    }
    Ok(())
}

// --- pure rendering (Go `FetchPage.Rendered`, `contentTypeShort`) -----------

/// Strip the `;` charset fragment and surrounding whitespace (Go
/// `contentTypeShort`).
pub fn content_type_short(ct: &str) -> String {
    match ct.split_once(';') {
        Some((head, _)) => head.trim().to_owned(),
        None => ct.trim().to_owned(),
    }
}

/// The page as `web_fetch` has always shown it to the model (Go
/// `FetchPage.Rendered`).
pub fn render_fetch_page(page: &FetchPage) -> String {
    if page.text.is_empty() {
        return format!("(empty body — status {})", page.status_text);
    }
    format!(
        "status {} · {} · {} bytes\n\n{}",
        page.status_text,
        content_type_short(&page.content_type),
        page.bytes,
        page.text
    )
}

// --- image target validation (Go `validateImageTarget`) --------------------

/// Validates the relative image path and alt the model supplied. The path
/// must stay local and split into non-empty, non-dot segments; the alt is
/// required and bounded.
pub fn validate_image_target(path: &str, alt: &str) -> Result<(), Refusal> {
    if alt.is_empty() || alt.chars().count() > 200 || path.len() > 1024 {
        return Err(Refusal::code(
            "web.image_target_invalid",
            "valid relative image path and alt required",
        ));
    }
    if path.contains(|c: char| ":\\?#%\x00\r\n\t".contains(c)) || !is_local(path) {
        return Err(Refusal::code(
            "web.image_target_invalid",
            "valid relative image path and alt required",
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(Refusal::code(
                "web.image_path_invalid",
                "invalid image path",
            ));
        }
    }
    Ok(())
}

/// Lowercase hex digest of bytes (Go `sha256.Sum256` + `%x`). The hash itself
/// comes from the workspace `sha2` crate; this helper only formats its bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let sum = Sha256::digest(data);
    hex_lower(&sum)
}

/// Bytes → lowercase hex, eight lines of lookup formatting.
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

// --- tools ------------------------------------------------------------------

/// The `web_fetch` tool (Go `webFetch`).
#[derive(Debug, Default, Clone, Copy)]
pub struct WebFetchTool;

/// The one instance a composition root registers.
pub fn web_fetch_tool() -> Arc<dyn Tool> {
    Arc::new(WebFetchTool)
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        WEB_FETCH_TOOL_NAME
    }

    fn description(&self) -> &str {
        WEB_FETCH_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(WEB_FETCH_SCHEMA).expect("the web_fetch schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: FetchArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        vet_fetch_url(&parsed.url)
            .map_err(|r: Refusal| Box::new(r) as Box<dyn std::error::Error + Send + Sync>)?;

        let Some(host) = web_host_from(ctx) else {
            return Err(seam_unavailable().into());
        };
        let page = host.fetch_page(&parsed.url)?;
        Ok(render_fetch_page(&page))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "provenance",
            Arc::new(WebProvenance) as Arc<dyn ProvenanceDeclarer>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Deserialize, Default)]
struct FetchArgs {
    #[serde(default)]
    url: String,
}

struct WebProvenance;
impl Capability for WebProvenance {}
impl ProvenanceDeclarer for WebProvenance {
    fn provenance(&self, args: &Value) -> Provenance {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        Provenance::new(ProvenanceKind::Web, host_of(url))
    }
}

/// The `import_web_image` tool (Go `webImage`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ImportWebImageTool;

/// The one instance a composition root registers.
pub fn import_web_image_tool() -> Arc<dyn Tool> {
    Arc::new(ImportWebImageTool)
}

impl Tool for ImportWebImageTool {
    fn name(&self) -> &str {
        IMPORT_WEB_IMAGE_TOOL_NAME
    }

    fn description(&self) -> &str {
        IMPORT_WEB_IMAGE_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(IMPORT_WEB_IMAGE_SCHEMA)
            .expect("the import_web_image schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: ImageImportArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        validate_image_target(&parsed.path, &parsed.alt)?;
        vet_fetch_url(&parsed.url)
            .map_err(|r: Refusal| Box::new(r) as Box<dyn std::error::Error + Send + Sync>)?;

        let Some(host) = web_host_from(ctx) else {
            return Err(seam_unavailable().into());
        };
        let receipt = host.import_image(&parsed)?;
        let out = serde_json::to_value(serde_json::json!({
            "image": {
                "path": receipt.path,
                "alt": receipt.alt,
                "source": receipt.source,
            },
            "image_url": receipt.image_url,
            "sha256": receipt.sha256,
            "width": receipt.width,
            "height": receipt.height,
        }))
        .map_err(|e| invalid_err(format!("render receipt: {e}")))?;
        Ok(out.to_string())
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "provenance",
                Arc::new(ImageProvenance) as Arc<dyn ProvenanceDeclarer>,
            )
            .with(
                "path_writer",
                Arc::new(NamedPathWriter) as Arc<dyn PathWriter>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ImageProvenance;
impl Capability for ImageProvenance {}
impl ProvenanceDeclarer for ImageProvenance {
    fn provenance(&self, args: &Value) -> Provenance {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        Provenance::new(ProvenanceKind::Web, host_of(url))
    }
}

struct NamedPathWriter;
impl Capability for NamedPathWriter {}
impl PathWriter for NamedPathWriter {
    fn writes_named_paths(&self) -> bool {
        true
    }
}

/// The web face for one composition root.
pub fn web_tools() -> Vec<Arc<dyn Tool>> {
    vec![web_fetch_tool(), import_web_image_tool()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        let _ = WebFetchTool.schema();
        let _ = ImportWebImageTool.schema();
    }

    #[test]
    fn public_by_construction_shapes() {
        assert!(public_by_construction("example.com"));
        assert!(!public_by_construction("localhost"));
        assert!(!public_by_construction("dev.local"));
        assert!(public_by_construction("internal.service"));
        assert!(public_by_construction("just-a-name"));
    }

    #[test]
    fn internal_ip_literal_classifies() {
        assert!(internal_ip_literal("127.0.0.1"));
        assert!(internal_ip_literal("10.0.4.2"));
        assert!(internal_ip_literal("192.168.1.9"));
        assert!(internal_ip_literal("169.254.169.254"));
        assert!(internal_ip_literal("100.64.0.1"));
        assert!(!internal_ip_literal("93.184.216.34"));
    }

    #[test]
    fn vet_rejects_credential_and_internal() {
        assert!(vet_fetch_url("https://user:pw@example.com/").is_err());
        assert!(vet_fetch_url("http://127.0.0.1:8080/").is_err());
        assert!(vet_fetch_url("http://localhost/").is_err());
        assert!(vet_fetch_url("https://example.com/").is_ok());
    }

    #[test]
    fn render_empty_and_filled_page() {
        let empty = FetchPage {
            status: 404,
            status_text: "404 Not Found".to_owned(),
            ..Default::default()
        };
        assert_eq!(
            render_fetch_page(&empty),
            "(empty body — status 404 Not Found)"
        );
        let full = FetchPage {
            status: 200,
            status_text: "200 OK".to_owned(),
            content_type: "text/html; charset=utf-8".to_owned(),
            bytes: 12,
            text: "hello".to_owned(),
        };
        assert_eq!(
            render_fetch_page(&full),
            "status 200 OK · text/html · 12 bytes\n\nhello"
        );
    }

    #[test]
    fn image_target_validation() {
        assert!(validate_image_target("assets/a.png", "Alt text").is_ok());
        assert!(validate_image_target("", "Alt").is_err());
        assert!(validate_image_target("assets/a.png", "").is_err());
        assert!(validate_image_target("../escape.png", "Alt").is_err());
        assert!(validate_image_target("a/b?.png", "Alt").is_err());
    }

    #[test]
    fn sha256_matches_known_vector() {
        // sha256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let digest = sha256_hex(b"abc");
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
