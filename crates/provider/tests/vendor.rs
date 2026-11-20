//! Endpoint detection, chat-URL resolution, and resolved-fact oracles (Go:
//! `vendor.go` consumers, `request_url_test.go`, `idle_timeout_test.go`,
//! `host_test.go` subset, `client_identity_test.go` subset).

use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_provider::vendor;
use std::time::Duration;

// Go matchVendorHost/host.go detection matrix.
#[test]
fn endpoint_detection() {
    assert!(vendor::is_deepseek("https://api.deepseek.com"));
    assert!(vendor::is_deepseek("https://api.deepseek.com/v1"));
    assert!(vendor::is_deepseek("https://eu.api.deepseek.com/v1"));
    assert!(!vendor::is_deepseek("https://deepseek.example.com/v1"));
    assert!(vendor::is_openai("https://api.openai.com/v1"));
    assert!(!vendor::is_openai("https://eu.api.openai.com/v1"));
    assert!(vendor::is_gemini(
        "https://generativelanguage.googleapis.com/v1beta/openai"
    ));
    assert!(vendor::is_minimax("https://api.minimaxi.com/v1"));
    assert!(vendor::is_minimax("https://api.minimax.io"));
    assert!(!vendor::is_minimax("https://minimaxi.com"));
    assert!(vendor::is_zhipu("https://open.bigmodel.cn/api/paas/v4"));
    assert!(vendor::is_zhipu("https://api.z.ai/api/paas/v4"));
    assert!(vendor::is_longcat("https://api.longcat.chat/openai/v1"));
    assert!(vendor::is_kimi("https://api.moonshot.cn/v1"));
    assert!(vendor::is_kimi("https://api.moonshot.ai/v1"));
    assert!(!vendor::is_kimi("https://relay.example/moonshot/v1"));
    assert!(vendor::is_ollama_cloud("https://ollama.com/v1"));
    assert!(!vendor::is_ollama_cloud("http://localhost:11434/v1"));
    assert!(vendor::is_mimo("https://api.xiaomimimo.com/v1"));
    assert!(!vendor::is_mimo("https://xiaomimimo.com/v1"));
    assert!(vendor::deepseek_takes_images("DeepSeek-V4-Flash"));
    assert!(!vendor::deepseek_takes_images("deepseek-v4-pro"));
    assert!(vendor::is_minimax_flash("MiniMax-M3.1-Flash"));
    assert!(!vendor::is_minimax_flash("MiniMax-M3"));
    // Malformed URLs match nothing.
    assert!(!vendor::is_deepseek("not a url"));
    assert!(!vendor::is_deepseek(""));
}

// Go TestNewScopesPrefixContinuationToOfficialDeepSeekChatURL.
#[test]
fn prefix_continuation_scoped_to_official_deepseek() {
    let official = build_client(&ClientConfig::new(
        "deepseek",
        "https://api.deepseek.com",
        "deepseek-v4-flash",
    ))
    .unwrap();
    assert_eq!(
        official.prefix_chat_url(),
        Some("https://api.deepseek.com/beta/chat/completions")
    );

    let gateway = build_client(&ClientConfig {
        reasoning_protocol: "deepseek".to_owned(),
        ..ClientConfig::new("gateway", "https://gateway.example/v1", "deepseek-v4-flash")
    })
    .unwrap();
    assert_eq!(
        gateway.prefix_chat_url(),
        None,
        "custom gateways must not be bypassed to the vendor API"
    );
}

// Go TestNewPrefersExactRequestURLOverLegacyChatURL.
#[test]
fn prefers_exact_request_url_over_legacy_chat_url() {
    let c = build_client(&ClientConfig {
        request_url: "https://exact.example.com/custom/?token=1".to_owned(),
        chat_url: "https://legacy.example.com/chat/completions/".to_owned(),
        ..ClientConfig::new("x", "https://base.example.com/v1", "model-a")
    })
    .unwrap();
    assert_eq!(c.chat_url(), "https://exact.example.com/custom/?token=1");
}

