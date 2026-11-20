use super::*;
use crate::capabilities::Capability;
use everruns_contracts::compact::{CompactOutputItem, CompactResponse, CompactUsage};
use everruns_contracts::driver_registry::{ChatDriver, LlmCallConfig, LlmResponseStream};

struct NativeUsageDriver {
    encrypted_bytes: usize,
    billed_output: u32,
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
                cost: Some(0.5),
            }),
        }))
    }
}

#[tokio::test]
async fn explicit_generation_usage_cannot_prove_or_disprove_window_reduction() {
    let policy = crate::builtins::CompactionCapability
        .compaction_policy(&serde_json::json!({"strategy":"native"}))
        .unwrap();
    for (encrypted_bytes, billed_output, reduced) in [(32, 10_000, true), (20_000, 1, false)] {
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
        let result = try_apply_native_compaction(
            &NativeUsageDriver {
                encrypted_bytes,
                billed_output,
            },
            policy.as_ref(),
            None,
            SessionId::new(),
            None,
            "openai",
            "gpt-6.2-sol",
            None,
            false,
            &mut messages,
            &mut config,
        )
        .await
        .unwrap();
        assert_eq!(result.is_some(), reduced);
        if let Some(result) = result {
            assert_eq!(
                result.tokens_after, None,
                "generated tokens are not compacted context tokens"
            );
            assert_eq!(result.cost_usd, Some(0.5));
            assert!(config.provider_opaque_context.is_some());
        } else {
            assert!(config.provider_opaque_context.is_none());
            assert_eq!(messages[0].content_as_text(), original);
        }
    }
}
