//! Tool naming/normalization oracles (MCP-001, MCP-006).
//!
//! Ported from `internal/ext/plugin/canonicalize_test.go`,
//! `toolname_test.go`, `protocol_inbound_test.go` (roots), and the Go
//! `normalizeName` / `shortNameHash` / `boundedToolName` implementations.

use reasonix_ext::mcp::{
    MAX_MODEL_TOOL_NAME, NamingError, bounded_tool_name, mcp_connect_permission_name, mcp_roots,
    mcp_roots_from_absolute, model_tool_name, normalize_name, short_name_hash, tool_name,
    tool_prefix, validate_mcp_tool_names,
};

#[test]
fn normalize_name_leaves_valid_identifiers_alone() {
    assert_eq!(normalize_name("valid_name-1"), "valid_name-1");
    assert_eq!(normalize_name("search_code"), "search_code");
    assert_eq!(normalize_name("foo"), "foo");
    assert_eq!(normalize_name("ABC_123-x"), "ABC_123-x");
}

#[test]
fn normalize_name_byte_exact_oracle_table() {
    // Values computed by running the Go implementation (see migration notes).
    let cases = [
        (
            "@modelcontextprotocol/server-memory",
            "modelcontextprotocol_server-memory_1691ab",
        ),
        ("mcp server/fetch", "mcp_server_fetch_b23fe8"),
        ("   ", "unnamed_873846"),
        ("search/code", "search_code_2fc740"),
        ("@foo", "foo_b785ac"),
        ("长", "unnamed_5b2e5e"),
    ];
    for (raw, want) in cases {
        assert_eq!(normalize_name(raw), want, "normalize_name({raw:?})");
    }
}

#[test]
fn fnv32a_short_hash_byte_exact() {
    let cases = [
        ("valid_name-1", "54a40d"),
        ("@modelcontextprotocol/server-memory", "1691ab"),
        ("mcp server/fetch", "b23fe8"),
        ("   ", "873846"),
        ("search/code", "2fc740"),
        ("@foo", "b785ac"),
        ("foo", "a9f37e"),
    ];
    for (raw, want) in cases {
        assert_eq!(short_name_hash(raw), want, "short_name_hash({raw:?})");
    }
}

#[test]
fn normalization_avoids_sanitized_collisions() {
    // Go TestNormalizeNameAvoidsSanitizedCollisions.
    let a = normalize_name("search/code");
    let b = normalize_name("search_code");
    assert_ne!(a, b);
    assert_eq!(b, "search_code");
    assert_ne!(normalize_name("@foo"), normalize_name("foo"));
}

#[test]
fn normalized_names_are_safe_identifiers() {
    // Go TestNormalizeNameForToolNames.
    for input in [
        "@modelcontextprotocol/server-memory",
        "mcp server/fetch",
        "   ",
        "a.b/c",
        "🚀",
    ] {
        let got = normalize_name(input);
        assert!(!got.is_empty(), "{input:?}");
        assert!(
            got.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{input:?} -> {got:?}"
        );
    }
}

#[test]
fn tool_prefix_and_connect_permission_name_byte_exact() {
    assert_eq!(tool_prefix("srv"), "mcp__srv__");
    assert_eq!(tool_prefix("mock"), "mcp__mock__");
    assert_eq!(mcp_connect_permission_name("mock"), "mcp_connect__mock");
    assert!(!mcp_connect_permission_name("mock").starts_with("mcp__"));
}

#[test]
fn model_tool_name_byte_exact_oracle_table() {
    assert_eq!(model_tool_name("srv", "echo"), "mcp__srv__echo");
    assert_eq!(
        model_tool_name("mock", "search/code"),
        "mcp__mock__search_code_2fc740"
    );
    assert_eq!(
        tool_name("srv", "@modelcontextprotocol/server-memory"),
        "mcp__srv__modelcontextprotocol_server-memory_1691ab"
    );
}

