//! Vendor endpoint detection read from the base URL host (Go:
//! `internal/contract/provider/vendor.go`, `schema_dialect.go`,
//! `endpoint_override.go`, and `openai/host.go`/`chaturl.go`).
//!
//! The host extractor covers the URL shapes endpoints actually carry
//! (scheme://[user@]host[:port]/path); detection must never misclassify on
//! them, and a malformed URL fails every check.

/// Lowercased host of a URL, without userinfo or port (Go: `url.Parse` +
/// `Hostname`). None when there is no scheme or no host.
pub fn url_host(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty()
        || !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
    {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() {
            h
        } else {
            authority
        }
    });
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Whether `base_url` is one of the canonical hosts or any subdomain of
/// `apex`, case-insensitively (Go: `matchVendorHost`). The bare apex is not an
/// API endpoint and does not match.
pub fn match_vendor_host(base_url: &str, apex: &str, canonical: &[&str]) -> bool {
    let Some(host) = url_host(base_url) else {
        return false;
    };
    canonical.iter().any(|c| host == *c) || host.ends_with(&format!(".{apex}"))
}

/// DeepSeek API: api.deepseek.com or any *.deepseek.com subdomain.
pub fn is_deepseek(base_url: &str) -> bool {
    match_vendor_host(base_url, "deepseek.com", &["api.deepseek.com"])
}

/// Official OpenAI API host only (Go: exact-host so a gateway under another
/// openai.com subdomain never receives the official wire shape).
pub fn is_openai(base_url: &str) -> bool {
    url_host(base_url).is_some_and(|h| h == "api.openai.com")
}

/// Gemini Developer API host only.
pub fn is_gemini(base_url: &str) -> bool {
    url_host(base_url).is_some_and(|h| h == "generativelanguage.googleapis.com")
}

/// MiniMax China or international endpoints, including subdomains.
pub fn is_minimax(base_url: &str) -> bool {
    match_vendor_host(base_url, "minimaxi.com", &["api.minimaxi.com"])
        || match_vendor_host(base_url, "minimax.io", &["api.minimax.io"])
}

/// Zhipu GLM endpoints: *.bigmodel.cn or *.z.ai.
pub fn is_zhipu(base_url: &str) -> bool {
    match_vendor_host(base_url, "bigmodel.cn", &["open.bigmodel.cn"])
        || match_vendor_host(base_url, "z.ai", &["api.z.ai"])
}

/// LongCat's OpenAI-compatible API.
pub fn is_longcat(base_url: &str) -> bool {
    match_vendor_host(base_url, "longcat.chat", &["api.longcat.chat"])
}

/// Moonshot's official Kimi direct API hosts.
pub fn is_kimi(base_url: &str) -> bool {
    matches!(
        url_host(base_url).as_deref(),
        Some("api.moonshot.cn") | Some("api.moonshot.ai")
    )
}

/// Ollama Cloud's hosted endpoint; local servers intentionally do not match.
pub fn is_ollama_cloud(base_url: &str) -> bool {
    match_vendor_host(base_url, "ollama.com", &["ollama.com"])
}

/// Xiaomi MiMo's OpenAI-compatible API; the apex itself is excluded (Go:
/// `IsMiMoEndpoint`).
pub fn is_mimo(base_url: &str) -> bool {
    url_host(base_url).is_some_and(|h| h != "xiaomimimo.com" && h.ends_with(".xiaomimimo.com"))
}

const DEEPSEEK_IMAGE_MODELS: [&str; 3] = [
    "deepseek-flash",
    "deepseek-v4-flash",
    "deepseek-v4-flash-vision-exp",
];

/// Official DeepSeek models that accept image input (Go:
/// `DeepSeekTakesImages`).
pub fn deepseek_takes_images(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    DEEPSEEK_IMAGE_MODELS.contains(&model.as_str())
}

/// Direct MiniMax-M3.1-Flash API model with always-on, tunable thinking.
pub fn is_minimax_flash(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("MiniMax-M3.1-Flash")
}

