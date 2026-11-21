//! Byte-exact golden wire tests (ledger SSE-014..SSE-021). Goldens are the
//! Go `json.Marshal` of the same struct values, generated from
//! `internal/contract/eventwire` and `internal/contract/event`. Field order,
//! omitempty and the snake_case guardian quirk are part of the contract.

use reasonix_contract::eventwire::approval::{Approval, RecoveryApproval};
use reasonix_contract::eventwire::ask::{Ask, AskOption, AskQuestion};
use reasonix_contract::eventwire::event::{
    Compaction, CompletionSummary, ContextMaintenance, DecisionReceipt, ProgressWatch,
    TodoProgress, WorkspaceChanged, WorkspacePathChange, WorkspaceRevision,
};
use reasonix_contract::eventwire::extension::{ExtensionStatus, ExtensionSurface};
use reasonix_contract::eventwire::guardian::Guardian;
use reasonix_contract::eventwire::lease::WorkspaceLease;
use reasonix_contract::eventwire::notice_payload::{
    CompactionNotice, ContextComposition, ContextPart, DormantPermissionRule, ExtensionSkipped,
    InboxRecovered, InboxUnrecovered, JobNotice, PermissionRulesDormant, SubagentWaiting,
    TurnsDropped,
};
use reasonix_contract::eventwire::receipt::CompletionReceipt;
use reasonix_contract::eventwire::tool::{Bound, Profile, Tool};
use reasonix_contract::eventwire::usage::Usage;
use reasonix_contract::gojson::marshal;

fn golden(name: &str, value: &str, expected: &str) {
    assert_eq!(value, expected, "{name}: Rust wire bytes differ from Go");
    let back: serde_json::Value =
        serde_json::from_str(value).unwrap_or_else(|e| panic!("{name}: {e}"));
    let want: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(back, want, "{name}: round-trip changed the JSON object");
}

#[test]
fn tool_wire_matches_go_bytes() {
    let t = Tool {
        id: "call-1".into(),
        name: "task".into(),
        args: r#"{"prompt":"x"}"#.into(),
        output: "ignored".into(),
        err: "blocked".into(),
        read_only: true,
        duration_ms: 522,
        bound: Some(Bound {
            kind: "truncated".into(),
            lines: 900,
            bytes: 40000,
            kept_bytes: 32768,
            ..Bound::default()
        }),
        started_at: 1754500000000,
        ended_at: 1754500000522,
        partial: true,
        refreshed: true,
        parent_id: "parent-1".into(),
        diff: "@@ -1 +1 @@\n-old\n+new\n".into(),
        added: 1,
        removed: 1,
        profile: Some(Profile {
            model: "deepseek-pro".into(),
            effort: "max".into(),
            ..Profile::default()
        }),
        ..Tool::default()
    };
    golden(
        "tool_full",
        &marshal(&t).unwrap(),
        r#"{"id":"call-1","name":"task","args":"{\"prompt\":\"x\"}","output":"ignored","err":"blocked","readOnly":true,"bound":{"kind":"truncated","lines":900,"bytes":40000,"keptBytes":32768},"durationMs":522,"startedAt":1754500000000,"endedAt":1754500000522,"partial":true,"refreshed":true,"parentId":"parent-1","diff":"@@ -1 +1 @@\n-old\n+new\n","added":1,"removed":1,"profile":{"model":"deepseek-pro","effort":"max"}}"#,
    );
}

#[test]
fn bound_wire_matches_go_bytes() {
    let b = Bound {
        kind: "spilled".into(),
        lines: 12,
        bytes: 345,
        path: "/tmp/out".into(),
        ..Bound::default()
    };
    golden(
        "bound_spilled",
        &marshal(&b).unwrap(),
        r#"{"kind":"spilled","lines":12,"bytes":345,"path":"/tmp/out"}"#,
    );
    assert!(!b.lossy());
    assert!(
        Bound {
            kind: "truncated".into(),
            ..Bound::default()
        }
        .lossy()
    );
}

