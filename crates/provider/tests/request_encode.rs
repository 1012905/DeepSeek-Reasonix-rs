//! Request-encoding oracles (Go: `openai/openai_test.go` buildRequest family
//! and `effort_*_test.go`). The wire bytes are the parity surface.

use reasonix_contract::provider::{Message, Role, ToolCall};
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_provider::error::ProviderError;
use reasonix_provider::{ChatCompletionRequest, ToolSchema};

fn client(cfg: ClientConfig) -> reasonix_provider::Client {
    build_client(&cfg).expect("build client")
}

fn client_err(cfg: ClientConfig) -> ProviderError {
    build_client(&cfg).expect_err("build client should fail")
}

/// The request body of `req` as a String, trailing newline included.
fn body(client: &reasonix_provider::Client, req: &ChatCompletionRequest) -> String {
    String::from_utf8(client.request_bytes(req).expect("marshal")).unwrap()
}

fn user(content: &str) -> Message {
    Message {
        role: Role::USER,
        content: content.to_owned(),
        ..Message::default()
    }
}

fn tool_schema(name: &str, parameters: &str) -> ToolSchema {
    ToolSchema {
        name: name.to_owned(),
        description: String::new(),
        parameters: parameters.to_owned(),
    }
}

// Go TestBuildRequestOrdinaryDeepSeekBytesStayPrefixFree — byte-exact. The Go
// oracle builds the client directly (no auto output ladder), which a
// DeepSeek-protocol gateway replicates: official DeepSeek adds the automatic
// 64K budget (covered by provider_specific_output_budget).
#[test]
fn ordinary_deepseek_request_bytes_stay_prefix_free() {
    let c = client(ClientConfig {
        effort: "high".to_owned(),
        reasoning_protocol: "deepseek".to_owned(),
        ..ClientConfig::new(
            "deepseek",
            "https://gateway.example/v1",
            "deepseek-v4-flash",
        )
    });
    let req = ChatCompletionRequest {
        messages: vec![user("hi")],
        ..ChatCompletionRequest::default()
    };
    let want = r#"{"model":"deepseek-v4-flash","messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true},"reasoning_effort":"high","thinking":{"type":"enabled"}}"#;
    assert_eq!(
        body(&c, &req),
        format!("{want}\n"),
        "ordinary DeepSeek request bytes changed"
    );
    assert!(
        !body(&c, &req).contains(r#""prefix""#),
        "ordinary request leaked prefix mode"
    );
}

// Go TestBuildRequestAlwaysSerializesContent: every message carries content;
// a pure tool_calls assistant turn serializes null.
#[test]
fn always_serializes_content() {
    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            user("list the files"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "ls".to_owned(),
                    arguments: r#"{"path":"."}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                content: "main.go".to_owned(),
                tool_call_id: "call_1".to_owned(),
                name: "ls".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    for (i, m) in messages.iter().enumerate() {
        assert!(
            m.get("content").is_some(),
            "messages[{i}] lost content: {json}"
        );
    }
    assert_eq!(messages[1]["content"], serde_json::Value::Null);
    assert!(
        messages[1].get("tool_calls").is_some(),
        "assistant lost tool_calls: {json}"
    );
}

// Go TestBuildRequestOmitsResolvedToolCallMetadata.
#[test]
fn omits_resolved_tool_call_metadata() {
    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let req = ChatCompletionRequest {
        messages: vec![Message {
            role: Role::ASSISTANT,
            tool_calls: vec![ToolCall {
                id: "call_1".to_owned(),
                name: "use_capability".to_owned(),
                arguments: "{}".to_owned(),
                resolved_name: "mcp__db__write".to_owned(),
                capability_id: "mcp-tool:db/write".to_owned(),
                resolved_read_only: Some(false),
                ..ToolCall::default()
            }],
            ..Message::default()
        }],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    for forbidden in [
        "resolved_name",
        "resolvedName",
        "capability_id",
        "capabilityId",
        "resolved_read_only",
        "resolvedReadOnly",
        "mcp__db__write",
    ] {
        assert!(
            !json.contains(forbidden),
            "provider request leaked local tool metadata {forbidden:?}: {json}"
        );
    }
    assert!(json.contains(r#""name":"use_capability""#));
}

// Go TestToolResultEmptyNameStillSerialized (#4711/#4727).
#[test]
fn tool_result_empty_name_still_serialized() {
    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: String::new(),
                    arguments: "{}".to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: "call_1".to_owned(),
                name: String::new(),
                content: "file contents".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["role"], "tool");
    let name = messages[1].get("name").expect("tool message lost name key");
    assert_eq!(name, "", "tool message name must serialize empty");
    assert!(
        messages[0].get("name").is_none(),
        "non-tool message leaked name key: {json}"
    );
}

// Go TestBuildRequestDropsReasoningContent: a plain assistant turn never
// echoes reasoning_content back.
#[test]
fn drops_reasoning_on_plain_assistant_turn() {
    let c = client(ClientConfig::new(
        "x",
        "https://api.deepseek.com",
        "deepseek-reasoner",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            user("explain"),
            Message {
                role: Role::ASSISTANT,
                content: "the answer".to_owned(),
                reasoning_content: "SECRET-CHAIN-OF-THOUGHT".to_owned(),
                ..Message::default()
            },
            user("thanks"),
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(
        !json.contains("reasoning_content"),
        "no-tool-calls assistant turn carried reasoning_content: {json}"
    );
    assert!(!json.contains("SECRET-CHAIN-OF-THOUGHT"));
    assert!(json.contains("the answer"));
}

// Go TestBuildRequestDropsLocalMetadata.
#[test]
fn drops_local_metadata() {
    let c = client(ClientConfig::new(
        "x",
        "https://api.deepseek.com",
        "deepseek-chat",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            user("continue"),
            Message {
                role: Role::USER,
                content: "edited prompt".to_owned(),
                edited: true,
                original: "original prompt".to_owned(),
                ..Message::default()
            },
            Message {
                role: Role::ASSISTANT,
                content: "done".to_owned(),
                work_duration_ms: 24_000,
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    for forbidden in [
        "memoryCitations",
        "MEMORY.md",
        "workDurationMs",
        "work_duration_ms",
        "original prompt",
        "\"edited\"",
        "\"original\"",
    ] {
        assert!(!json.contains(forbidden), "local metadata leaked: {json}");
    }
    assert!(json.contains("done"));
}

// Go TestBuildRequestRoundTripsReasoningOnDeepSeekToolCalls — DeepSeek only.
#[test]
fn round_trips_reasoning_on_deepseek_tool_calls() {
    let messages = vec![
        user("count the go files"),
        Message {
            role: Role::ASSISTANT,
            reasoning_content: "CHAIN-OF-THOUGHT".to_owned(),
            tool_calls: vec![ToolCall {
                id: "c1".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"ls"}"#.to_owned(),
                ..ToolCall::default()
            }],
            ..Message::default()
        },
        Message {
            role: Role::TOOL,
            content: "14".to_owned(),
            tool_call_id: "c1".to_owned(),
            name: "bash".to_owned(),
            ..Message::default()
        },
    ];
    let deepseek = client(ClientConfig::new(
        "x",
        "https://api.deepseek.com",
        "deepseek-v4",
    ));
    let json = body(
        &deepseek,
        &ChatCompletionRequest {
            messages: messages.clone(),
            ..Default::default()
        },
    );
    assert!(
        json.contains("reasoning_content") && json.contains("CHAIN-OF-THOUGHT"),
        "DeepSeek tool_calls turn must round-trip reasoning_content: {json}"
    );
    let other = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "mimo-v2",
    ));
    let json = body(
        &other,
        &ChatCompletionRequest {
            messages,
            ..Default::default()
        },
    );
    assert!(
        !json.contains("CHAIN-OF-THOUGHT"),
        "non-DeepSeek backends must not re-upload reasoning_content: {json}"
    );
}

// Go TestBuildRequestAlwaysSendsReasoningKeyOnDeepSeekToolCalls: a tool_calls
// turn whose reasoning was lost still serializes the key as "".
#[test]
fn always_sends_reasoning_key_on_deepseek_tool_calls() {
    let c = client(ClientConfig {
        reasoning_protocol: "deepseek".to_owned(),
        ..ClientConfig::new(
            "deepseek-proxy",
            "https://api.deepseek.com",
            "deepseek-v4-pro",
        )
    });
    let req = ChatCompletionRequest {
        messages: vec![
            user("inspect"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"main.go"}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                content: "package main".to_owned(),
                tool_call_id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                ..Message::default()
            },
            Message {
                role: Role::ASSISTANT,
                content: "plain text turn".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    let rc = messages[1]
        .get("reasoning_content")
        .expect("tool_calls turn must serialize the reasoning key");
    assert_eq!(rc, "");
    assert!(
        messages[3].get("reasoning_content").is_none(),
        "plain assistant turn must keep omitting reasoning_content"
    );
}

// Go TestBuildRequestForwardsReasoningEffort.
#[test]
fn forwards_reasoning_effort() {
    let c = client(ClientConfig {
        effort: "high".to_owned(),
        ..ClientConfig::new("x", "https://gateway.example/v1", "mimo-v2")
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"high""#), "{json}");

    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(
        !json.contains("reasoning_effort"),
        "empty effort must be omitted: {json}"
    );
}

// Go TestBuildRequestTemperatureSerialization.
#[test]
fn temperature_serialization() {
    let c = client(ClientConfig::new("x", "https://gateway.example/v1", "m"));
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(
        !json.contains("temperature"),
        "unset temperature serialized: {json}"
    );

    let json = body(
        &c,
        &ChatCompletionRequest {
            temperature: Some(0.0),
            ..ChatCompletionRequest::default()
        },
    );
    assert!(
        json.contains(r#""temperature":0"#),
        "zero temperature: {json}"
    );

    let json = body(
        &c,
        &ChatCompletionRequest {
            temperature: Some(0.25),
            ..ChatCompletionRequest::default()
        },
    );
    assert!(json.contains(r#""temperature":0.25"#), "nonzero: {json}");
}

// Go TestBuildRequestDeepSeekThinking + TestBuildRequestDeepSeekDisabled +
// TestBuildRequestDeepSeekPreservesCallerTemperature.
#[test]
fn deepseek_thinking_levels() {
    let build = |effort: &str| {
        client(ClientConfig {
            effort: effort.to_owned(),
            ..ClientConfig::new("x", "https://api.deepseek.com", "deepseek-v4")
        })
    };
    for (effort, want_thinking, want_reasoning) in
        [("high", "enabled", "high"), ("max", "enabled", "max")]
    {
        let json = body(&build(effort), &ChatCompletionRequest::default());
        assert!(
            json.contains(&format!(r#""thinking":{{"type":"{want_thinking}"}}"#)),
            "thinking wrong for {effort}: {json}"
        );
        assert!(
            json.contains(&format!(r#""reasoning_effort":"{want_reasoning}""#)),
            "depth wrong for {effort}: {json}"
        );
    }
    let json = body(&build(""), &ChatCompletionRequest::default());
    assert!(
        !json.contains(r#""temperature""#),
        "unset temperature: {json}"
    );
    assert!(json.contains(r#""thinking":{"type":"enabled"}"#), "{json}");

    for name in ["effort-disabled", "thinking-disabled"] {
        let c = match name {
            "effort-disabled" => build("disabled"),
            _ => client(ClientConfig {
                thinking: "disabled".to_owned(),
                ..ClientConfig::new("x", "https://api.deepseek.com", "deepseek-v4")
            }),
        };
        let json = body(&c, &ChatCompletionRequest::default());
        assert!(
            json.contains(r#""thinking":{"type":"disabled"}"#),
            "{name}: {json}"
        );
        assert!(!json.contains("reasoning_effort"), "{name}: {json}");
    }
    // effort=off is a retired level; it falls back to the default depth.
    let c = client(ClientConfig {
        effort: "off".to_owned(),
        ..ClientConfig::new("x", "https://api.deepseek.com", "deepseek-v4")
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"high""#), "{json}");
}

// Go TestBuildRequestDeepSeekDisabled: disabled mode omits reasoning_content
// on a reasoning-less tool_calls turn but keeps round-tripping saved reasoning.
#[test]
fn deepseek_disabled_keeps_history_bytes() {
    let c = client(ClientConfig {
        effort: "disabled".to_owned(),
        ..ClientConfig::new("ds", "https://api.deepseek.com", "deepseek-v4")
    });
    let req = ChatCompletionRequest {
        messages: vec![
            user("inspect"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"main.go"}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                content: "package main".to_owned(),
                tool_call_id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                ..Message::default()
            },
            Message {
                role: Role::ASSISTANT,
                reasoning_content: "from a thinking round".to_owned(),
                tool_calls: vec![ToolCall {
                    id: "call_2".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"go.mod"}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                content: "module demo".to_owned(),
                tool_call_id: "call_2".to_owned(),
                name: "read_file".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(value["thinking"], serde_json::json!({"type":"disabled"}));
    assert!(!json.contains("reasoning_effort"), "{json}");
    assert!(
        messages[1].get("reasoning_content").is_none(),
        "disabled mode must omit reasoning_content on a reasoning-less tool_calls turn: {json}"
    );
    assert_eq!(
        messages[3]["reasoning_content"], "from a thinking round",
        "disabled mode must keep round-tripping thinking-round reasoning"
    );
}

// Go TestBuildRequestMiniMaxThinking + TestBuildRequestZhipuThinking +
// TestBuildRequestGenericThinking + TestBuildRequestNonDeepSeekOmitsThinking.
#[test]
fn binary_thinking_dialects() {
    for (base, model, effort, want) in [
        ("https://api.minimaxi.com/v1", "MiniMax-M3", "", "adaptive"),
        (
            "https://api.minimaxi.com/v1",
            "MiniMax-M3",
            "adaptive",
            "adaptive",
        ),
        (
            "https://api.minimaxi.com/v1",
            "MiniMax-M3",
            "disabled",
            "disabled",
        ),
        ("https://api.z.ai/api/paas/v4", "glm-4.5-air", "", "enabled"),
        (
            "https://api.z.ai/api/paas/v4",
            "glm-4.5-air",
            "enabled",
            "enabled",
        ),
        (
            "https://api.z.ai/api/paas/v4",
            "glm-4.5-air",
            "disabled",
            "disabled",
        ),
    ] {
        let c = client(ClientConfig {
            effort: effort.to_owned(),
            ..ClientConfig::new("x", base, model)
        });
        let json = body(&c, &ChatCompletionRequest::default());
        assert!(
            json.contains(&format!(r#""thinking":{{"type":"{want}"}}"#)),
            "{model}/{effort}: {json}"
        );
        assert!(
            !json.contains("reasoning_effort"),
            "{model} must not send reasoning_effort: {json}"
        );
    }
    // Generic thinking config field.
    let c = client(ClientConfig {
        thinking: "enabled".to_owned(),
        ..ClientConfig::new("x", "https://gateway.example/v1", "some-model")
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""thinking":{"type":"enabled"}"#), "{json}");
    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "some-model",
    ));
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(!json.contains("\"thinking\""), "unset thinking: {json}");
    // Generic OpenAI-compatible provider with effort: no thinking knob.
    let c = client(ClientConfig {
        effort: "high".to_owned(),
        ..ClientConfig::new("x", "https://gateway.example/v1", "mimo-v2")
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(!json.contains("\"thinking\""), "{json}");
    assert!(json.contains(r#""reasoning_effort":"high""#));
}

// Go TestZhipuDepthRequestByModel — the documented depth contract per model.
#[test]
fn zhipu_depth_request_by_model() {
    let cases: [(&str, &str, &str, &str, &str); 7] = [
        ("glm-5.2", "high", "", "enabled", "high"),
        ("glm-5.2", "none", "", "disabled", ""),
        ("glm-5.2", "disabled", "", "disabled", ""),
        ("glm-5.3", "low", "", "enabled", "low"),
        ("glm-5.3-flash", "max", "", "enabled", "max"),
        ("glm-5.3", "disabled", "", "enabled", "low"),
        ("glm-5.3", "max", "disabled", "enabled", "low"),
    ];
    for (model, effort, thinking, want_thinking, want_depth) in cases {
        let c = client(ClientConfig {
            effort: effort.to_owned(),
            thinking: thinking.to_owned(),
            ..ClientConfig::new("glm", "https://api.z.ai/api/paas/v4", model)
        });
        let json = body(&c, &ChatCompletionRequest::default());
        if want_depth.is_empty() {
            assert!(
                !json.contains("reasoning_effort"),
                "{model}/{effort}/{thinking}: {json}"
            );
        } else {
            assert!(
                json.contains(&format!(r#""reasoning_effort":"{want_depth}""#)),
                "{model}/{effort}/{thinking}: {json}"
            );
        }
        assert!(
            json.contains(&format!(r#""thinking":{{"type":"{want_thinking}"}}"#)),
            "{model}/{effort}/{thinking}: {json}"
        );
    }
    // GLM-5.3 rejects unsupported levels; a max config accepts a low override.
    let err = client_err(ClientConfig {
        effort: "medium".to_owned(),
        ..ClientConfig::new("glm", "https://api.z.ai/api/paas/v4", "glm-5.3")
    });
    assert!(matches!(err, ProviderError::Config { .. }));
    let c = client(ClientConfig {
        effort: "max".to_owned(),
        ..ClientConfig::new("glm", "https://api.z.ai/api/paas/v4", "glm-5.3")
    });
    let json = body(
        &c,
        &ChatCompletionRequest {
            effort_override: "low".to_owned(),
            ..ChatCompletionRequest::default()
        },
    );
    assert!(json.contains(r#""reasoning_effort":"low""#), "{json}");
}

// Go TestZhipuThinkingOffLevelIgnoresDepthOverride.
#[test]
fn zhipu_thinking_off_ignores_depth_override() {
    for off in ["none", "minimal"] {
        let c = client(ClientConfig {
            effort: off.to_owned(),
            ..ClientConfig::new("glm", "https://api.z.ai/api/paas/v4", "glm-5.2")
        });
        assert!(c.request_efforts().is_empty(), "{off} must take no depths");
        let json = body(
            &c,
            &ChatCompletionRequest {
                effort_override: "high".to_owned(),
                ..ChatCompletionRequest::default()
            },
        );
        assert!(json.contains(r#""thinking":{"type":"disabled"}"#), "{json}");
        assert!(!json.contains("reasoning_effort"), "{json}");
    }
    let c = client(ClientConfig {
        effort: "high".to_owned(),
        ..ClientConfig::new("glm", "https://api.z.ai/api/paas/v4", "glm-5.2")
    });
    let json = body(
        &c,
        &ChatCompletionRequest {
            effort_override: "max".to_owned(),
            ..ChatCompletionRequest::default()
        },
    );
    assert!(json.contains(r#""reasoning_effort":"max""#), "{json}");
}

// Go TestDeclaredGLMProtocolOnARelayTakesTheDepthContract.
#[test]
fn declared_glm_protocol_on_relay_takes_depth_contract() {
    let cases: [(&str, &str, &str, &str); 4] = [
        ("glm-5.3", "glm", "high", "high"),
        ("glm-5.3", "glm", "disabled", "low"),
        ("glm-5.2", "glm", "none", ""),
        ("glm-4.5", "glm", "disabled", ""),
    ];
    for (model, protocol, effort, want_depth) in cases {
        let c = client(ClientConfig {
            reasoning_protocol: protocol.to_owned(),
            effort: effort.to_owned(),
            ..ClientConfig::new("relay", "https://relay.example/v1", model)
        });
        let json = body(&c, &ChatCompletionRequest::default());
        if want_depth.is_empty() {
            assert!(
                !json.contains("reasoning_effort"),
                "{model}/{effort}: {json}"
            );
        } else {
            assert!(
                json.contains(&format!(r#""reasoning_effort":"{want_depth}""#)),
                "{model}/{effort}: {json}"
            );
        }
    }
    let err = client_err(ClientConfig {
        reasoning_protocol: "glm".to_owned(),
        effort: "medium".to_owned(),
        ..ClientConfig::new("relay", "https://relay.example/v1", "glm-5.3")
    });
    assert!(matches!(err, ProviderError::Config { .. }));
    // A relay declared openai must not take the GLM contract.
    build_client(&ClientConfig {
        reasoning_protocol: "openai".to_owned(),
        effort: "medium".to_owned(),
        ..ClientConfig::new("relay", "https://relay.example/v1", "glm-5.3")
    })
    .expect("openai protocol must accept medium");
}

// Go TestBuildRequestRoundTripsGLMReasoningHistory.
#[test]
fn round_trips_glm_reasoning_history() {
    let build = |effort: &str| {
        client(ClientConfig {
            reasoning_protocol: "glm".to_owned(),
            effort: effort.to_owned(),
            ..ClientConfig::new("glm-gateway", "https://tokenrhythm.studio/v1", "glm-5.2")
        })
    };
    let req = ChatCompletionRequest {
        messages: vec![
            user("inspect"),
            Message {
                role: Role::ASSISTANT,
                reasoning_content: "read main.go first".to_owned(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"main.go"}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                content: "package main".to_owned(),
                tool_call_id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                ..Message::default()
            },
            user("continue"),
            Message {
                role: Role::ASSISTANT,
                content: "done".to_owned(),
                reasoning_content: "combine the result".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    for effort in ["enabled", "disabled"] {
        let c = build(effort);
        assert!(
            !c.requires_tool_call_reasoning(),
            "{effort}: no DeepSeek recovery policy"
        );
        let json = body(&c, &req);
        assert!(
            json.contains("read main.go first") && json.contains("combine the result"),
            "{effort}: complete reasoning history must replay: {json}"
        );
    }
    let enabled = build("enabled");
    assert!(enabled.requires_reasoning_round_trip());
    let disabled = build("disabled");
    assert!(!disabled.requires_reasoning_round_trip());
}

// Go TestBuildRequestKimiK3OfficialWireShape.
#[test]
fn kimi_k3_official_wire_shape() {
    let mut cfg = ClientConfig::new("kimi-cn", "https://api.moonshot.cn/v1", "kimi-k3");
    cfg.effort = "max".to_owned();
    cfg.supported_efforts = vec!["low".into(), "high".into(), "max".into()];
    cfg.reasoning_protocol = "openai".to_owned();
    cfg.extra_body
        .insert("top_p".into(), serde_json::json!(0.5));
    cfg.extra_body.insert("n".into(), serde_json::json!(2));
    cfg.extra_body
        .insert("presence_penalty".into(), serde_json::json!(1));
    cfg.extra_body
        .insert("frequency_penalty".into(), serde_json::json!(1));
    cfg.extra_body
        .insert("max_completion_tokens".into(), serde_json::json!(99));
    cfg.extra_body
        .insert("trace_id".into(), serde_json::json!("keep-me"));
    let c = client(cfg);
    assert!(c.requires_reasoning_round_trip());
    let req = ChatCompletionRequest {
        temperature: Some(0.0),
        max_tokens: 2000,
        messages: vec![
            user("first"),
            Message {
                role: Role::ASSISTANT,
                content: "answer".to_owned(),
                reasoning_content: "provider reasoning".to_owned(),
                ..Message::default()
            },
            user("use a tool"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call-1".to_owned(),
                    name: "lookup".to_owned(),
                    arguments: "{}".to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: "call-1".to_owned(),
                name: "lookup".to_owned(),
                content: "result".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    for field in [
        "temperature",
        "max_tokens",
        "top_p",
        "n",
        "presence_penalty",
        "frequency_penalty",
    ] {
        assert!(value.get(field).is_none(), "K3 must omit {field}: {json}");
    }
    assert_eq!(value["max_completion_tokens"], 2000);
    assert_eq!(value["trace_id"], "keep-me");
    assert_eq!(value["reasoning_effort"], "max");
    let messages = value["messages"].as_array().unwrap();
    assert_eq!(messages[1]["reasoning_content"], "provider reasoning");
    assert_eq!(messages[3]["reasoning_content"], "");
    // A relay carrying the same model id keeps its own wire shape.
    let mut relay = ClientConfig::new("opencode-go", "https://opencode.ai/zen/go/v1", "kimi-k3");
    relay.effort = "max".to_owned();
    relay.supported_efforts = vec!["high".into(), "max".into()];
    let relay = client(relay);
    assert!(!relay.requires_reasoning_round_trip());
    let json = body(
        &relay,
        &ChatCompletionRequest {
            temperature: Some(0.0),
            max_tokens: 77,
            ..ChatCompletionRequest::default()
        },
    );
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(value["temperature"], 0.0);
    assert_eq!(value["max_tokens"], 77);
    assert!(value.get("max_completion_tokens").is_none(), "{json}");
}

// Go TestBuildRequestUsesProviderSpecificOutputBudget.
#[test]
fn provider_specific_output_budget() {
    let budget = |base: &str, model: &str, max: i64, effort: &str| {
        let mut cfg = ClientConfig::new("test", base, model);
        cfg.max_output_tokens = max;
        cfg.effort = effort.to_owned();
        let json = body(&client(cfg), &ChatCompletionRequest::default());
        let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
        (
            value
                .get("max_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            value
                .get("max_completion_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        )
    };
    // Official DeepSeek defaults to high effort, so the automatic ladder
    // lands on the 64K high-reasoning tier.
    assert_eq!(
        budget("https://api.deepseek.com", "deepseek-v4-flash", 0, ""),
        (65536, 0)
    );
    assert_eq!(
        budget("https://api.deepseek.com", "deepseek-v4-flash", 0, "low"),
        (32768, 0)
    );
    assert_eq!(
        budget("https://api.deepseek.com", "deepseek-v4-pro", 0, "disabled"),
        (16384, 0)
    );
    assert_eq!(
        budget(
            "https://api.deepseek.com",
            "deepseek-v4-pro",
            8192,
            "disabled"
        ),
        (8192, 0)
    );
    assert_eq!(
        budget("https://api.openai.com/v1", "o3", 8192, ""),
        (0, 8192)
    );
    assert_eq!(
        budget("https://gateway.example/v1", "plain-chat", 8192, ""),
        (8192, 0)
    );
    assert_eq!(
        budget("https://gateway.example/v1", "plain-chat", 0, ""),
        (0, 0)
    );
    // A disabled budget (-1) sends nothing.
    assert_eq!(
        budget("https://api.deepseek.com", "deepseek-v4-flash", -1, ""),
        (0, 0)
    );
}

// Go TestBuildRequestScopesGeminiThoughtSignature.
#[test]
fn scopes_gemini_thought_signature() {
    let req = ChatCompletionRequest {
        messages: vec![Message {
            role: Role::ASSISTANT,
            tool_calls: vec![ToolCall {
                id: "call_abc123".to_owned(),
                name: "write_file".to_owned(),
                arguments: r#"{"path":"test.txt"}"#.to_owned(),
                thought_signature: "gemini_sig_xyz789".to_owned(),
                ..ToolCall::default()
            }],
            ..Message::default()
        }],
        ..ChatCompletionRequest::default()
    };
    for (base, model, want) in [
        (
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "custom-alias",
            "gemini_sig_xyz789",
        ),
        (
            "https://openrouter.ai/api/v1",
            "google/gemini-3.1-pro",
            "gemini_sig_xyz789",
        ),
        ("https://api.deepseek.com/v1", "deepseek-chat", ""),
    ] {
        let c = client(ClientConfig::new("x", base, model));
        let json = body(&c, &req);
        let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
        let tc = &value["messages"][0]["tool_calls"][0];
        let got = tc
            .pointer("/extra_content/google/thought_signature")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(got, want, "{base}/{model}: {json}");
        if want.is_empty() {
            assert!(tc.get("extra_content").is_none(), "{json}");
        }
        assert!(
            tc.pointer("/function/thought_signature").is_none(),
            "legacy function.thought_signature must not be sent: {json}"
        );
    }
}

// Go TestBuildRequestContentNullForAssistantToolCalls +
// TestBuildRequestOmitsResponseOnlyToolCallIndex +
// TestBuildRequestDefaultsEmptyToolParameters +
// TestBuildRequestReplaysNoArgumentToolCallAsEmptyObject.
#[test]
fn tool_wire_shapes() {
    let c = client(ClientConfig::new("x", "https://api.example.com/v1", "m"));
    let req = ChatCompletionRequest {
        messages: vec![
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "ls".to_owned(),
                    arguments: "{}".to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: "c1".to_owned(),
                name: "ls".to_owned(),
                content: String::new(),
                ..Message::default()
            },
            Message {
                role: Role::ASSISTANT,
                content: "all done".to_owned(),
                ..Message::default()
            },
        ],
        tools: vec![tool_schema("noargs", "")],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(json.contains(r#""tool_calls""#));
    assert!(json.contains(r#""content":null"#));
    assert!(json.contains(r#"{"role":"tool","content":""#));
    assert!(json.contains(r#""content":"all done""#));
    assert!(
        json.contains(r#""parameters":{"properties":{},"required":[],"type":"object"}"#),
        "no-param tool must serialize the strict empty-object schema: {json}"
    );
    assert!(
        !json.contains(r#""index""#),
        "response-only index leaked: {json}"
    );

    // An empty-arguments call replays as an empty object.
    let req = ChatCompletionRequest {
        messages: vec![
            user("go"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "c1".to_owned(),
                    name: "status".to_owned(),
                    arguments: String::new(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: "c1".to_owned(),
                name: "status".to_owned(),
                content: "ok".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(
        json.contains(r#""arguments":"{}""#),
        "replayed arguments are not an empty object: {json}"
    );
}

// Go TestBuildRequestPreservesEmptyIDToolResults: sanitize_tool_pairing keys
// on tool_call_id, but non-distinct ids pair positionally, so every result
// survives.
#[test]
fn preserves_empty_id_tool_results() {
    let c = client(ClientConfig::new(
        "x",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            user("scan"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![
                    ToolCall {
                        id: String::new(),
                        name: "read_file".to_owned(),
                        arguments: r#"{"p":"a"}"#.to_owned(),
                        ..ToolCall::default()
                    },
                    ToolCall {
                        id: String::new(),
                        name: "read_file".to_owned(),
                        arguments: r#"{"p":"b"}"#.to_owned(),
                        ..ToolCall::default()
                    },
                ],
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: String::new(),
                name: "read_file".to_owned(),
                content: "RESULT-A".to_owned(),
                ..Message::default()
            },
            Message {
                role: Role::TOOL,
                tool_call_id: String::new(),
                name: "read_file".to_owned(),
                content: "RESULT-B".to_owned(),
                ..Message::default()
            },
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(
        json.contains("RESULT-A") && json.contains("RESULT-B"),
        "{json}"
    );
}

// Go TestStreamRepairsDanglingToolCalls at the request layer: an
// interrupted history with unanswered tool_calls is repaired before sending.
#[test]
fn repairs_dangling_tool_calls() {
    let c = client(ClientConfig::new(
        "deepseek-flash",
        "https://gateway.example/v1",
        "deepseek-v4",
    ));
    let req = ChatCompletionRequest {
        messages: vec![
            user("list the files"),
            Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_owned(),
                    name: "ls".to_owned(),
                    arguments: r#"{"path":"."}"#.to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            },
            user("never mind, what time is it?"),
        ],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    let messages = value["messages"].as_array().unwrap();
    let answered: Vec<&serde_json::Value> =
        messages.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(
        answered.len(),
        1,
        "the dangling tool_call must be paired with its placeholder: {json}"
    );
    assert_eq!(answered[0]["tool_call_id"], "call_1");
}

// Go TestNewDeepSeekThinkingDefaultsAndValidation +
// TestNewDeepSeekV4FlashForwardsLowEffort + effort-vocabulary errors.
#[test]
fn deepseek_effort_validation() {
    let c = client(ClientConfig::new(
        "deepseek",
        "https://api.deepseek.com",
        "deepseek-v4",
    ));
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"high""#), "{json}");

    let err = client_err(ClientConfig {
        effort: "medium".to_owned(),
        ..ClientConfig::new("deepseek", "https://api.deepseek.com", "deepseek-v4")
    });
    assert!(matches!(err, ProviderError::Config { .. }));

    let err = client_err(ClientConfig {
        effort: "low".to_owned(),
        ..ClientConfig::new("deepseek", "https://api.deepseek.com", "deepseek-v4-pro")
    });
    assert!(err.to_string().contains("requires deepseek-v4-flash"));

    let c = client(ClientConfig {
        effort: "low".to_owned(),
        reasoning_protocol: "deepseek".to_owned(),
        ..ClientConfig::new("deepseek", "https://api.deepseek.com", "deepseek-v4-flash")
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"low""#), "{json}");

    let c = client(ClientConfig {
        effort: "low".to_owned(),
        reasoning_protocol: "deepseek".to_owned(),
        supported_efforts: vec!["low".into(), "high".into(), "max".into()],
        ..ClientConfig::new(
            "custom-deepseek",
            "https://gateway.example.com/v1",
            "custom-flash",
        )
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"low""#), "{json}");
}

// Go TestNewMiniMaxEffortValidation + TestNewZhipuEffortValidation.
#[test]
fn vendor_effort_validation() {
    for ok in ["", "adaptive", "disabled"] {
        build_client(&ClientConfig {
            effort: ok.to_owned(),
            ..ClientConfig::new("m3", "https://api.minimaxi.com/v1", "MiniMax-M3")
        })
        .unwrap_or_else(|e| panic!("effort={ok:?} should be accepted: {e}"));
    }
    for bad in ["high", "low", "max", "turbo"] {
        assert!(
            build_client(&ClientConfig {
                effort: bad.to_owned(),
                ..ClientConfig::new("m3", "https://api.minimaxi.com/v1", "MiniMax-M3")
            })
            .is_err(),
            "effort={bad:?} should be rejected"
        );
    }
    for ok in ["", "enabled", "disabled"] {
        build_client(&ClientConfig {
            effort: ok.to_owned(),
            ..ClientConfig::new("glm", "https://open.bigmodel.cn/api/paas/v4", "glm-4.5-air")
        })
        .unwrap_or_else(|e| panic!("effort={ok:?} should be accepted: {e}"));
    }
    for bad in ["high", "low", "max", "adaptive"] {
        assert!(
            build_client(&ClientConfig {
                effort: bad.to_owned(),
                ..ClientConfig::new("glm", "https://open.bigmodel.cn/api/paas/v4", "glm-4.5-air")
            })
            .is_err(),
            "effort={bad:?} should be rejected"
        );
    }
}

// Go TestNewOllamaCloudReasoningEffort + TestNewThinkingConfigParsing.
#[test]
fn ollama_cloud_and_thinking_config() {
    let c = client(ClientConfig {
        effort: "max".to_owned(),
        ..ClientConfig::new(
            "ollama-cloud",
            "https://ollama.com/v1",
            "nemotron-3-nano:30b",
        )
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(json.contains(r#""reasoning_effort":"max""#), "{json}");
    let c = client(ClientConfig {
        effort: "none".to_owned(),
        ..ClientConfig::new(
            "ollama-cloud",
            "https://ollama.com/v1",
            "nemotron-3-nano:30b",
        )
    });
    let json = body(&c, &ChatCompletionRequest::default());
    assert!(!json.contains("reasoning_effort"), "{json}");
    assert!(
        build_client(&ClientConfig {
            effort: "ultra".to_owned(),
            ..ClientConfig::new(
                "ollama-cloud",
                "https://ollama.com/v1",
                "nemotron-3-nano:30b"
            )
        })
        .is_err(),
        "invalid Ollama effort must be rejected"
    );

    for (input, want) in [
        ("enabled", "enabled"),
        ("DISABLED", "disabled"),
        ("adaptive", ""),
        ("garbage", ""),
        ("", ""),
    ] {
        let c = client(ClientConfig {
            thinking: input.to_owned(),
            ..ClientConfig::new("gen", "https://api.example.com/v1", "x")
        });
        let json = body(&c, &ChatCompletionRequest::default());
        let got = if let Some(pos) = json.find(r#""thinking":{"type":""#) {
            let rest = &json[pos + r#""thinking":{"type":""#.len()..];
            rest.split('"').next().unwrap_or("").to_owned()
        } else {
            String::new()
        };
        assert_eq!(got, want, "thinking={input:?} routed wrong");
    }
}

// Go TestStreamSendsExtraBody: reserved fields cannot be overridden; other
// extras ride top-level and integral numbers keep Go's map shape.
#[test]
fn extra_body_merges_and_keeps_reserved_fields() {
    let mut cfg = ClientConfig::new("custom", "https://gateway.example/v1", "model-a");
    cfg.extra_body
        .insert("enable_thinking".into(), serde_json::json!(true));
    cfg.extra_body
        .insert("top_p".into(), serde_json::json!(0.7));
    cfg.extra_body
        .insert("model".into(), serde_json::json!("wrong"));
    cfg.extra_body
        .insert("stream".into(), serde_json::json!(false));
    let c = client(cfg);
    let json = body(&c, &ChatCompletionRequest::default());
    let value: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(value["enable_thinking"], true);
    assert_eq!(value["top_p"], 0.7);
    assert_eq!(
        value["model"], "model-a",
        "reserved field overridden: {json}"
    );
    assert_eq!(value["stream"], true, "reserved field overridden: {json}");
}

// Go TestBuildRequestScopesLegacyTupleMigrationToMiMo's non-MiMo half: a
// schema rides the wire byte-for-byte.
#[test]
fn non_mimo_schema_passes_through() {
    let c = client(ClientConfig::new("x", "https://gateway.example/v1", "m"));
    let legacy = r#"{"type":"object","properties":{"pair":{"type":"array","items":[{"type":"string"},{"type":"number"}]}}}"#;
    let req = ChatCompletionRequest {
        tools: vec![tool_schema("tuple", legacy)],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(json.contains(legacy), "schema changed: {json}");
}

// Go TestNewMiniMaxSetsFlag/TestNewZhipuSetsFlag equivalents at the wire
// level: base-URL detection picks the dialect flag.
#[test]
fn vendor_flag_detection_reaches_the_wire() {
    for base in ["https://api.minimaxi.com/v1", "https://api.minimaxi.com"] {
        let c = client(ClientConfig::new("m3", base, "MiniMax-M3"));
        let json = body(&c, &ChatCompletionRequest::default());
        assert!(
            json.contains(r#""thinking":{"type":"adaptive"}"#),
            "{base}: MiniMax dialect missing"
        );
    }
    for base in [
        "https://open.bigmodel.cn/api/paas/v4",
        "https://api.z.ai/api/paas/v4",
    ] {
        let c = client(ClientConfig::new("glm", base, "glm-4.5-air"));
        let json = body(&c, &ChatCompletionRequest::default());
        assert!(
            json.contains(r#""thinking":{"type":"enabled"}"#),
            "{base}: Zhipu dialect missing"
        );
    }
}

// Go TestBuildRequestAlwaysSerializesContent's vision arm: user images ride
// image_url parts.
#[test]
fn vision_user_turn_serializes_image_parts() {
    let mut cfg = ClientConfig::new("x", "https://gateway.example/v1", "vision-model");
    cfg.vision = true;
    let c = client(cfg);
    let req = ChatCompletionRequest {
        messages: vec![Message {
            role: Role::USER,
            content: "look".to_owned(),
            images: vec!["data:image/png;base64,AAA".to_owned()],
            ..Message::default()
        }],
        ..ChatCompletionRequest::default()
    };
    let json = body(&c, &req);
    assert!(
        json.contains(r#""image_url":{"url":"data:image/png;base64,AAA"}"#),
        "{json}"
    );
}
