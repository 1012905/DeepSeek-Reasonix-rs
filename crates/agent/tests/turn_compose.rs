//! Request composition at the provider boundary: what the transport
//! receives across two turns (Go: cache_body_test.go's shape — the prefix
//! cache keys on these bytes, so the wire is the oracle).

mod support;

use reasonix_contract::provider::Role;
use support::{
    chunked_sse, fixture, registry_with_readers, serve_capturing, text_frame, usage_frame,
};

fn messages_of(body: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(body).expect("request body is JSON");
    v.get("messages")
        .expect("messages array")
        .as_array()
        .expect("array")
        .iter()
        .map(|m| serde_json::to_string(m).expect("canonical message"))
        .collect()
}

#[tokio::test]
async fn the_next_turns_request_reuses_the_previous_ones_bytes() {
    let base_responses = |first: &str, second: &str| {
        vec![
            chunked_sse(&[&text_frame(first), &usage_frame(7, 3, "stop"), "[DONE]"]),
            chunked_sse(&[&text_frame(second), &usage_frame(7, 3, "stop"), "[DONE]"]),
        ]
    };
    let (base, bodies) = serve_capturing(base_responses("the answer", "second answer")).await;
    let f = fixture(&base, registry_with_readers()).await;

    f.agent.run("first prompt").await.expect("turn one");
    f.agent.run("second prompt").await.expect("turn two");

    let captured = bodies.lock().expect("capture lock").clone();
    assert_eq!(captured.len(), 2, "one request per turn");
    let one = messages_of(&captured[0]);
    let two = messages_of(&captured[1]);
    // The prefix property the cache keys on: every message of the earlier
    // request reappears byte-identical in the later one, and only appended
    // messages differ.
    assert!(
        two.len() == one.len() + 2 && two[..one.len()] == one[..],
        "turn two's messages must extend turn one's verbatim\none:   {one:?}\n\ntwo:   {two:?}"
    );
    // The appended tail is exactly the prior answer and the new prompt.
    assert!(two[one.len()].contains("the answer"));
    assert!(two[one.len() + 1].contains("second prompt"));
}

#[tokio::test]
async fn wall_clock_metadata_never_reaches_the_wire() {
    let (base, bodies) = serve_capturing(vec![chunked_sse(&[
        &text_frame("ok"),
        &usage_frame(5, 1, "stop"),
        "[DONE]",
    ])])
    .await;
    let f = fixture(&base, registry_with_readers()).await;
    f.agent.run("a prompt").await.expect("turn");
    for body in bodies.lock().expect("capture lock").iter() {
        assert!(
            !body.contains("createdAt") && !body.contains("created_at"),
            "wall-clock metadata must not reach the transport: {body}"
        );
    }
}

#[tokio::test]
async fn the_system_prompt_leads_every_request_and_stays_stable() {
    let (base, bodies) = serve_capturing(vec![
        chunked_sse(&[&text_frame("one"), &usage_frame(5, 1, "stop"), "[DONE]"]),
        chunked_sse(&[&text_frame("two"), &usage_frame(5, 1, "stop"), "[DONE]"]),
    ])
    .await;
    let f = fixture(&base, registry_with_readers()).await;
    f.agent.run("first").await.expect("turn one");
    f.agent.run("second").await.expect("turn two");
    let captured = bodies.lock().expect("capture lock").clone();
    for body in &captured {
        let v: serde_json::Value = serde_json::from_str(body).expect("JSON");
        let head = &v["messages"][0];
        assert_eq!(head["role"], Role::SYSTEM.as_str());
        assert_eq!(head["content"], "You are the test kernel.");
    }
}

#[tokio::test]
async fn the_tool_contract_is_identical_across_turns() {
    let (base, bodies) = serve_capturing(vec![
        chunked_sse(&[&text_frame("one"), &usage_frame(5, 1, "stop"), "[DONE]"]),
        chunked_sse(&[&text_frame("two"), &usage_frame(5, 1, "stop"), "[DONE]"]),
    ])
    .await;
    let f = fixture(&base, registry_with_readers()).await;
    f.agent.run("first").await.expect("turn one");
    f.agent.run("second").await.expect("turn two");
    let captured = bodies.lock().expect("capture lock").clone();
    let tools_of = |body: &str| -> String {
        let v: serde_json::Value = serde_json::from_str(body).expect("JSON");
        serde_json::to_string(&v["tools"]).expect("canonical tools")
    };
    assert!(!captured.is_empty());
    assert_eq!(
        tools_of(&captured[0]),
        tools_of(&captured[1]),
        "the tool contract is part of the cache-stable prefix"
    );
}
