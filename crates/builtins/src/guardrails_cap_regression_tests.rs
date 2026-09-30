    fn utility_checks_before_jev(stage: &str, check_type: &str) -> serde_json::Value {
        let utility_cap = if check_type == "moderation" {
            MAX_MODERATION_CALLS_PER_INVOCATION
        } else {
            MAX_JUDGE_CALLS_PER_INVOCATION
        };
        let mut checks = (0..=utility_cap)
            .map(|index| {
                if check_type == "moderation" {
                    json!({
                        "id": format!("utility_{index}"),
                        "stage": stage,
                        "type": "moderation"
                    })
                } else {
                    json!({
                        "id": format!("utility_{index}"),
                        "stage": stage,
                        "type": "llm_judge",
                        "prompt": "allow"
                    })
                }
            })
            .collect::<Vec<_>>();
        checks.push(if check_type == "moderation" {
            json!({
                "id": "jev_block",
                "stage": stage,
                "type": "moderation",
                "engine": "jev"
            })
        } else {
            json!({
                "id": "jev_block",
                "stage": stage,
                "type": "llm_judge",
                "engine": "jev",
                "prompt": "block"
            })
        });
        json!({"checks": checks})
    }

    #[tokio::test]
    async fn utility_moderation_cap_does_not_skip_later_jev_check() {
        let config = utility_checks_before_jev("output", "moderation");
        let utility: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":0}}"#);
        let decision = run_moderation_seam_with(
            &config,
            Some(utility),
            Some(StubJudgment::severe(1.0)),
            "blocked content",
        )
        .await;

        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn utility_tool_use_cap_does_not_skip_later_jev_check() {
        let config = utility_checks_before_jev("tool_use", "llm_judge");
        let hooks = GuardrailsCapability.pre_tool_use_hooks_with_config(&config);
        let ctx = ToolContext::new(SessionId::new())
            .with_utility_llm_service(StubJudge::allow())
            .with_decisions(StubJudgment::noul(1.0));
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;

        assert!(matches!(decision, PreToolUseDecision::Block { .. }));
    }

    #[tokio::test]
    async fn utility_tool_output_cap_does_not_skip_later_jev_check() {
        let config = utility_checks_before_jev("tool_output", "llm_judge");
        let hooks = GuardrailsCapability.post_tool_exec_hooks_with_config(&config);
        let ctx = ToolContext::new(SessionId::new())
            .with_utility_llm_service(StubJudge::allow())
            .with_decisions(StubJudgment::noul(1.0));
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("blocked content")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: Some("blocked raw output".to_string()),
        };
        hooks[0]
            .after_exec(
                &tool_call("any_tool", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;

        assert_eq!(result.result, Some(json!(DEFAULT_TOOL_OUTPUT_REPLACEMENT)));
        assert!(result.raw_output.is_none());
    }