/// Supported MiniMax-Flash depth levels, ascending (Go:
/// `MiniMaxFlashEfforts`).
pub const MINIMAX_FLASH_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The official DeepSeek Beta chat endpoint enabling assistant-prefix
/// completion, derived only from an already-DeepSeek-hosted URL (Go:
/// `deepSeekPrefixChatURL`) — custom gateways opting into the DeepSeek wire
/// shape are never bypassed to the vendor's direct API.
pub fn deepseek_prefix_chat_url(chat_url: &str) -> Option<String> {
    if !is_deepseek(chat_url) {
        return None;
    }
    let chat_url = chat_url.trim();
    let (scheme, rest) = chat_url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}/beta/chat/completions"))
}

/// Whether an endpoint override only repeats `base_url` (Go:
/// `EndpointOverrideRepeatsBase`): scheme+host case-insensitive, path equal
/// after trailing-slash trim; a query or fragment is deliberate and stays.
pub fn endpoint_override_repeats_base(override_url: &str, base_url: &str) -> bool {
    let Some((scheme, host, path, has_suffix)) = parse_endpoint(override_url) else {
        return false;
    };
    if has_suffix {
        return false;
    }
    let Some((base_scheme, base_host, base_path, _)) = parse_endpoint(base_url) else {
        return false;
    };
    scheme.eq_ignore_ascii_case(&base_scheme)
        && host.eq_ignore_ascii_case(&base_host)
        && path.trim_end_matches('/') == base_path.trim_end_matches('/')
}

/// (scheme, host, path, query-or-fragment present) of a URL.
fn parse_endpoint(raw: &str) -> Option<(String, String, String, bool)> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let (authority, tail) = match rest.find(['/', '?', '#']) {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if authority.is_empty() {
        return None;
    }
    let has_suffix = tail.contains('?') || tail.contains('#');
    let path = match tail.split_once('?').or_else(|| tail.split_once('#')) {
        Some((p, _)) => p,
        None => tail,
    };
    Some((
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase(),
        path.to_owned(),
        has_suffix,
    ))
}

/// POST target for chat completions: an override that only repeats base_url
/// counts as unset, and POSTing to the API root answers 404 (Go:
/// `resolveChatURL`/`normalizeChatURL`).
pub fn resolve_chat_url(base_url: &str, request_url: &str, chat_url: &str) -> String {
    let request_url = request_url.trim();
    if !request_url.is_empty() && !endpoint_override_repeats_base(request_url, base_url) {
        return request_url.to_owned();
    }
    let legacy = chat_url.trim().trim_end_matches('/');
    if !legacy.is_empty() && !endpoint_override_repeats_base(legacy, base_url) {
        return legacy.to_owned();
    }
    format!("{}/chat/completions", base_url.trim().trim_end_matches('/'))
}

/// Whether an endpoint/model pair speaks Gemini's OpenAI-compatible
/// thought-signature extension (Go: `usesGeminiThoughtSignatures`): the
/// official endpoint, or a gateway routing a gemini-prefixed model id.
pub fn uses_gemini_thought_signatures(base_url: &str, model: &str) -> bool {
    if is_gemini(base_url) {
        return true;
    }
    model
        .trim()
        .to_ascii_lowercase()
        .split(['/', ':'])
        .any(|segment| {
            segment == "gemini" || segment.starts_with("gemini-") || segment.starts_with("gemini_")
        })
}

/// Whether thought signatures ride the request for this endpoint/model.
pub(crate) fn request_carries_thought_signature(base_url: &str, model: &str) -> bool {
    uses_gemini_thought_signatures(base_url, model)
}

const OPEN_CODE_GO_PREFIXES: [&str; 2] = ["/zen/go", "/inference/go"];

/// Whether the URL is on OpenCode Go's gateway (Go: `IsOpenCodeGoEndpoint`).
pub fn is_opencode_go_endpoint(raw_url: &str) -> bool {
    let raw_url = raw_url.trim();
    let Some((_, rest)) = raw_url.split_once("://") else {
        return false;
    };
    let (host, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].to_ascii_lowercase()),
        None => (rest, String::new()),
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if !host
        .rsplit_once(':')
        .map_or(host, |(h, p)| {
            if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
                h
            } else {
                host
            }
        })
        .eq_ignore_ascii_case("opencode.ai")
    {
        return false;
    }
    OPEN_CODE_GO_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}