#[test]
fn usage_wire_matches_go_bytes() {
    let u = Usage {
        prompt_tokens: 1000,
        completion_tokens: 200,
        total_tokens: 1200,
        cache_hit_tokens: 900,
        cache_miss_tokens: 100,
        reasoning_tokens: 33,
        estimated: true,
        source: "title".into(),
        session_cache_hit_tokens: 8000,
        session_cache_miss_tokens: 2000,
        ..Usage::default()
    };
    golden(
        "usage",
        &marshal(&u).unwrap(),
        r#"{"promptTokens":1000,"completionTokens":200,"totalTokens":1200,"cacheHitTokens":900,"cacheMissTokens":100,"reasoningTokens":33,"estimated":true,"source":"title","sessionCacheHitTokens":8000,"sessionCacheMissTokens":2000}"#,
    );
}

#[test]
fn approval_and_ask_wires_match_go_bytes() {
    golden(
        "approval",
        &marshal(&Approval {
            id: "a1".into(),
            tool: "bash".into(),
            subject: "rm".into(),
            ..Approval::default()
        })
        .unwrap(),
        r#"{"id":"a1","tool":"bash","subject":"rm"}"#,
    );
    golden(
        "approval_scoped",
        &marshal(&Approval {
            id: "a4".into(),
            tool: "best_of_n".into(),
            scope: "unattended_attempts".into(),
            ..Approval::default()
        })
        .unwrap(),
        r#"{"id":"a4","tool":"best_of_n","subject":"","scope":"unattended_attempts"}"#,
    );
    golden(
        "recovery",
        &marshal(&RecoveryApproval {
            next_action: "git push origin feature".into(),
            can_grant_task: true,
            task_grant_scope: "git push origin → feature".into(),
            ..RecoveryApproval::default()
        })
        .unwrap(),
        r#"{"next_action":"git push origin feature","can_grant_task":true,"task_grant_scope":"git push origin → feature"}"#,
    );
    golden(
        "ask",
        &marshal(&Ask {
            id: "ask-1".into(),
            questions: vec![AskQuestion {
                id: "q1".into(),
                header: "Pick".into(),
                prompt: "Choose".into(),
                multi: true,
                options: vec![
                    AskOption {
                        label: "A".into(),
                        description: "Alpha".into(),
                    },
                    AskOption {
                        label: "B".into(),
                        ..AskOption::default()
                    },
                ],
                ..AskQuestion::default()
            }],
            ..Ask::default()
        })
        .unwrap(),
        r#"{"id":"ask-1","questions":[{"id":"q1","header":"Pick","prompt":"Choose","options":[{"label":"A","description":"Alpha"},{"label":"B"}],"multi":true}]}"#,
    );
}

#[test]
fn extension_wire_matches_go_bytes() {
    golden(
        "extension_status",
        &marshal(&ExtensionSurface {
            plugin_id: "alpha".into(),
            surface_id: "s1".into(),
            session_id: "sess".into(),
            generation: 7,
            kind: "status".into(),
            status: Some(ExtensionStatus {
                label: "working".into(),
                detail: "half".into(),
                severity: "warn".into(),
                ..ExtensionStatus::default()
            }),
            ..ExtensionSurface::default()
        })
        .unwrap(),
        r#"{"pluginId":"alpha","surfaceId":"s1","sessionId":"sess","generation":7,"kind":"status","status":{"label":"working","detail":"half","severity":"warn"}}"#,
    );
}

