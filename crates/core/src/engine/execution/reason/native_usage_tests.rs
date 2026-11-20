use super::*;
use crate::capabilities::Capability;
use crate::engine::driver_registry::MessageRole;
use crate::engine::test_fixtures::TestEventEmitter;
use everruns_contracts::compact::{CompactOutputItem, CompactResponse, CompactUsage};
use everruns_contracts::driver_registry::{ChatDriver, LlmCallConfig, LlmResponseStream};

struct NativeUsageDriver {
    encrypted_bytes: usize,
    billed_output: u32,
    cost: Option<f64>,
}

#[async_trait::async_trait]
impl ChatDriver for NativeUsageDriver {
    fn supports_compact(&self) -> bool {
        true
    }
    async fn chat_completion_stream(
        &self,
        _: &crate::engine::ProviderEndpoint,
        _: Vec<Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        panic!("only native compaction is under test")
    }
    async fn compact(
        &self,
        _: &crate::engine::ProviderEndpoint,
        _: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        Ok(Some(CompactResponse {
            output: vec![CompactOutputItem::Compaction {
                encrypted_content: "x".repeat(self.encrypted_bytes),
            }],
            usage: Some(CompactUsage {
                input_tokens: Some(1000),
                output_tokens: Some(self.billed_output),
                total_tokens: Some(1000 + self.billed_output),
                cost: self.cost,
            }),
        }))
    }
}

#[tokio::test]
async fn explicit_generation_usage_cannot_prove_or_disprove_window_reduction() {
    let policy = crate::builtins::CompactionCapability
        .compaction_policy(&serde_json::json!({"strategy":"native"}))
        .unwrap();
    for (encrypted_bytes, billed_output, reduced, cost) in [
        (32, 10_000, true, Some(0.5)),
        (20_000, 1, false, Some(0.5)),
        (32, 10_000, true, None),
        (20_000, 1, false, None),
    ] {
        let mut messages = vec![Message::text(
            MessageRole::User,
            "original evidence ".repeat(400),
        )];
        let original = messages[0].content_as_text().to_string();
        let mut config = LlmCallConfig::new("gpt-6.2-sol");
        config.reasoning_state = Some(everruns_contracts::reasoning_updates::ReasoningState {
            epoch: "epoch".into(),
            baseline: None,
            effective: None,
            pending: None,
        });
        let emitter = TestEventEmitter::new();
        let session_id = SessionId::new();
        let context = EventContext::empty();
        let result = try_apply_native_compaction(
            &NativeUsageDriver {
                encrypted_bytes,
                billed_output,
                cost,
            },
            policy.as_ref(),
            None,
            session_id,
            None,
            "openai",
            "provider-account",
            &emitter,
            &context,
            "gpt-6.2-sol",
            None,
            false,
            &mut messages,
            &mut config,
        )
        .await
        .unwrap();
        assert_eq!(result.is_some(), reduced);
        let events = emitter.events().await;
        assert_eq!(
            events.len(),
            1,
            "even an ineffective paid result is recorded"
        );
        let crate::engine::events::EventData::LlmGeneration(generation) = &events[0].data else {
            panic!("native accounting must use the existing usage event");
        };
        assert_eq!(
            generation.metadata.provider_id.as_deref(),
            Some("provider-account")
        );
        assert_eq!(
            generation
                .metadata
                .request_options
                .as_ref()
                .unwrap()
                .metadata["purpose"],
            "context_compaction"
        );
        assert!(
            generation.messages.is_empty(),
            "opaque state is not published in usage events"
        );
        let usage = generation.metadata.usage.as_ref().unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens),
            (1000, billed_output)
        );
        assert_eq!(usage.actual_cost_usd, cost);
        assert_eq!(
            usage.effective_cost_usd(),
            cost.or(usage.estimated_cost_usd)
        );
        assert!(
            (usage.estimated_cost_usd.unwrap() - (0.002 + f64::from(billed_output) * 0.00001))
                .abs()
                < 1e-9
        );
        if let Some(result) = result {
            assert_eq!(
                result.tokens_after, None,
                "generated tokens are not compacted context tokens"
            );
            assert_eq!(result.cost_usd, cost);
            assert!((result.estimated_cost_usd.unwrap() - 0.102).abs() < 1e-9);
            assert!(config.provider_opaque_context.is_some());
        } else {
            assert!(config.provider_opaque_context.is_none());
            assert_eq!(messages[0].content_as_text(), original);
        }
    }
}
