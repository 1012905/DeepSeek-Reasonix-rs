//! Wire-model tests for the provider message model
//! (docs/contracts/provider-message.md, ledger PMSG-001..007).
//!
//! Goldens mirror the reference's Go `json.Marshal` output: compact,
//! declaration-ordered keys, HTML-escaped strings, `null` for nil Go
//! slices without `omitempty`, zero values omitted with `omitempty`.

use reasonix_contract::agentcomposition::{
    Binding, CompositionError, Definition, LIVE_BINDING_VERSION, LIVE_RESOURCES, Module, Resource,
    SCHEMA_VERSION, Selection, Version, unavailable,
};
use reasonix_contract::gojson::marshal;
use reasonix_contract::provider::{
    ARGUMENT_ENVELOPE_UNWRAPPED, ArgumentNormalization, DecisionReceipt, InterruptedToolSummary,
    InterruptedTurnRecovery, MemoryCitation, Message, Role, StreamFailureCause, StreamFailureError,
    ToolCall, ToolExecution, ToolFailure, UnavailableImage, Via, VideoRef, safe_video_name,
    video_failed_note, video_not_sent_note, video_shown_note,
};
use serde_json::value::RawValue;

const FULL_GOLDEN: &str = "{\"agent_composition\":{\"formatVersion\":2,\"resourceMode\":\"live\",\"resolved\":{\"schemaVersion\":0,\"id\":\"\",\"name\":\"\",\"description\":\"\",\"persona\":{\"prefix\":\"\",\"suffix\":\"\",\"include_runtime_context\":false},\"tools\":{\"mode\":\"\",\"members\":null},\"skills\":{\"mode\":\"\",\"members\":null},\"mcp\":{\"mode\":\"\",\"members\":null},\"extensions\":{\"mode\":\"\",\"members\":null},\"hooks\":{\"mode\":\"\",\"members\":null},\"commands\":{\"mode\":\"\",\"members\":null}},\"started\":true,\"version\":{\"definition\":{\"schemaVersion\":1,\"id\":\"builtin:x\",\"name\":\"X\",\"description\":\"\",\"persona\":{\"prefix\":\"\",\"suffix\":\"\",\"include_runtime_context\":false},\"tools\":{\"mode\":\"\",\"members\":null},\"skills\":{\"mode\":\"\",\"members\":null},\"mcp\":{\"mode\":\"\",\"members\":null},\"extensions\":{\"mode\":\"\",\"members\":null},\"hooks\":{\"mode\":\"\",\"members\":null},\"commands\":{\"mode\":\"\",\"members\":null}},\"revision\":\"r1\"},\"digest\":\"digest\",\"workspace\":\"ws\",\"resources\":[],\"systemPrompt\":\"sp\",\"toolSchemaDigest\":\"tsd\",\"toolSurface\":[\"bash\"]},\"role\":\"assistant\",\"content\":\"c\",\"raw_content\":\"rc\",\"provider_content\":\"pc\",\"images\":[\"data:image/png;base64,AAA\"],\"videos\":[{\"url\":\"https://v.example/v.mp4\",\"path\":\"C:\\\\v.mp4\",\"name\":\"v.mp4\",\"sha256\":\"abc\",\"mime\":\"video/mp4\",\"size\":10,\"duration_ms\":100}],\"unavailable_images\":[{\"index\":1,\"ref\":\"sha256:deadbeef\"}],\"reasoning_content\":\"think\",\"reasoning_id\":\"ri\",\"reasoning_status\":\"completed\",\"reasoning_signature\":\"sig\",\"tool_calls\":[{\"id\":\"call1\",\"name\":\"read\",\"arguments\":\"{}\",\"argument_normalization\":{\"reason\":\"redundant_arguments_envelope\",\"original\":\"{}\",\"effective\":\"{}\"},\"thought_signature\":\"ts\",\"diff\":\"d\",\"added\":1,\"removed\":2,\"resolved_name\":\"rn\",\"capability_id\":\"cap\",\"resolved_read_only\":true}],\"responses_items\":[{\"type\":\"web_search_call\"}],\"tool_call_id\":\"call1\",\"name\":\"read\",\"memoryCitations\":[{\"id\":\"m1\",\"source\":\"memory.md\",\"lineStart\":1,\"lineEnd\":2,\"note\":\"n\",\"kind\":\"k\"}],\"workDurationMs\":12,\"thoughtMs\":34,\"createdAt\":1700000000000,\"edited\":true,\"original\":\"o\",\"host_authored\":true,\"modelRef\":\"model-x\",\"via\":{\"device\":\"d1\",\"ordinal\":2},\"local_only\":true,\"decision_receipt\":{\"id\":\"r1\",\"kind\":\"tool\",\"tool\":\"bash\",\"subject\":\"subj\",\"outcome\":\"allow\",\"via\":{\"device\":\"d1\",\"ordinal\":1}},\"decision_receipts\":[null,{\"id\":\"r2\",\"kind\":\"tool\",\"outcome\":\"deny\"}],\"interrupted_turn\":{\"stream_failure\":\"responses.invalid_function_call\",\"pending\":true,\"completed_tools\":[{\"id\":\"c1\",\"name\":\"bash\",\"files\":[\"a.txt\"],\"added\":1}],\"interrupted_tools\":[\"write\"],\"dropped_partial_text\":true},\"tool_execution\":{\"kind\":\"shell\",\"shell\":\"bash\",\"shellVersion\":\"5\",\"platform\":\"windows\",\"supportsAndAnd\":true,\"state\":\"done\",\"exitCode\":0,\"outputTail\":\"tail\",\"mutationRisk\":\"low\",\"verification\":\"none\",\"durationMs\":5},\"tool_failure\":{\"refusalCode\":\"X\",\"blocked\":true}}";
const BINDING_GOLDEN: &str = "{\"formatVersion\":2,\"resourceMode\":\"live\",\"resolved\":{\"schemaVersion\":0,\"id\":\"\",\"name\":\"\",\"description\":\"\",\"persona\":{\"prefix\":\"\",\"suffix\":\"\",\"include_runtime_context\":false},\"tools\":{\"mode\":\"\",\"members\":null},\"skills\":{\"mode\":\"\",\"members\":null},\"mcp\":{\"mode\":\"\",\"members\":null},\"extensions\":{\"mode\":\"\",\"members\":null},\"hooks\":{\"mode\":\"\",\"members\":null},\"commands\":{\"mode\":\"\",\"members\":null}},\"started\":true,\"version\":{\"definition\":{\"schemaVersion\":1,\"id\":\"builtin:x\",\"name\":\"X\",\"description\":\"\",\"persona\":{\"prefix\":\"\",\"suffix\":\"\",\"include_runtime_context\":false},\"tools\":{\"mode\":\"\",\"members\":null},\"skills\":{\"mode\":\"\",\"members\":null},\"mcp\":{\"mode\":\"\",\"members\":null},\"extensions\":{\"mode\":\"\",\"members\":null},\"hooks\":{\"mode\":\"\",\"members\":null},\"commands\":{\"mode\":\"\",\"members\":null}},\"revision\":\"r1\"},\"digest\":\"digest\",\"workspace\":\"ws\",\"resources\":[],\"systemPrompt\":\"sp\",\"toolSchemaDigest\":\"tsd\",\"toolSurface\":[\"bash\"]}";
const DEF_GOLDEN: &str = "{\"schemaVersion\":0,\"id\":\"\",\"name\":\"\",\"description\":\"\",\"persona\":{\"prefix\":\"\",\"suffix\":\"\",\"include_runtime_context\":false},\"tools\":{\"mode\":\"\",\"members\":null},\"skills\":{\"mode\":\"\",\"members\":null},\"mcp\":{\"mode\":\"\",\"members\":null},\"extensions\":{\"mode\":\"\",\"members\":null},\"hooks\":{\"mode\":\"\",\"members\":null},\"commands\":{\"mode\":\"\",\"members\":null}}";