#[test]
fn summary_and_receipt_wires_match_go_bytes() {
    golden(
        "completion_summary",
        &marshal(&CompletionSummary {
            preset: "delivery".into(),
            verdict: "complete".into(),
            mutations: 2,
            checks_passed: 3,
            review: "passed".into(),
            ..CompletionSummary::default()
        })
        .unwrap(),
        r#"{"preset":"delivery","verdict":"complete","mutations":2,"checks_passed":3,"checks_failed":0,"checks_suppressed":0,"review":"passed"}"#,
    );
    golden(
        "completion_receipt",
        &marshal(&CompletionReceipt {
            verdict: "done".into(),
            risks: vec!["r".into()],
            says_something: true,
            severity: "notice".into(),
            ..CompletionReceipt::default()
        })
        .unwrap(),
        r#"{"verdict":"done","risks":["r"],"saysSomething":true,"severity":"notice"}"#,
    );
}

#[test]
fn maintenance_wires_match_go_bytes() {
    golden(
        "compaction",
        &marshal(&Compaction {
            trigger: "manual".into(),
            messages: 7,
            summary: "brief".into(),
            source_tokens: 100,
            projection_tokens: 40,
            ..Compaction::default()
        })
        .unwrap(),
        r#"{"trigger":"manual","messages":7,"summary":"brief","sourceTokens":100,"projectionTokens":40}"#,
    );
    golden(
        "context_maintenance",
        &marshal(&ContextMaintenance {
            status: "applied".into(),
            action: "prune".into(),
            saved_tokens: 4096,
            projection_version: 3,
            cache_break: true,
            ..ContextMaintenance::default()
        })
        .unwrap(),
        r#"{"status":"applied","action":"prune","savedTokens":4096,"projectionVersion":3,"cacheBreak":true}"#,
    );
    golden(
        "todo",
        &marshal(&TodoProgress {
            kind: "advanced".into(),
            steps: 4,
            completed: 2,
            ..TodoProgress::default()
        })
        .unwrap(),
        r#"{"kind":"advanced","steps":4,"completed":2}"#,
    );
    golden(
        "progress_watch",
        &marshal(&ProgressWatch {
            stalled: true,
            cause: "tokens".into(),
            idle_rounds: 6,
            prompt_tokens: 90000,
            token_limit: 100000,
            token_multiple: 3,
            pausing: true,
            ..ProgressWatch::default()
        })
        .unwrap(),
        r#"{"stalled":true,"cause":"tokens","idleRounds":6,"promptTokens":90000,"tokenLimit":100000,"tokenMultiple":3,"pausing":true}"#,
    );
    golden(
        "lease",
        &marshal(&WorkspaceLease {
            contended: 1,
            held_ms: 1200,
            idle_ms: 30,
            holder: "other".into(),
            paths: vec!["a".into()],
            requested_paths: vec!["b".into()],
            ..WorkspaceLease::default()
        })
        .unwrap(),
        r#"{"contended":1,"heldMs":1200,"idleMs":30,"holder":"other","paths":["a"],"requestedPaths":["b"]}"#,
    );
}

#[test]
fn guardian_and_workspace_wires_match_go_bytes() {
    golden(
        "guardian",
        &marshal(&Guardian {
            id: "g1".into(),
            tool: "bash".into(),
            subject: "rm".into(),
            outcome: "allow".into(),
            risk_level: "high".into(),
            user_authorization: "low".into(),
            duration_ms: 42,
            ..Guardian::default()
        })
        .unwrap(),
        r#"{"id":"g1","tool":"bash","subject":"rm","outcome":"allow","risk_level":"high","user_authorization":"low","duration_ms":42}"#,
    );
    golden(
        "workspace_changed",
        &marshal(&WorkspaceChanged {
            revisions: WorkspaceRevision {
                content: 4,
                tree: 2,
                working_tree: 3,
                git_meta: 1,
                session: 7,
            },
            changes: vec![],
            source: "reconcile".into(),
            watch_state: "degraded".into(),
            ..WorkspaceChanged::default()
        })
        .unwrap(),
        r#"{"revisions":{"content":4,"tree":2,"workingTree":3,"gitMeta":1,"session":7},"changes":[],"allPaths":false,"source":"reconcile","watchState":"degraded"}"#,
    );
    let _ = WorkspacePathChange::default();
    golden(
        "decision_receipt",
        &marshal(&DecisionReceipt {
            id: "approval-1".into(),
            kind: "tool".into(),
            tool: "write_file".into(),
            subject: "src/app.go".into(),
            outcome: "allow_once".into(),
            ..DecisionReceipt::default()
        })
        .unwrap(),
        r#"{"id":"approval-1","kind":"tool","tool":"write_file","subject":"src/app.go","outcome":"allow_once"}"#,
    );
}

