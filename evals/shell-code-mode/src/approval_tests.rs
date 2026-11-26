use super::*;

#[cfg(feature = "monty-runtime-tests")]
mod runtime {
    use super::*;
    use everruns::{Provider, ToolCall};
    use everruns_contracts::driver_registry::{
        ChatDriver, LlmCallConfig, LlmResponseStream, Message,
    };
    use everruns_llmsim::{LlmSimConfig, LlmSimDriver};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ContinuationModel {
        step: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl ChatDriver for ContinuationModel {
        async fn chat_completion_stream(
            &self,
            endpoint: &everruns_contracts::runtime_provider::ProviderEndpoint,
            messages: Vec<Message>,
            config: &LlmCallConfig,
        ) -> everruns_contracts::error::Result<LlmResponseStream> {
            let step = self.step.fetch_add(1, Ordering::SeqCst);
            let calls = match step {
                0 => vec![ToolCall {
                    id: "start".into(),
                    name: "python".into(),
                    arguments: json!({"script":"r = tool('reserve_change', {'target': 'staging'})\na = tool('apply_change', {'reservation': r['result']['reservation']})\ntool('read_change', {})"}),
                }],
                1 => {
                    let pattern = regex::Regex::new(r"[0-9a-f]{64}:[0-9]{1,3}").unwrap();
                    let token = messages
                        .iter()
                        .rev()
                        .find_map(|m| {
                            pattern
                                .find(&m.content_as_text())
                                .map(|m| m.as_str().to_owned())
                        })
                        .expect("the real paused tool result contains a resume token");
                    vec![ToolCall {
                        id: "continue".into(),
                        name: "python".into(),
                        arguments: json!({"resume":token}),
                    }]
                }
                _ => vec![],
            };
            LlmSimDriver::new(LlmSimConfig::fixed("Done.").with_tool_call_sequence(vec![calls]))
                .chat_completion_stream(endpoint, messages, config)
                .await
        }
    }

    #[tokio::test]
    async fn monty_approval_eval_uses_the_engine_pause_and_saved_continuation() {
        crate::code_mode::preflight("monty").await.unwrap();
        for decision in ["allow", "reject"] {
            let sample = mira::Dataset::from_jsonl_str(crate::DATASET)
                .unwrap()
                .samples
                .into_iter()
                .find(|s| s.id == format!("approval-{decision}"))
                .unwrap();
            let log = CallLog::default();
            let session = crate::subject::build_session_for_surface(
                &sample,
                &mira::Target::new("offline", "offline", "test-model"),
                Provider::new(
                    "offline",
                    ContinuationModel {
                        step: AtomicUsize::new(0),
                    },
                ),
                &log,
                "monty",
            )
            .unwrap();
            let mut transcript = crate::subject::run_turns_with_log(&session, &sample, &log).await;
            crate::subject::finish(&mut transcript, &log);
            crate::code_mode::finish(&mut transcript, "monty");
            assert!(transcript.error.is_none(), "{:?}", transcript.error);
            for scorer in [
                approval_verified(),
                crate::scorers::expected_calls(),
                crate::scorers::forbidden_calls(),
                crate::code_mode::surface_used(),
            ] {
                let score = scorer.score(&sample, &transcript).await;
                assert!(score.pass, "{decision}: {}: {}", score.scorer, score.reason);
            }
        }
    }
}

#[tokio::test]
async fn approval_scorer_rejects_missing_decisions_and_restarted_source() {
    let sample: Sample = serde_json::from_value(
        json!({"id":"approval", "input":["act"], "metadata":{"approval_decision":"allow"}}),
    )
    .unwrap();
    let mut t = Transcript::default();
    assert!(!approval_verified().score(&sample, &t).await.pass);
    t.metadata.insert("surface".into(), "monty".into());
    for (key, value) in [
        ("approval.requests", 1.0),
        ("approval.effects_before_decision", 0.0),
        ("approval.unexpected_request", 0.0),
        ("code.python_scripts", 2.0),
        ("code.python_resumes", 0.0),
    ] {
        t.record_metric(key, value);
    }
    assert!(!approval_verified().score(&sample, &t).await.pass);
    t.record_metric("code.python_scripts", 1.0);
    t.record_metric("code.python_resumes", 1.0);
    assert!(approval_verified().score(&sample, &t).await.pass);
    t.record_metric("approval.effects_before_decision", 1.0);
    assert!(!approval_verified().score(&sample, &t).await.pass);
}
