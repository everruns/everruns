    // ---- llm_judge / mcp on the end-of-message output seam (EVE-572) ----
    //
    // Included into `guardrails.rs`'s test module so the shared fakes
    // (`StubJudge`, `StubJudgment`, `StubMcpInvoker`) stay in one place.

    fn output_check(check: serde_json::Value, mode: &str) -> serde_json::Value {
        json!({ "mode": mode, "checks": [check] })
    }

    fn output_judge(on_fail: &str, replacement: Option<&str>) -> serde_json::Value {
        let mut check = json!({
            "id": "secret-leak-output",
            "stage": "output",
            "type": "llm_judge",
            "prompt": "Block a message that reveals a secret.",
            "on_fail": on_fail,
        });
        if let Some(replacement) = replacement {
            check["replacement"] = json!(replacement);
        }
        check
    }

    fn output_mcp(on_fail: &str) -> serde_json::Value {
        json!({
            "id": "external-output",
            "stage": "output",
            "type": "mcp",
            "server": "guard",
            "tool": "scan",
            "on_fail": on_fail,
            "replacement": "[withheld by external guardrail]",
        })
    }

    async fn run_output_seam(
        config: &serde_json::Value,
        utility: Option<Arc<dyn UtilityLlmService>>,
        judgment: Option<Arc<dyn DecisionsService>>,
        mcp: Option<Arc<dyn crate::builtins::McpToolInvoker>>,
        text: &str,
    ) -> GuardrailDecision {
        let provider = GuardrailsCapability
            .post_output_guardrails_with_config(config)
            .into_iter()
            .next()
            .expect("an output llm_judge/mcp check contributes the end-of-message provider");
        let ctx = PostGenerationOutputContext {
            system_prompt: "",
            message_text: text,
            utility_llm_service: utility.as_ref(),
            decisions: judgment.as_ref(),
            mcp_invoker: mcp.as_ref(),
        };
        provider.check_message(&ctx).await
    }

    #[test]
    fn output_judge_and_mcp_checks_validate_and_contribute_the_message_seam() {
        for check in [output_judge("block", None), output_mcp("block")] {
            let config = output_check(check, "active");
            assert!(GuardrailsCapability.validate_config(&config).is_ok());
            let providers = GuardrailsCapability.post_output_guardrails_with_config(&config);
            assert_eq!(providers.len(), 1);
            assert_eq!(providers[0].id(), "guardrail_checks");
        }
    }

    #[tokio::test]
    async fn output_judge_block_replaces_the_message_with_the_check_replacement() {
        let config = output_check(output_judge("block", Some("[secret removed]")), "active");
        let decision =
            run_output_seam(&config, Some(StubJudge::block()), None, None, "key=abc").await;
        match decision {
            GuardrailDecision::Block(block) => {
                assert_eq!(block.reason_code, "guardrail.llm_judge");
                assert_eq!(block.replacement, "[secret removed]");
            }
            GuardrailDecision::Pass => panic!("expected the judge to block the message"),
        }
    }

    #[tokio::test]
    async fn output_judge_block_defaults_to_the_output_replacement() {
        let config = output_check(output_judge("block", None), "active");
        match run_output_seam(&config, Some(StubJudge::block()), None, None, "x").await {
            GuardrailDecision::Block(block) => {
                assert_eq!(block.replacement, DEFAULT_OUTPUT_REPLACEMENT)
            }
            GuardrailDecision::Pass => panic!("expected block"),
        }
    }

    #[tokio::test]
    async fn output_judge_allow_verdict_passes() {
        let config = output_check(output_judge("block", None), "active");
        let decision = run_output_seam(&config, Some(StubJudge::allow()), None, None, "x").await;
        assert!(matches!(decision, GuardrailDecision::Pass));
    }

    #[tokio::test]
    async fn output_judge_advisory_and_log_only_never_block() {
        for config in [
            output_check(output_judge("block", None), "advisory"),
            output_check(output_judge("log", None), "active"),
        ] {
            let decision =
                run_output_seam(&config, Some(StubJudge::block()), None, None, "x").await;
            assert!(matches!(decision, GuardrailDecision::Pass), "{config}");
        }
    }

    #[tokio::test]
    async fn output_judge_fails_open_on_utility_llm_failure() {
        let config = output_check(output_judge("block", None), "active");
        for utility in [Some(StubJudge::error() as Arc<dyn UtilityLlmService>), None] {
            let decision = run_output_seam(&config, utility, None, None, "x").await;
            assert!(matches!(decision, GuardrailDecision::Pass));
        }
        let malformed: Arc<dyn UtilityLlmService> = StubJudge::malformed("not json");
        let decision = run_output_seam(&config, Some(malformed), None, None, "x").await;
        assert!(matches!(decision, GuardrailDecision::Pass));
    }

    #[tokio::test]
    async fn output_jev_judge_rides_the_batched_request() {
        let mut check = output_judge("block", None);
        check["engine"] = json!("jev");
        let config = output_check(check, "active");
        let judgment = StubJudgment::noul(0.9);
        let decision =
            run_output_seam(&config, None, Some(judgment.clone()), None, "the text").await;
        assert!(matches!(decision, GuardrailDecision::Block(ref b) if b.reason_code == "guardrail.llm_judge"));
        assert_eq!(judgment.requests.lock().unwrap().len(), 1);
        let low = run_output_seam(&config, None, Some(StubJudgment::noul(0.1)), None, "x").await;
        assert!(matches!(low, GuardrailDecision::Pass));
    }

    #[tokio::test]
    async fn utility_output_judge_cap_does_not_skip_later_jev_check() {
        let config = utility_checks_before_jev("output", "llm_judge");
        let decision = run_output_seam(
            &config,
            Some(StubJudge::allow()),
            Some(StubJudgment::noul(1.0)),
            None,
            "x",
        )
        .await;
        assert!(matches!(decision, GuardrailDecision::Block(_)));
    }

    #[tokio::test]
    async fn output_mcp_block_replaces_the_message_and_sends_the_output_stage() {
        let config = output_check(output_mcp("block"), "active");
        let invoker = StubMcpInvoker::block();
        let decision =
            run_output_seam(&config, None, None, Some(invoker.clone()), "final answer").await;
        match decision {
            GuardrailDecision::Block(block) => {
                assert_eq!(block.reason_code, "guardrail.mcp");
                assert_eq!(block.replacement, "[withheld by external guardrail]");
            }
            GuardrailDecision::Pass => panic!("expected the mcp check to block"),
        }
        let call = invoker.last_call.lock().unwrap().clone().expect("mcp called");
        assert_eq!(call.name, mcp_tool_name("guard", "scan"));
        assert_eq!(call.arguments["stage"], "output");
        assert!(call.arguments["tool"].is_null());
        assert_eq!(call.arguments["content"], "final answer");
    }

    #[tokio::test]
    async fn output_mcp_allow_advisory_and_failures_pass() {
        let active = output_check(output_mcp("block"), "active");
        let advisory = output_check(output_mcp("block"), "advisory");
        let error = StubMcpInvoker::new(McpBehavior::Error("down".to_string()));
        for (config, invoker) in [
            (&active, Some(StubMcpInvoker::allow())),
            (&advisory, Some(StubMcpInvoker::block())),
            (&active, Some(error)),
            (&active, None),
        ] {
            let invoker = invoker.map(|i| i as Arc<dyn crate::builtins::McpToolInvoker>);
            let decision = run_output_seam(config, None, None, invoker, "x").await;
            assert!(matches!(decision, GuardrailDecision::Pass));
        }
    }