#[test]
fn model_tool_name_stays_within_the_provider_limit() {
    // Go TestModelToolNameStaysWithinTheProviderLimit and friends.
    let prefix = tool_prefix("srv"); // "mcp__srv__" = 10 bytes
    let fitting = "a".repeat(MAX_MODEL_TOOL_NAME - prefix.len());
    let one_over = "a".repeat(MAX_MODEL_TOOL_NAME - prefix.len() + 1);
    for raw in [
        "a".repeat(150),
        fitting.clone(),
        one_over,
        "长".repeat(80),
        format!("weird name/{}", "x".repeat(140)),
    ] {
        let got = model_tool_name("srv", &raw);
        assert!(got.len() <= MAX_MODEL_TOOL_NAME, "{}", got.len());
        assert!(
            got.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        assert!(got.starts_with(&prefix));
    }
    // Go TestModelToolNameLeavesFittingNamesAlone.
    assert_eq!(
        model_tool_name("srv", &fitting),
        format!("{prefix}{fitting}")
    );
}

#[test]
fn long_siblings_stay_apart_and_are_deterministic() {
    // Go TestModelToolNameKeepsLongSiblingsApart.
    let head = "a".repeat(140);
    let a = model_tool_name("srv", &format!("{head}_one"));
    let b = model_tool_name("srv", &format!("{head}_two"));
    assert_ne!(a, b);
    assert_eq!(a, model_tool_name("srv", &format!("{head}_one")));
}

#[test]
fn oversized_server_prefix_still_owns_its_prefix() {
    // Go TestModelToolNameWithAnOversizedServerPrefix.
    let server = "s".repeat(130);
    let prefix = tool_prefix(&server);
    assert!(model_tool_name(&server, "tool").starts_with(&prefix));
}

#[test]
fn bounded_tool_name_keeps_prefix_and_hash_when_truncating() {
    let prefix = "mcp__srv__";
    let tail = "a".repeat(200);
    let got = bounded_tool_name(prefix, &tail);
    assert_eq!(got.len(), MAX_MODEL_TOOL_NAME);
    assert!(got.starts_with(prefix));
    assert!(got.ends_with(&format!("_{}", short_name_hash(&tail))));
}

#[test]
fn validate_catalog_rejects_empty_names() {
    // Go validateMCPToolNames ErrAmbiguousToolCatalog.
    assert_eq!(
        validate_mcp_tool_names("srv", "", &["ok".into(), "   ".into()]).unwrap_err(),
        NamingError::EmptyToolName
    );
    assert!(validate_mcp_tool_names("srv", "", &["ok".into()]).is_ok());
}

#[test]
fn validate_catalog_rejects_colliding_model_visible_names() {
    // Go toolname_collision_test.go: a raw name and its normalized form share
    // one model-visible name; "a.b" and "a_b" must NOT collide (the hash).
    assert!(validate_mcp_tool_names("srv", "", &["a.b".into(), "a_b".into()]).is_ok());
    let collision = vec!["a.b".to_string(), normalize_name("a.b")];
    let err = validate_mcp_tool_names("srv", "", &collision).unwrap_err();
    let shared = format!("mcp__srv__a_b_{}", short_name_hash("a.b"));
    // Check the diagnostic before destructuring so no field is moved out.
    assert!(err.to_string().contains("colliding names"));
    match err {
        NamingError::Collision {
            previous,
            candidate,
            shared: got,
        } => {
            assert_eq!(previous, "a.b");
            assert_eq!(candidate, normalize_name("a.b"));
            assert_eq!(got, shared);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn validate_catalog_respects_strip_raw_prefix() {
    let raw = ["server_search".to_string(), "server_fetch".to_string()];
    assert!(validate_mcp_tool_names("srv", "server_", &raw).is_ok());
    assert_eq!(
        tool_name("srv", raw[0].strip_prefix("server_").unwrap()),
        "mcp__srv__search"
    );
}

#[test]
fn mcp_roots_projection_byte_exact() {
    assert!(mcp_roots("").is_empty());
    assert!(mcp_roots("   ").is_empty());

    // Pure projection cases (no cwd involvement), matching Go's URL building:
    // volume paths get a leading slash, UNC hosts map to the URL host.
    let proj = mcp_roots_from_absolute("C:/Users/me/proj");
    assert_eq!(proj[0].uri, "file:///C:/Users/me/proj");
    assert_eq!(proj[0].name, "proj");
    assert_eq!(
        mcp_roots_from_absolute("C:\\Users\\me\\proj")[0].uri,
        "file:///C:/Users/me/proj"
    );
    let unc = mcp_roots_from_absolute("//server/share/dir");
    assert_eq!(unc[0].uri, "file://server/share/dir");
    assert_eq!(unc[0].name, "dir");

    // A volume-less absolute path projects as-is (Go's filepath.Abs resolves
    // the drive first on Windows, an environment detail kept out of the seam).
    let plain = mcp_roots_from_absolute("/tmp/proj");
    assert_eq!(plain[0].uri, "file:///tmp/proj");
    assert_eq!(plain[0].name, "proj");

    // Root separators: the whole path becomes the name, URI keeps the slash.
    assert_eq!(mcp_roots_from_absolute("/")[0].uri, "file:///");
    assert_eq!(mcp_roots_from_absolute("/")[0].name, "/");
    assert_eq!(mcp_roots_from_absolute("//server")[0].uri, "file://server/");

    assert_eq!(mcp_roots("  C:/work  ")[0].uri, "file:///C:/work");
}