fn full_binding() -> Binding {
    Binding {
        format_version: LIVE_BINDING_VERSION,
        resource_mode: LIVE_RESOURCES.to_owned(),
        resolved: Definition::default(),
        started: true,
        version: Version {
            definition: Definition {
                schema_version: SCHEMA_VERSION,
                id: "builtin:x".to_owned(),
                name: "X".to_owned(),
                ..Definition::default()
            },
            revision: "r1".to_owned(),
        },
        digest: "digest".to_owned(),
        workspace: "ws".to_owned(),
        resources: Some(Vec::new()),
        system_prompt: "sp".to_owned(),
        tool_schema_digest: "tsd".to_owned(),
        tool_surface: Some(vec!["bash".to_owned()]),
    }
}

fn full_message() -> Message {
    Message {
        agent_composition: Some(full_binding()),
        role: Role::ASSISTANT,
        content: "c".to_owned(),
        raw_content: "rc".to_owned(),
        provider_content: "pc".to_owned(),
        images: vec!["data:image/png;base64,AAA".to_owned()],
        videos: vec![VideoRef {
            url: "https://v.example/v.mp4".to_owned(),
            path: "C:\\v.mp4".to_owned(),
            name: "v.mp4".to_owned(),
            sha256: "abc".to_owned(),
            mime: "video/mp4".to_owned(),
            size: 10,
            duration_ms: 100,
        }],
        unavailable_images: vec![UnavailableImage {
            index: 1,
            reference: "sha256:deadbeef".to_owned(),
        }],
        reasoning_content: "think".to_owned(),
        reasoning_id: "ri".to_owned(),
        reasoning_status: "completed".to_owned(),
        reasoning_signature: "sig".to_owned(),
        tool_calls: vec![ToolCall {
            id: "call1".to_owned(),
            name: "read".to_owned(),
            arguments: "{}".to_owned(),
            argument_normalization: Some(ArgumentNormalization {
                reason: ARGUMENT_ENVELOPE_UNWRAPPED.to_owned(),
                original: "{}".to_owned(),
                effective: "{}".to_owned(),
            }),
            thought_signature: "ts".to_owned(),
            diff: "d".to_owned(),
            added: 1,
            removed: 2,
            resolved_name: "rn".to_owned(),
            capability_id: "cap".to_owned(),
            resolved_read_only: Some(true),
        }],
        responses_items: vec![
            RawValue::from_string(r#"{"type":"web_search_call"}"#.to_owned()).expect("valid JSON"),
        ],
        tool_call_id: "call1".to_owned(),
        name: "read".to_owned(),
        memory_citations: vec![MemoryCitation {
            id: "m1".to_owned(),
            source: "memory.md".to_owned(),
            line_start: 1,
            line_end: 2,
            note: "n".to_owned(),
            kind: "k".to_owned(),
        }],
        work_duration_ms: 12,
        thought_ms: 34,
        created_at: 1_700_000_000_000,
        edited: true,
        original: "o".to_owned(),
        host_authored: true,
        model_ref: "model-x".to_owned(),
        via: Some(Via {
            device: "d1".to_owned(),
            ordinal: 2,
        }),
        local_only: true,
        decision_receipt: Some(DecisionReceipt {
            id: "r1".to_owned(),
            kind: "tool".to_owned(),
            tool: "bash".to_owned(),
            subject: "subj".to_owned(),
            outcome: "allow".to_owned(),
            via: Some(Via {
                device: "d1".to_owned(),
                ordinal: 1,
            }),
        }),
        decision_receipts: vec![
            None,
            Some(DecisionReceipt {
                id: "r2".to_owned(),
                kind: "tool".to_owned(),
                outcome: "deny".to_owned(),
                ..DecisionReceipt::default()
            }),
        ],
        interrupted_turn: Some(InterruptedTurnRecovery {
            stream_failure: StreamFailureCause::INVALID_FUNCTION_CALL,
            pending: true,
            completed_tools: vec![InterruptedToolSummary {
                id: "c1".to_owned(),
                name: "bash".to_owned(),
                files: vec!["a.txt".to_owned()],
                added: 1,
                ..InterruptedToolSummary::default()
            }],
            interrupted_tools: vec!["write".to_owned()],
            dropped_partial_text: true,
            ..InterruptedTurnRecovery::default()
        }),
        tool_execution: Some(ToolExecution {
            kind: "shell".to_owned(),
            shell: "bash".to_owned(),
            shell_version: "5".to_owned(),
            platform: "windows".to_owned(),
            supports_and_and: true,
            state: "done".to_owned(),
            exit_code: Some(0),
            output_tail: "tail".to_owned(),
            mutation_risk: "low".to_owned(),
            verification: "none".to_owned(),
            duration_ms: 5,
            ..ToolExecution::default()
        }),
        tool_failure: Some(ToolFailure {
            refusal_code: "X".to_owned(),
            blocked: true,
        }),
    }
}

#[test]
fn minimal_user_serializes_role_only() {
    let m = Message {
        role: Role::USER,
        ..Default::default()
    };
    assert_eq!(marshal(&m).unwrap(), r#"{"role":"user"}"#);
}

#[test]
fn default_message_serializes_empty_role() {
    let m = Message::default();
    assert_eq!(marshal(&m).unwrap(), r#"{"role":""}"#);
}

#[test]
fn message_full_field_order() {
    let m = full_message();
    assert_eq!(marshal(&m).unwrap(), FULL_GOLDEN);
}

#[test]
fn message_omitempty_drops_zero_scalars_and_empty_slices() {
    let m = Message {
        role: Role::USER,
        work_duration_ms: 12,
        ..Default::default()
    };
    assert_eq!(
        marshal(&m).unwrap(),
        r#"{"role":"user","workDurationMs":12}"#
    );
    let m = Message {
        role: Role::USER,
        images: Vec::new(),
        tool_calls: Vec::new(),
        responses_items: Vec::new(),
        decision_receipts: Vec::new(),
        ..Default::default()
    };
    assert_eq!(marshal(&m).unwrap(), r#"{"role":"user"}"#);
    let m = Message {
        role: Role::USER,
        thought_ms: 2,
        created_at: 1,
        edited: true,
        host_authored: true,
        local_only: true,
        ..Default::default()
    };
    assert_eq!(
        marshal(&m).unwrap(),
        r#"{"role":"user","thoughtMs":2,"createdAt":1,"edited":true,"host_authored":true,"local_only":true}"#
    );
}

#[test]
fn decode_ignores_unknown_fields_and_missing_keys() {
    let m: Message = serde_json::from_str(r#"{"role":"user","totally_unknown":"x"}"#).unwrap();
    assert_eq!(m.role, Role::USER);
    assert_eq!(m.content, "");
    let m: Message = serde_json::from_str("{}").unwrap();
    assert!(m.role.is_empty());
    assert!(!m.edited);
    assert!(m.images.is_empty());
}

#[test]
fn decode_null_tolerates_every_field_class() {
    let m: Message = serde_json::from_str(
        r#"{"role":null,"content":null,"tool_calls":null,"responses_items":null,"decision_receipts":null,"images":null,"workDurationMs":null,"edited":null,"via":null,"decision_receipt":null,"agent_composition":null,"interrupted_turn":null,"tool_execution":null,"tool_failure":null}"#,
    )
    .unwrap();
    assert!(m.role.is_empty());
    assert_eq!(m.content, "");
    assert!(m.images.is_empty());
    assert!(m.tool_calls.is_empty());
    assert!(m.responses_items.is_empty());
    assert!(m.decision_receipts.is_empty());
    assert_eq!(m.work_duration_ms, 0);
    assert!(!m.edited);
    assert!(m.via.is_none());
    assert!(m.decision_receipt.is_none());
    assert!(m.agent_composition.is_none());
    assert!(m.interrupted_turn.is_none());
    assert!(m.tool_execution.is_none());
    assert!(m.tool_failure.is_none());
    assert_eq!(marshal(&m).unwrap(), r#"{"role":""}"#);
}

#[test]
fn decode_type_mismatch_fails() {
    for bad in [
        r#"{"role":5}"#,
        r#"{"content":[]}"#,
        r#"{"edited":"yes"}"#,
        r#"{"workDurationMs":"x"}"#,
        r#"{"images":{}}"#,
        r#"{"via":[]}"#,
        r#"{"agent_composition":[]}"#,
        // Go rejects arrays for struct values everywhere; a positional
        // serde seq decode would silently produce a zero struct.
        r#"{"decision_receipt":[]}"#,
        r#"{"interrupted_turn":[]}"#,
        r#"{"tool_execution":[]}"#,
        r#"{"tool_failure":[]}"#,
        r#"{"memoryCitations":[[]]}"#,
        r#"{"tool_calls":[[]]}"#,
        r#"{"videos":[[]]}"#,
        r#"{"unavailable_images":[[]]}"#,
        r#"{"decision_receipts":[[]]}"#,
        r#"{"agent_composition":{"resolved":[]}}"#,
        r#"{"agent_composition":{"version":{"definition":[]}}}"#,
        r#"{"agent_composition":{"resources":[[]]}}"#,
        r#"{"agent_composition":{"toolSurface":{}}}"#,
    ] {
        assert!(
            serde_json::from_str::<Message>(bad).is_err(),
            "expected decode failure: {bad}"
        );
    }
    for bad in ["[]", "5", "\"x\"", "true"] {
        assert!(
            serde_json::from_str::<Message>(bad).is_err(),
            "expected decode failure: {bad}"
        );
    }
}

#[test]
fn decode_null_struct_fields_become_defaults() {
    let m: Message = serde_json::from_str(
        r#"{"agent_composition":{"resolved":{"persona":null,"tools":null},"version":null},"interrupted_turn":{"completed_tools":[null]}}"#,
    )
    .unwrap();
    let binding = m.agent_composition.as_ref().unwrap();
    let persona = &binding.resolved.persona;
    assert_eq!(persona.prefix, "");
    assert_eq!(persona.suffix, "");
    assert!(!persona.include_runtime_context);
    assert_eq!(binding.resolved.tools.mode, "");
    assert!(binding.resolved.tools.members.is_none());
    assert_eq!(binding.version.definition.id, "");
    assert_eq!(binding.version.revision, "");
    let completed = &m.interrupted_turn.as_ref().unwrap().completed_tools;
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, "");
    assert_eq!(completed[0].name, "");
    assert_eq!(completed[0].files, Vec::<String>::new());
    assert_eq!(completed[0].added, 0);
    assert_eq!(completed[0].removed, 0);
}

#[test]
fn decode_null_slice_elements_become_zero_values() {
    let m: Message = serde_json::from_str(
        r#"{"role":"user","images":[null,"x"],"videos":[null],"unavailable_images":[null],"tool_calls":[null],"memoryCitations":[null],"decision_receipts":[null]}"#,
    )
    .unwrap();
    assert_eq!(m.images, vec!["".to_owned(), "x".to_owned()]);
    assert_eq!(m.videos, vec![VideoRef::default()]);
    assert_eq!(m.unavailable_images, vec![UnavailableImage::default()]);
    assert_eq!(m.tool_calls, vec![ToolCall::default()]);
    assert_eq!(m.memory_citations, vec![MemoryCitation::default()]);
    assert_eq!(m.decision_receipts, vec![None]);
}

#[test]
fn tool_call_shape_and_execution_arguments() {
    let tc = ToolCall {
        id: "c1".to_owned(),
        name: "bash".to_owned(),
        arguments: "{}".to_owned(),
        argument_normalization: Some(ArgumentNormalization {
            reason: ARGUMENT_ENVELOPE_UNWRAPPED.to_owned(),
            original: "{}".to_owned(),
            effective: r#"{"cmd":"ls"}"#.to_owned(),
        }),
        ..ToolCall::default()
    };
    assert_eq!(tc.execution_arguments(), r#"{"cmd":"ls"}"#);
    assert_eq!(
        marshal(&tc).unwrap(),
        "{\"id\":\"c1\",\"name\":\"bash\",\"arguments\":\"{}\",\"argument_normalization\":{\"reason\":\"redundant_arguments_envelope\",\"original\":\"{}\",\"effective\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}"
    );
    let mismatched = ToolCall {
        arguments: "{}".to_owned(),
        argument_normalization: Some(ArgumentNormalization {
            reason: ARGUMENT_ENVELOPE_UNWRAPPED.to_owned(),
            original: "DIFFERENT".to_owned(),
            effective: r#"{"cmd":"ls"}"#.to_owned(),
        }),
        ..ToolCall::default()
    };
    assert_eq!(mismatched.execution_arguments(), "{}");
    let other_reason = ToolCall {
        arguments: "{}".to_owned(),
        argument_normalization: Some(ArgumentNormalization {
            reason: "something_else".to_owned(),
            original: "{}".to_owned(),
            effective: r#"{"cmd":"ls"}"#.to_owned(),
        }),
        ..ToolCall::default()
    };
    assert_eq!(other_reason.execution_arguments(), "{}");
    let plain = ToolCall {
        arguments: "{}".to_owned(),
        ..ToolCall::default()
    };
    assert_eq!(plain.execution_arguments(), "{}");
    assert_eq!(
        marshal(&ToolCall::default()).unwrap(),
        r#"{"id":"","name":"","arguments":""}"#
    );
}

#[test]
fn local_metadata_wire_shapes() {
    assert_eq!(
        marshal(&ToolExecution::default()).unwrap(),
        r#"{"supportsAndAnd":false}"#
    );
    let te = ToolExecution {
        kind: "shell".to_owned(),
        shell: "bash".to_owned(),
        shell_version: "5".to_owned(),
        platform: "windows".to_owned(),
        supports_and_and: true,
        state: "done".to_owned(),
        exit_code: Some(0),
        output_tail: "tail".to_owned(),
        mutation_risk: "low".to_owned(),
        verification: "none".to_owned(),
        duration_ms: 5,
        ..ToolExecution::default()
    };
    assert_eq!(
        marshal(&te).unwrap(),
        r#"{"kind":"shell","shell":"bash","shellVersion":"5","platform":"windows","supportsAndAnd":true,"state":"done","exitCode":0,"outputTail":"tail","mutationRisk":"low","verification":"none","durationMs":5}"#
    );
    let te = ToolExecution {
        kind: "shell".to_owned(),
        exit_code: None,
        ..ToolExecution::default()
    };
    assert_eq!(
        marshal(&te).unwrap(),
        r#"{"kind":"shell","supportsAndAnd":false}"#
    );
    assert_eq!(marshal(&ToolFailure::default()).unwrap(), r#"{}"#);
    assert_eq!(
        marshal(&ToolFailure {
            blocked: true,
            ..ToolFailure::default()
        })
        .unwrap(),
        r#"{"blocked":true}"#
    );
    assert_eq!(
        marshal(&UnavailableImage {
            index: 1,
            reference: "sha256:deadbeef".to_owned(),
        })
        .unwrap(),
        r#"{"index":1,"ref":"sha256:deadbeef"}"#
    );
    assert_eq!(
        marshal(&UnavailableImage::default()).unwrap(),
        r#"{"index":0,"ref":""}"#
    );
    assert_eq!(
        marshal(&MemoryCitation::default()).unwrap(),
        r#"{"source":""}"#
    );
    assert_eq!(
        marshal(&Via::default()).unwrap(),
        r#"{"device":"","ordinal":0}"#
    );
    assert_eq!(
        marshal(&DecisionReceipt::default()).unwrap(),
        r#"{"id":"","kind":"","outcome":""}"#
    );
    let dr = DecisionReceipt {
        id: "r1".to_owned(),
        kind: "tool".to_owned(),
        tool: "bash".to_owned(),
        subject: "subj".to_owned(),
        outcome: "allow".to_owned(),
        via: Some(Via {
            device: "d1".to_owned(),
            ordinal: 1,
        }),
    };
    assert_eq!(
        marshal(&dr).unwrap(),
        r#"{"id":"r1","kind":"tool","tool":"bash","subject":"subj","outcome":"allow","via":{"device":"d1","ordinal":1}}"#
    );
    assert_eq!(
        marshal(&InterruptedToolSummary::default()).unwrap(),
        r#"{"name":""}"#
    );
    assert_eq!(
        marshal(&InterruptedTurnRecovery::default()).unwrap(),
        r#"{}"#
    );
    let itr = InterruptedTurnRecovery {
        stream_failure: StreamFailureCause::INVALID_FUNCTION_CALL,
        pending: true,
        completed_tools: vec![InterruptedToolSummary {
            id: "c1".to_owned(),
            name: "bash".to_owned(),
            files: vec!["a.txt".to_owned()],
            added: 1,
            ..InterruptedToolSummary::default()
        }],
        interrupted_tools: vec!["write".to_owned()],
        dropped_partial_text: true,
        ..InterruptedTurnRecovery::default()
    };
    assert_eq!(
        marshal(&itr).unwrap(),
        r#"{"stream_failure":"responses.invalid_function_call","pending":true,"completed_tools":[{"id":"c1","name":"bash","files":["a.txt"],"added":1}],"interrupted_tools":["write"],"dropped_partial_text":true}"#
    );
}

#[test]
fn stream_failure_cause_and_sentinels() {
    assert_eq!(
        StreamFailureCause::INVALID_FUNCTION_CALL.as_str(),
        "responses.invalid_function_call"
    );
    assert_eq!(
        StreamFailureCause::UNFINISHED_FUNCTION_CALL.as_str(),
        "responses.unfinished_function_call"
    );
    assert_eq!(
        StreamFailureCause::INVALID_FUNCTION_CALL.description(),
        "The Responses stream contained a function call with an invalid wire shape. The host refused the whole response; none of its tool calls ran or entered model history."
    );
    assert_eq!(
        StreamFailureCause::UNFINISHED_FUNCTION_CALL.description(),
        "The Responses stream ended successfully while a function call was still unclosed. The host refused the whole response; none of its tool calls ran or entered model history."
    );
    assert_eq!(StreamFailureCause::new("other").description(), "");
    assert!(StreamFailureCause::default().is_empty());
    assert_eq!(
        StreamFailureError::InvalidFunctionCall.to_string(),
        "responses: stream contains an invalid function call"
    );
    assert_eq!(
        StreamFailureError::UnfinishedFunctionCall.to_string(),
        "responses: stream ended with an unfinished function call"
    );
    assert_eq!(
        StreamFailureError::InvalidFunctionCall.cause(),
        StreamFailureCause::INVALID_FUNCTION_CALL
    );
    assert_eq!(
        StreamFailureError::UnfinishedFunctionCall.cause(),
        StreamFailureCause::UNFINISHED_FUNCTION_CALL
    );
    let itr = InterruptedTurnRecovery {
        stream_failure: StreamFailureCause::UNFINISHED_FUNCTION_CALL,
        ..InterruptedTurnRecovery::default()
    };
    assert_eq!(
        marshal(&itr).unwrap(),
        r#"{"stream_failure":"responses.unfinished_function_call"}"#
    );
}

#[test]
fn video_ref_shape_notes_and_safe_name() {
    let vr = VideoRef {
        url: "https://v.example/v.mp4".to_owned(),
        path: "C:\\v.mp4".to_owned(),
        name: "v.mp4".to_owned(),
        sha256: "abc".to_owned(),
        mime: "video/mp4".to_owned(),
        size: 10,
        duration_ms: 100,
    };
    assert_eq!(
        marshal(&vr).unwrap(),
        r#"{"url":"https://v.example/v.mp4","path":"C:\\v.mp4","name":"v.mp4","sha256":"abc","mime":"video/mp4","size":10,"duration_ms":100}"#
    );
    assert_eq!(
        marshal(&VideoRef::default()).unwrap(),
        r#"{"path":"","name":"","sha256":"","mime":"","size":0}"#
    );
    assert_eq!(
        video_shown_note("v.mp4"),
        "\n[video v.mp4 was shown to you when it was read and is not attached again; call read_video to look at it again]"
    );
    assert_eq!(
        video_not_sent_note("v.mp4"),
        "\n[video v.mp4 was not sent: this model cannot take video natively]"
    );
    assert_eq!(
        video_failed_note("v.mp4", "timeout"),
        "\n[video v.mp4 was not sent (timeout)]"
    );
    assert_eq!(safe_video_name("a\nb]c\rd"), "a b)c d");
}

#[test]
fn responses_items_keep_raw_bytes() {
    let m: Message = serde_json::from_str(
        r#"{"role":"user","responses_items":[{"type":"web_search_call"},null,123]}"#,
    )
    .unwrap();
    assert_eq!(m.responses_items.len(), 3);
    assert_eq!(m.responses_items[0].get(), r#"{"type":"web_search_call"}"#);
    assert_eq!(m.responses_items[1].get(), "null");
    assert_eq!(m.responses_items[2].get(), "123");
    assert_eq!(
        marshal(&m).unwrap(),
        r#"{"role":"user","responses_items":[{"type":"web_search_call"},null,123]}"#
    );
}

#[test]
fn decision_receipts_preserve_null_elements() {
    let m: Message = serde_json::from_str(
        r#"{"role":"user","decision_receipts":[null,{"id":"r2","kind":"tool","outcome":"deny"}]}"#,
    )
    .unwrap();
    assert_eq!(m.decision_receipts.len(), 2);
    assert!(m.decision_receipts[0].is_none());
    assert_eq!(m.decision_receipts[1].as_ref().unwrap().id, "r2");
    assert_eq!(
        marshal(&m).unwrap(),
        r#"{"role":"user","decision_receipts":[null,{"id":"r2","kind":"tool","outcome":"deny"}]}"#
    );
}

#[test]
fn role_const_partial_eq_and_roundtrip() {
    assert_eq!(Role::SYSTEM.as_str(), "system");
    assert_eq!(Role::USER.as_str(), "user");
    assert_eq!(Role::ASSISTANT.as_str(), "assistant");
    assert_eq!(Role::TOOL.as_str(), "tool");
    assert!(Role::SYSTEM == "system");
    assert!(Role::USER == *"user");
    assert_eq!(Role::from("tool"), Role::TOOL);
    assert_eq!(Role::from(String::from("assistant")), Role::ASSISTANT);
    assert!(Role::default().is_empty());
    assert_eq!(Role::new("custom-role").to_string(), "custom-role");
    let m: Message = serde_json::from_str(r#"{"role":"custom-role"}"#).unwrap();
    assert_eq!(m.role, Role::new("custom-role"));
    assert_eq!(marshal(&m).unwrap(), r#"{"role":"custom-role"}"#);
}

#[test]
fn composition_wire_shapes() {
    assert_eq!(marshal(&Definition::default()).unwrap(), DEF_GOLDEN);
    let d: Definition = serde_json::from_str(DEF_GOLDEN).unwrap();
    assert_eq!(marshal(&d).unwrap(), DEF_GOLDEN);

    let selection = Selection {
        mode: "select".to_owned(),
        members: Some(Vec::new()),
    };
    assert_eq!(
        marshal(&selection).unwrap(),
        r#"{"mode":"select","members":[]}"#
    );
    let selection = Selection {
        mode: "select".to_owned(),
        members: None,
    };
    assert_eq!(
        marshal(&selection).unwrap(),
        r#"{"mode":"select","members":null}"#
    );

    assert_eq!(marshal(&full_binding()).unwrap(), BINDING_GOLDEN);
    let b: Binding = serde_json::from_str(BINDING_GOLDEN).unwrap();
    assert_eq!(marshal(&b).unwrap(), BINDING_GOLDEN);
}

#[test]
fn module_parameters_null_vs_absent() {
    let m: Module =
        serde_json::from_str(r#"{"id":"m","parameters":null,"optional":true}"#).unwrap();
    assert_eq!(m.parameters.as_ref().unwrap().get(), "null");
    assert!(m.optional);
    assert_eq!(
        marshal(&m).unwrap(),
        r#"{"id":"m","parameters":null,"optional":true}"#
    );
    let m: Module = serde_json::from_str(r#"{"id":"m"}"#).unwrap();
    assert!(m.parameters.is_none());
    assert_eq!(marshal(&m).unwrap(), r#"{"id":"m"}"#);
    let m: Module = serde_json::from_str(r#"{"id":"m","parameters":{"a":1}}"#).unwrap();
    assert_eq!(m.parameters.as_ref().unwrap().get(), r#"{"a":1}"#);
    assert_eq!(marshal(&m).unwrap(), r#"{"id":"m","parameters":{"a":1}}"#);
}

#[test]
fn resource_data_defaults_to_null() {
    let r: Resource = serde_json::from_str(r#"{"id":"i","kind":"k"}"#).unwrap();
    assert!(r.data.is_none());
    assert_eq!(marshal(&r).unwrap(), r#"{"id":"i","kind":"k","data":null}"#);
    let r: Resource = serde_json::from_str(r#"{"id":"i","kind":"k","data":{"a":1}}"#).unwrap();
    assert_eq!(r.data.as_deref().unwrap().get(), r#"{"a":1}"#);
    assert_eq!(
        marshal(&r).unwrap(),
        r#"{"id":"i","kind":"k","data":{"a":1}}"#
    );
    let r = Resource {
        id: "i".to_owned(),
        kind: "k".to_owned(),
        ..Resource::default()
    };
    assert_eq!(marshal(&r).unwrap(), r#"{"id":"i","kind":"k","data":null}"#);
}

#[test]
fn binding_summary_projection() {
    let s = full_binding().summary();
    assert_eq!(s.id, "builtin:x");
    assert_eq!(s.name, "X");
    assert_eq!(s.revision, "r1");
    assert_eq!(s.digest, "digest");
    assert!(s.started);
    assert_eq!(s.format_version, 2);
    assert_eq!(s.resource_mode, "live");
    assert_eq!(
        marshal(&s).unwrap(),
        r#"{"id":"builtin:x","name":"X","revision":"r1","digest":"digest","started":true,"formatVersion":2,"resourceMode":"live"}"#
    );
}

#[test]
fn diagnostic_error_display_and_source() {
    use std::error::Error as _;

    let d = unavailable("mod", "why");
    assert_eq!(d.to_string(), "agent composition unavailable; mod: why");
    assert_eq!(d.diagnostics.len(), 1);
    assert_eq!(d.diagnostics[0].code, "agent_composition.unavailable");
    assert_eq!(d.diagnostics[0].module, "mod");
    assert_eq!(d.diagnostics[0].detail, "why");
    let src = d.source().expect("diagnostic error has a source");
    assert_eq!(src.to_string(), "agent composition unavailable");
    assert_eq!(
        CompositionError::invalid().to_string(),
        "invalid agent composition"
    );
    assert_eq!(
        CompositionError::Unavailable.to_string(),
        "agent composition unavailable"
    );
    assert_eq!(
        CompositionError::Locked.to_string(),
        "agent composition locked"
    );
    assert_eq!(
        CompositionError::WorkspaceBound.to_string(),
        "agent composition is bound to its workspace"
    );
}

#[test]
fn marshal_html_escapes_go_style() {
    let m = Message {
        role: Role::new(""),
        content: "a<b>&c\u{2028}d\u{2029}e".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        marshal(&m).unwrap(),
        "{\"role\":\"\",\"content\":\"a\\u003cb\\u003e\\u0026c\\u2028d\\u2029e\"}"
    );
}