#[test]
fn notice_payload_goldens_match_go_bytes() {
    golden(
        "pl_extension_skipped",
        &ExtensionSkipped {
            extension: "aipush".into(),
            point: "tool.before".into(),
            reason: "no_live_sidecar".into(),
        }
        .encode(),
        r#"{"extension":"aipush","point":"tool.before","reason":"no_live_sidecar"}"#,
    );
    golden(
        "pl_inbox_recovered",
        &InboxRecovered { count: 3 }.encode(),
        r#"{"count":3}"#,
    );
    golden(
        "pl_inbox_unrecovered",
        &InboxUnrecovered {
            dir: "/tmp/inbox".into(),
        }
        .encode(),
        r#"{"dir":"/tmp/inbox"}"#,
    );
    golden(
        "pl_job",
        &JobNotice {
            kind: "shell".into(),
            id: "bash-3".into(),
            label: "build".into(),
            exit_code: Some(2),
            quiet_seconds: 30,
            steps: 4,
            ..JobNotice::default()
        }
        .encode(),
        r#"{"kind":"shell","id":"bash-3","label":"build","exitCode":2,"quietSeconds":30,"steps":4}"#,
    );
    golden(
        "pl_rules_dormant",
        &PermissionRulesDormant {
            rules: vec![DormantPermissionRule {
                list: "allow".into(),
                rule: "bash:*".into(),
                tool: "bash".into(),
            }],
        }
        .encode(),
        r#"{"rules":[{"list":"allow","rule":"bash:*","tool":"bash"}]}"#,
    );
    golden(
        "pl_context_composition",
        &ContextComposition {
            total: 100,
            at: 80,
            active: 20,
            parts: vec![ContextPart {
                name: "system".into(),
                tokens: 40,
            }],
            largest: vec![],
        }
        .encode(),
        r#"{"total":100,"at":80,"active":20,"parts":[{"name":"system","tokens":40}]}"#,
    );
    golden(
        "pl_subagent_waiting",
        &SubagentWaiting {
            task: "t1".into(),
            by: "writer".into(),
            whole: true,
        }
        .encode(),
        r#"{"task":"t1","by":"writer","whole":true}"#,
    );
    golden(
        "pl_compaction_notice",
        &CompactionNotice {
            trigger: "pressure".into(),
            mode: "summarized".into(),
            source_tokens: 1000,
            fold_tokens: 200,
            spans: 3,
            projection_tokens: 100,
            input_tokens: 1100,
            output_tokens: 200,
            cache_hit_tokens: 900,
            cache_miss_tokens: 100,
            cache_write_tokens: 50,
            request_count: 2,
            user_turns_kept: 4,
            user_turns_dropped: 1,
            ..CompactionNotice::default()
        }
        .encode(),
        r#"{"trigger":"pressure","mode":"summarized","source_tokens":1000,"fold_tokens":200,"spans":3,"projection_tokens":100,"input_tokens":1100,"output_tokens":200,"cache_hit_tokens":900,"cache_miss_tokens":100,"cache_write_tokens":50,"request_count":2,"user_turns_kept":4,"user_turns_dropped":1}"#,
    );
    golden(
        "pl_turns_dropped",
        &TurnsDropped {
            count: 2,
            tokens: 500,
            budget: 400,
        }
        .encode(),
        r#"{"count":2,"tokens":500,"budget":400}"#,
    );
}
