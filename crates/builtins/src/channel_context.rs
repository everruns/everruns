//! Channel thread context as leading conversation context.
//!
//! A messaging-channel session (Slack today) accumulates a [`ThreadContext`]:
//! who has spoken in the thread, and where the user is currently looking. Both
//! are written by the channel webhook and persisted under a reserved session KV
//! key; this capability is the read side that puts them in front of the model.
//!
//! Deliberately conversation context, not system prompt. Participant display
//! names and the platform's view report are external user-controlled strings —
//! the same trust class as workspace `AGENTS.md` — so they belong below the
//! harness safety instructions and outside the cache-stable prefix (EVE-977).

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityStatus, SystemPromptContext};
use everruns_core::channel::load_thread_context;

/// Capability id for channel thread context.
pub const CHANNEL_CONTEXT_CAPABILITY_ID: &str = "channel_context";

/// Renders the session's persisted channel `ThreadContext` for the model.
pub struct ChannelContextCapability;

#[async_trait]
impl Capability for ChannelContextCapability {
    fn id(&self) -> &str {
        CHANNEL_CONTEXT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Channel thread context"
    }

    fn description(&self) -> &str {
        "For sessions driven by a messaging channel (Slack), tells the agent who else is in the thread and, when the platform reports it, where the user is currently looking. Accumulates across the thread and survives a restart. Contributes nothing to sessions that are not channel-backed."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("users")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    // No static system_prompt_addition: the content is per-session state.

    async fn conversation_context_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let store = ctx.session_storage.as_ref()?;
        let thread = load_thread_context(store.as_ref(), ctx.session_id).await?;

        // Two independent lines: a thread can have participants and no reported
        // view, or a view and a single participant.
        let mut lines = Vec::new();
        let participants = thread.participants_summary();
        if !participants.is_empty() {
            lines.push(participants);
        }
        let view = thread.view_summary();
        if !view.is_empty() {
            lines.push(view);
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::ExternalActor;
    use everruns_core::channel::{ChannelViewContext, ThreadContext, encode_thread_context};
    use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
    use everruns_provider::error::Result;
    use everruns_provider::typed_id::SessionId;
    use std::sync::Arc;

    /// Serves one record, so the capability is tested against the real codec
    /// rather than a hand-built string.
    struct OneRecordStore(Option<String>);

    #[async_trait]
    impl SessionStorageStore for OneRecordStore {
        async fn set_value(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        async fn get_value(&self, _: SessionId, key: &str) -> Result<Option<String>> {
            assert_eq!(key, everruns_core::channel::THREAD_CONTEXT_KV_KEY);
            Ok(self.0.clone())
        }
        async fn delete_value(&self, _: SessionId, _: &str) -> Result<bool> {
            Ok(false)
        }
        async fn list_keys(&self, _: SessionId) -> Result<Vec<KeyInfo>> {
            Ok(vec![])
        }
        async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        async fn get_secret(&self, _: SessionId, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn delete_secret(&self, _: SessionId, _: &str) -> Result<bool> {
            Ok(false)
        }
        async fn list_secrets(&self, _: SessionId) -> Result<Vec<SecretInfo>> {
            Ok(vec![])
        }
    }

    fn ctx_with(record: Option<String>) -> SystemPromptContext {
        let mut ctx = SystemPromptContext::without_file_store(SessionId::new());
        ctx.session_storage = Some(Arc::new(OneRecordStore(record)));
        ctx
    }

    fn actor(id: &str, name: &str) -> ExternalActor {
        ExternalActor {
            actor_id: id.to_string(),
            actor_name: Some(name.to_string()),
            source: "slack".to_string(),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn renders_participants_and_view() {
        let mut thread = ThreadContext::new("1700.1", "slack");
        thread.track_participant(&actor("U1", "Alice"));
        thread.track_participant(&actor("U2", "Bob"));
        thread.set_current_view(ChannelViewContext {
            channel_id: Some("C123".to_string()),
            ..Default::default()
        });

        let out = ChannelContextCapability
            .conversation_context_contribution(&ctx_with(Some(
                encode_thread_context(&thread).unwrap(),
            )))
            .await
            .expect("context should be contributed");

        assert!(out.contains("Thread participants: Alice, Bob"), "{out}");
        assert!(out.contains("C123"), "{out}");
        assert!(out.contains("have not been given access"), "{out}");
    }

    /// A session that is not channel-backed must contribute nothing at all —
    /// an empty line in every non-Slack turn's context would be pure noise.
    #[tokio::test]
    async fn contributes_nothing_without_a_record() {
        assert!(
            ChannelContextCapability
                .conversation_context_contribution(&ctx_with(None))
                .await
                .is_none()
        );
    }

    /// No store handle (callers that do not supply one) is not an error.
    #[tokio::test]
    async fn contributes_nothing_without_a_store() {
        let ctx = SystemPromptContext::without_file_store(SessionId::new());
        assert!(
            ChannelContextCapability
                .conversation_context_contribution(&ctx)
                .await
                .is_none()
        );
    }

    /// A record with neither participants nor a view yields no line, rather
    /// than an empty string that would render as a blank context block.
    #[tokio::test]
    async fn empty_thread_contributes_nothing() {
        let thread = ThreadContext::new("1700.1", "slack");
        assert!(
            ChannelContextCapability
                .conversation_context_contribution(&ctx_with(Some(
                    encode_thread_context(&thread).unwrap()
                )))
                .await
                .is_none()
        );
    }

    /// A corrupt record degrades to no context instead of failing the turn.
    #[tokio::test]
    async fn malformed_record_contributes_nothing() {
        assert!(
            ChannelContextCapability
                .conversation_context_contribution(&ctx_with(Some("not json".to_string())))
                .await
                .is_none()
        );
    }
}
