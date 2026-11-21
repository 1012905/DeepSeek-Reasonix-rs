use std::sync::Arc;

    use reasonix_provider::Usage;

    use super::*;
    use crate::window::testing::*;

    // Go: TestContextUsedTokensMatchesTheTriggerInput — the gauge and the
    // compaction trigger must read the same number, and a stale tiny
    // reading from the previous turn must not become the gauge.
    #[test]
    fn the_gauge_matches_the_trigger_input() {
        let mut cfg = config(1_000_000, 0.0);
        cfg.shared_window = crate::window::SharedWindowPolicy::default();
        let window = Arc::new(window_over(cfg, transcript_with_tool_results(12)));
        // A stale, tiny reading from the previous turn — exactly what a fold
        // leaves behind, and what the gauge used to display.
        window.store_latest_request_usage(&Usage {
            prompt_tokens: 900,
            completion_tokens: 100,
            ..Usage::default()
        });

        let used = window.context_used_tokens();
        let projected = window.context_maintenance_snapshot().projected_tokens;
        assert_eq!(
            used, projected,
            "gauge and trigger input must be the same measurement"
        );
        assert!(used > 1_000, "gauge = {used}, want the real view size");
    }

    // Go: TestContextUsedTokensIncludesToolSchemasLikeTheTrigger — the gauge
    // must price tool schemas like the trigger, or a session displays 80%
    // while the trigger already crossed compact_ratio.
    #[test]
    fn the_gauge_includes_tool_schemas_like_the_trigger() {
        let registry = Arc::new(reasonix_tools::Registry::new());
        registry.add(Arc::new(BigSchemaTool));
        let window = Window::new(
            config(1_000_000, 0.0),
            Arc::new(reasonix_state::sessionstore::Session::new("")),
            registry,
            Arc::new(crate::sink::Discard),
        );
        let used = window.context_used_tokens();
        let projected = window.context_maintenance_snapshot().projected_tokens;
        assert_eq!(used, projected);
        let msg_only = crate::window::estimate::estimated_prompt_tokens(
            &window,
            &window.model_visible_messages(),
        );
        assert!(
            used > msg_only,
            "gauge = {used}, message-only estimate = {msg_only}; the gauge must price tool \
             schemas like the trigger"
        );
    }

    // Go: TestContextUsedTokensFollowsLiveToolRegistry.
    #[test]
    fn the_gauge_follows_the_live_tool_registry() {
        let registry = Arc::new(reasonix_tools::Registry::new());
        let window = Window::new(
            config(1_000_000, 0.0),
            Arc::new(reasonix_state::sessionstore::Session::new("")),
            registry.clone(),
            Arc::new(crate::sink::Discard),
        );
        let without_tools = window.context_used_tokens();
        registry.add(Arc::new(BigSchemaTool));
        let with_tools = window.context_used_tokens();
        assert!(with_tools > without_tools, "registration must grow the gauge");

        assert_eq!(registry.remove_prefix("big_"), 1);
        assert_eq!(window.context_used_tokens(), without_tools);

        registry.add(Arc::new(BigSchemaTool));
        assert_eq!(window.context_used_tokens(), with_tools);
        assert_eq!(registry.suspend_prefix("big_"), 1);
        assert_eq!(window.context_used_tokens(), without_tools);
    }

    // Go: TestContextUsedTokensIsZeroWithoutASession — an empty view reads
    // zero so the frontend hides the gauge.
    #[test]
    fn an_empty_view_reads_zero() {
        let window = window_over(config(1_000_000, 0.0), vec![]);
        assert_eq!(window.context_used_tokens(), 0);
    }

    // Go: TestContextUsedTokensFollowsTheTranscript.
    #[test]
    fn the_gauge_follows_the_transcript() {
        let window = Arc::new(window_over(
            config(1_000_000, 0.0),
            transcript_with_tool_results(4),
        ));
        let before = window.context_used_tokens();
        assert_eq!(window.context_used_tokens(), before, "repeated reads agree");
        window.session().add(user(&"more context\n".repeat(500)));
        let after = window.context_used_tokens();
        assert!(after > before, "gauge {before} -> {after}, want growth");
    }

    // Go: usage_anchor.go — the provider's count floors the reading until a
    // fold invalidates the view it described.
    #[test]
    fn the_provider_floor_lasts_until_the_next_fold() {
        let window = window_over(config(1_000_000, 0.0), vec![user("task")]);
        let shape = crate::window::visible_request_shape(&window, &window.model_visible_messages());
        crate::window::usage::store_anchor_shape(&window, shape, window.current_projection_version());
        crate::window::usage::anchor_provider_usage(
            &window,
            &Usage {
                prompt_tokens: 500,
                ..Usage::default()
            },
        );
        let used = window.context_used_tokens();
        assert_eq!(
            used, 500,
            "the provider's count outranks the local estimate, high or low"
        );
        // A fold moves the projection version: the reported size describes a
        // context that no longer exists, so the estimate stands again.
        let mut bigger = user(&"growth ".repeat(2_000));
        bigger.created_at = 0;
        window.session().add(bigger);
        let used = window.context_used_tokens();
        assert!(
            used > 500,
            "after the anchor's view is gone the estimate must stand: {used}"
        );
    }

    // Go: setPromptTokenCalibrationFromUsage — estimated usage must not
    // replace the provider's calibration, but it is retained for accounting.
    #[test]
    fn estimated_usage_never_replaces_the_calibration() {
        let window = window_over(config(0, 0.0), vec![]);
        let shape = crate::window::RequestCalibrationShape {
            request_chars: 200_000,
            compact_chars: 100_000,
            ..crate::window::RequestCalibrationShape::default()
        };
        let learned = crate::window::RequestCalibrationShape {
            request_chars: 100_000,
            compact_chars: 80_000,
            ..crate::window::RequestCalibrationShape::default()
        };
        crate::window::estimate::set_prompt_token_calibration(&window, 50_000, &learned);
        crate::window::usage::store_active_shape(&window, shape);
        window.store_latest_request_usage(&Usage {
            prompt_tokens: 10_000,
            total_tokens: 10_100,
            estimated: true,
            ..Usage::default()
        });
        let cal = crate::window::estimate::OutputEstimateState::lock(window.output_state())
            .prompt_calibration
            .clone()
            .expect("calibration");
        assert_eq!(
            (cal.prompt_tokens, cal.request_chars, cal.compact_chars),
            (50_000, 100_000, 80_000),
            "estimated usage replaced provider calibration"
        );
        let latest = crate::window::estimate::OutputEstimateState::lock(window.output_state())
            .last_usage
            .clone()
            .expect("last usage");
        assert!(latest.estimated, "estimated usage was not retained");
    }

    // Go: TestLatestUsagePairsWithActiveRequestSize — the calibration learns
    // about the request actually sent.
    #[test]
    fn the_calibration_pairs_with_the_active_request() {
        let window = window_over(config(0, 0.0), vec![]);
        crate::window::usage::store_active_shape(
            &window,
            crate::window::RequestCalibrationShape {
                request_chars: 222,
                compact_chars: 111,
                cjk_runes: 22,
                cjk_bytes: 66,
                ..crate::window::RequestCalibrationShape::default()
            },
        );
        window.store_latest_request_usage(&Usage {
            prompt_tokens: 100,
            ..Usage::default()
        });
        let cal = crate::window::estimate::OutputEstimateState::lock(window.output_state())
            .prompt_calibration
            .clone()
            .expect("calibration");
        assert_eq!(cal.prompt_tokens, 100);
        assert_eq!(cal.request_chars, 222);
        assert_eq!(cal.compact_chars, 111);
        assert_eq!(cal.cjk_runes, 22);
        assert_eq!(cal.cjk_bytes, 66);
    }

    // Go: TestSetSessionResetsPerTranscriptUsageState — lastUsage and the
    // active shape belong to the transcript; the calibration outlives it.
    #[test]
    fn a_conversation_rebind_resets_per_transcript_usage_state() {
        let window = window_over(config(0, 0.0), vec![]);
        window.store_latest_request_usage(&Usage {
            prompt_tokens: 200_000,
            ..Usage::default()
        });
        crate::window::usage::store_active_shape(
            &window,
            crate::window::RequestCalibrationShape {
                request_chars: 900_000,
                compact_chars: 850_000,
                ..crate::window::RequestCalibrationShape::default()
            },
        );
        crate::window::estimate::set_prompt_token_calibration(
            &window,
            200_000,
            &crate::window::RequestCalibrationShape {
                request_chars: 1_000_000,
                compact_chars: 950_000,
                ..crate::window::RequestCalibrationShape::default()
            },
        );
        window.reset();
        let out = crate::window::estimate::OutputEstimateState::lock(window.output_state());
        assert!(out.last_usage.is_none(), "lastUsage survived the swap");
        assert!(out.active_shape.is_none(), "activeReqShape survived the swap");
        assert!(
            out.prompt_calibration.is_some(),
            "promptCalibration was dropped; the tokenizer ratio outlives the transcript"
        );
    }

    fn transcript_with_tool_results(tool_results: usize) -> Vec<reasonix_contract::provider::Message> {
        use reasonix_contract::provider::{Message, Role, ToolCall};
        let big = "line\n".repeat(400);
        let mut msgs = vec![system("system"), user("task")];
        for i in 0..tool_results {
            let id = format!("call-{i}");
            msgs.push(Message {
                role: Role::ASSISTANT,
                tool_calls: vec![ToolCall {
                    id: id.clone(),
                    name: "read_file".to_owned(),
                    arguments: "{}".to_owned(),
                    ..ToolCall::default()
                }],
                ..Message::default()
            });
            msgs.push(Message {
                role: Role::TOOL,
                tool_call_id: id,
                name: "read_file".to_owned(),
                content: big.clone(),
                ..Message::default()
            });
        }
        msgs
    }

    // Go: bigSchemaTool — the compaction trigger counts tool schemas in the
    // prompt it sizes, so the gauge must too.
    struct BigSchemaTool;

    impl reasonix_tools::Tool for BigSchemaTool {
        fn name(&self) -> &str {
            "big_schema"
        }

        fn description(&self) -> &str {
            "a tool with a large schema"
        }

        fn schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "a fairly long property \
                        description that consumes tokens. ".repeat(200)},
                    "query": {"type": "string", "description": "another long field to \
                        inflate the schema"}
                }
            })
        }

        fn execute(
            &self,
            _ctx: &reasonix_tools::CallContext,
            _args: &serde_json::Value,
        ) -> reasonix_tools::ToolResult {
            Ok(String::new())
        }

        fn read_only(&self) -> bool {
            true
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
