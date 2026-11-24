//! ACP-014: admission order, ACP round-trip fail-closed identity, argv meta
//! decision, and the byte-based tool-result clip boundary.

use reasonix_acp::dispatch::{MAX_RESULT_CHARS, clip, tool_result_text};
use reasonix_acp::gating::{
    Admission, RoundTripInput, StaticFeatures, StaticRejectReason, admit, argv_meta, exact_argv,
    resolve_option_id, resolve_round_trip,
};

#[test]
fn admission_short_circuits_ceiling_then_posture() {
    // Above the startup ceiling refuses before any prompt.
    assert_eq!(admit("yolo", "ask"), Admission::Deny);
    // Read-only and dont-ask refuse.
    assert_eq!(admit("readOnly", "yolo"), Admission::Deny);
    assert_eq!(admit("dontAsk", "yolo"), Admission::Deny);
    // Ask goes to the round-trip; auto/yolo grant.
    assert_eq!(admit("ask", "yolo"), Admission::Prompt);
    assert_eq!(admit("auto", "yolo"), Admission::Grant);
    assert_eq!(admit("yolo", "yolo"), Admission::Grant);
}

#[test]
fn round_trip_fails_closed_on_every_non_selected_branch() {
    for input in [
        RoundTripInput::NoBinder,
        RoundTripInput::TransportFailed,
        RoundTripInput::MalformedReply,
        RoundTripInput::OutcomeNotSelected,
    ] {
        assert!(!resolve_round_trip(input).allow);
    }
    let allow = resolve_round_trip(RoundTripInput::Selected {
        option_id: "allow_once".to_owned(),
    });
    assert!(allow.allow);
    assert!(!allow.session);
    assert!(!allow.persist);

    let persist = resolve_option_id("allow_persist");
    assert!(persist.allow && persist.session && persist.persist);
    assert!(!resolve_option_id("bogus").allow);
}

#[test]
fn exact_argv_rejects_expansion_and_nested_execution() {
    let clean = StaticFeatures::default();
    assert!(exact_argv(&clean, &["ls".to_owned()]).is_ok());
    assert!(exact_argv(&clean, &[]).is_err());

    let expansion = StaticFeatures {
        expansion: true,
        ..Default::default()
    };
    assert_eq!(
        exact_argv(&expansion, &["echo".to_owned()]).unwrap_err(),
        StaticRejectReason::ShellExpansion
    );

    let assignment = StaticFeatures {
        assignment: true,
        ..Default::default()
    };
    assert_eq!(
        exact_argv(&assignment, &["A=1".to_owned()]).unwrap_err(),
        StaticRejectReason::ShellAssignment
    );

    let here_doc = StaticFeatures {
        here_document: true,
        ..Default::default()
    };
    assert_eq!(
        exact_argv(&here_doc, &["cat".to_owned()]).unwrap_err(),
        StaticRejectReason::HereDocument
    );
}

#[test]
fn six_static_reject_reasons_carry_verbatim_labels() {
    let labels = [
        (StaticRejectReason::ParseError, "parse error"),
        (StaticRejectReason::HereDocument, "here document"),
        (
            StaticRejectReason::ShellControlSyntax,
            "shell control syntax",
        ),
        (StaticRejectReason::ShellRedirection, "shell redirection"),
        (StaticRejectReason::ShellAssignment, "shell assignment"),
        (StaticRejectReason::ShellExpansion, "shell expansion"),
    ];
    for (reason, label) in labels {
        assert_eq!(reason.to_string(), label);
    }
}

#[test]
fn argv_meta_hides_command_when_untrusted() {
    let trusted = argv_meta(true, &["ls".to_owned()], "/proj");
    assert_eq!(trusted["commandSchemaVersion"], 1);
    assert_eq!(trusted["argv"], serde_json::json!(["ls"]));
    assert_eq!(trusted["cwd"], "/proj");

    let untrusted = argv_meta(false, &["ls".to_owned()], "/proj");
    assert!(untrusted.is_empty());
}

#[test]
fn clip_keeps_utf8_boundary_and_verbatim_suffix() {
    assert_eq!(clip("short"), "short");

    let one_over = "x".repeat(MAX_RESULT_CHARS + 1);
    let clipped = clip(&one_over);
    assert!(clipped.ends_with("\n…(1 more chars truncated)"));
    assert_eq!(
        clipped.len(),
        MAX_RESULT_CHARS + "\n…(1 more chars truncated)".len()
    );

    let multibyte = "字".repeat(MAX_RESULT_CHARS / 3 + 5);
    let clipped = clip(&multibyte);
    assert!(std::str::from_utf8(clipped.as_bytes()).is_ok());
    assert!(clipped.ends_with("more chars truncated)"));

    // Diff bodies pass through inside a fenced block; plain results clip.
    let diff = "```diff\n- a\n+ b\n```";
    let out = tool_result_text(diff, true);
    assert!(out.starts_with("````diff\n"));
    assert_eq!(tool_result_text("hello", false), "hello");
}