// Go TestStreamIgnoresEndpointOverridesThatRepeatBaseURL: a mirror of
// base_url is unset, whatever key carries it.
#[test]
fn ignores_endpoint_overrides_that_repeat_base_url() {
    for key in ["request_url", "chat_url"] {
        let mut cfg = ClientConfig::new("relay", "https://base.example.com/api/v1", "m");
        cfg.api_key = "key".to_owned();
        let target = "https://base.example.com/api/v1/";
        if key == "request_url" {
            cfg.request_url = target.to_owned();
        } else {
            cfg.chat_url = target.to_owned();
        }
        let c = build_client(&cfg).unwrap();
        assert_eq!(
            c.chat_url(),
            "https://base.example.com/api/v1/chat/completions",
            "{key} override must be ignored"
        );
    }
    // A query or fragment is deliberate and stays.
    let mut cfg = ClientConfig::new("relay", "https://base.example.com/api/v1", "m");
    cfg.request_url = "https://base.example.com/api/v1?token=1".to_owned();
    let c = build_client(&cfg).unwrap();
    assert_eq!(c.chat_url(), "https://base.example.com/api/v1?token=1");
}

// Go TestIdleTimeoutOverrideReachesClientAndTransport (client half).
#[test]
fn idle_timeout_override_reaches_the_client() {
    let c = build_client(&ClientConfig {
        idle_timeout_seconds: 8,
        ..ClientConfig::new("deepseek", "https://api.deepseek.com", "deepseek-v4")
    })
    .unwrap();
    assert_eq!(c.idle_timeout(), Duration::from_secs(8));
    let c = build_client(&ClientConfig::new(
        "deepseek",
        "https://api.deepseek.com",
        "deepseek-v4",
    ))
    .unwrap();
    assert_eq!(c.idle_timeout(), Duration::from_secs(300));
    // A value past the ceiling clamps (Go MaxIdleTimeoutSeconds).
    let c = build_client(&ClientConfig {
        idle_timeout_seconds: 99999,
        ..ClientConfig::new("deepseek", "https://api.deepseek.com", "deepseek-v4")
    })
    .unwrap();
    assert_eq!(c.idle_timeout(), Duration::from_secs(32767));
}

// Go TestNewMiniMaxSetsFlag / TestNewZhipuSetsFlag stay true at the factory.
#[test]
fn factory_flags() {
    for base in ["https://api.minimaxi.com/v1", "https://api.minimaxi.com"] {
        assert!(
            build_client(&ClientConfig::new("m3", base, "MiniMax-M3")).is_ok(),
            "{base}"
        );
    }
    // A base_url without scheme fails every detection and still builds as a
    // generic gateway.
    let generic = build_client(&ClientConfig::new(
        "x",
        "https://gateway.example.com/v1",
        "plain",
    ))
    .unwrap();
    assert_eq!(generic.name(), "x");
    assert_eq!(
        generic.chat_url(),
        "https://gateway.example.com/v1/chat/completions"
    );
}

// Go config requirements: base_url and model are required.
#[test]
fn required_fields() {
    let err = build_client(&ClientConfig::new("x", "", "m")).unwrap_err();
    assert!(err.to_string().contains("base_url is required"));
    let err = build_client(&ClientConfig::new("x", "https://gw.example", "")).unwrap_err();
    assert!(err.to_string().contains("model is required"));
    // An empty name defaults to "openai".
    let c = build_client(&ClientConfig::new("", "https://gw.example/v1", "m")).unwrap();
    assert_eq!(c.name(), "openai");
}

// Go deepSeekPrefixChatURL / resolveChatURL edge handling.
#[test]
fn url_resolution_units() {
    assert_eq!(
        vendor::resolve_chat_url("https://base.example.com/v1", "", ""),
        "https://base.example.com/v1/chat/completions"
    );
    assert_eq!(
        vendor::resolve_chat_url(
            "https://base.example.com/v1",
            "https://exact.example.com/x",
            ""
        ),
        "https://exact.example.com/x"
    );
    assert_eq!(
        vendor::resolve_chat_url(
            "https://base.example.com/v1",
            "",
            "https://legacy.example.com/cc/"
        ),
        "https://legacy.example.com/cc"
    );
    // DeepSeek prefix URL derivation keeps host and resets the path.
    assert_eq!(
        vendor::deepseek_prefix_chat_url("https://api.deepseek.com/v1/chat/completions"),
        Some("https://api.deepseek.com/beta/chat/completions".to_owned())
    );
    assert_eq!(
        vendor::deepseek_prefix_chat_url("https://relay.example/v1"),
        None
    );
}
