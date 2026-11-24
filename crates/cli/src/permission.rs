//! `--permission-mode` parsing (reference `cli.go:235-258`).
//!
//! The same parser backs `run`, the TUI, and `acp`; callers map a failure to
//! stderr with the `error:` prefix and exit code 2.

/// Approval posture a permission mode selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    Ask,
    ReadOnly,
    Auto,
    AskAllowEdits,
    DontAsk,
    Plan,
    Yolo,
}

/// Resolved permission posture: approval posture, the plan flag, and the
/// write tools accepted up front (the `acceptEdits` allow-list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionMode {
    pub approval: ApprovalMode,
    pub plan: bool,
    pub allow: Vec<&'static str>,
}

/// Write tools the `acceptEdits` / `accept-edits` mode accepts up front
/// (reference cli.go:244-246).
pub const ACCEPTEDITS_TOOLS: [&str; 7] = [
    "write_file",
    "edit_file",
    "multi_edit",
    "move_file",
    "notebook_edit",
    "delete_range",
    "delete_symbol",
];

/// Parses a raw `--permission-mode` value. Matching is case-insensitive after
/// trimming; the error text quotes the original, un-normalized value.
pub fn parse_permission_mode(value: &str) -> Result<PermissionMode, String> {
    let normalized = value.trim().to_lowercase();
    let mode = match normalized.as_str() {
        "" | "default" | "ask" => PermissionMode {
            approval: ApprovalMode::Ask,
            plan: false,
            allow: Vec::new(),
        },
        "read-only" | "readonly" => PermissionMode {
            approval: ApprovalMode::ReadOnly,
            plan: false,
            allow: Vec::new(),
        },
        "auto" | "workspace-write" => PermissionMode {
            approval: ApprovalMode::Auto,
            plan: false,
            allow: Vec::new(),
        },
        "acceptedits" | "accept-edits" => PermissionMode {
            approval: ApprovalMode::Ask,
            plan: false,
            allow: ACCEPTEDITS_TOOLS.to_vec(),
        },
        "manual" => PermissionMode {
            approval: ApprovalMode::Ask,
            plan: false,
            allow: Vec::new(),
        },
        "dontask" | "dont-ask" => PermissionMode {
            approval: ApprovalMode::DontAsk,
            plan: false,
            allow: Vec::new(),
        },
        "plan" => PermissionMode {
            approval: ApprovalMode::Ask,
            plan: true,
            allow: Vec::new(),
        },
        "bypasspermissions" | "bypass-permissions" | "yolo" | "danger-full-access" => {
            PermissionMode {
                approval: ApprovalMode::Yolo,
                plan: false,
                allow: Vec::new(),
            }
        }
        _ => {
            return Err(format!(
                "unknown permission mode {:?} (want read-only, manual, ask, auto, acceptEdits, dontAsk, plan, or bypassPermissions; 1.x's workspace-write and danger-full-access also work)",
                value
            ));
        }
    };
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_map_to_posture() {
        let cases: [(&str, ApprovalMode, bool); 17] = [
            ("", ApprovalMode::Ask, false),
            ("default", ApprovalMode::Ask, false),
            ("ask", ApprovalMode::Ask, false),
            ("read-only", ApprovalMode::ReadOnly, false),
            ("readonly", ApprovalMode::ReadOnly, false),
            ("auto", ApprovalMode::Auto, false),
            ("workspace-write", ApprovalMode::Auto, false),
            ("acceptedits", ApprovalMode::Ask, false),
            ("accept-edits", ApprovalMode::Ask, false),
            ("manual", ApprovalMode::Ask, false),
            ("dontask", ApprovalMode::DontAsk, false),
            ("dont-ask", ApprovalMode::DontAsk, false),
            ("plan", ApprovalMode::Ask, true),
            ("bypasspermissions", ApprovalMode::Yolo, false),
            ("bypass-permissions", ApprovalMode::Yolo, false),
            ("yolo", ApprovalMode::Yolo, false),
            ("danger-full-access", ApprovalMode::Yolo, false),
        ];
        for (raw, approval, plan) in cases {
            let mode = parse_permission_mode(raw).expect(raw);
            assert_eq!(mode.approval, approval, "{raw}");
            assert_eq!(mode.plan, plan, "{raw}");
        }
    }

    #[test]
    fn acceptedits_carries_the_write_tool_allow_list() {
        let mode = parse_permission_mode("accept-edits").unwrap();
        assert_eq!(mode.allow, ACCEPTEDITS_TOOLS);
    }

    #[test]
    fn matching_is_case_and_space_insensitive() {
        let mode = parse_permission_mode("  Auto ").unwrap();
        assert_eq!(mode.approval, ApprovalMode::Auto);
        let mode = parse_permission_mode("DONTASK").unwrap();
        assert_eq!(mode.approval, ApprovalMode::DontAsk);
    }

    #[test]
    fn invalid_value_keeps_raw_text_in_message() {
        let err = parse_permission_mode("sneaky").unwrap_err();
        assert_eq!(
            err,
            "unknown permission mode \"sneaky\" (want read-only, manual, ask, auto, acceptEdits, dontAsk, plan, or bypassPermissions; 1.x's workspace-write and danger-full-access also work)"
        );
    }
}
