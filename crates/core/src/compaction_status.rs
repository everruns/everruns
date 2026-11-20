//! Read-only compaction policy and checkpoint metadata shared by hosts.
//! Inspection never invokes a model or exposes checkpoint contents.

use crate::compaction_checkpoint::{
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore,
    PORTABLE_COMPACTION_FORMAT_VERSION, PORTABLE_COMPACTION_MODEL, PORTABLE_COMPACTION_PROVIDER,
};
use crate::compaction_policy::CompactionPolicy;
use crate::typed_id::SessionId;
use crate::{AgentCapabilityConfig, CapabilityRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Effective configured policy and the last compatible saved checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompactionStatus {
    /// Whether a compaction policy is configured after inheritance and dependencies.
    pub enabled: bool,
    /// Configured strategy; runtime/provider eligibility still controls execution.
    pub strategy: Option<String>,
    /// Whether proactive reduction is enabled.
    pub proactive: bool,
    /// Soft working-history target, distinct from the provider hard window.
    pub context_budget_tokens: Option<usize>,
    /// Native request pressure threshold as a fraction of the model window.
    pub budget_percent: Option<f32>,
    /// Whether raw history retrieval is allowed by the configured prompt hooks.
    pub history_retrieval_available: bool,
    /// Store availability, not a promise about the backend's crash durability.
    /// None when a remote host has not reported its store.
    pub checkpoint_store_available: Option<bool>,
    /// Saved checkpoint metadata only; never summary text or provider wire state.
    pub checkpoint: Option<ContextCheckpoint>,
    /// Latest reported lifecycle outcome, when a host provides event observations.
    pub last_event: Option<CompactionEventSummary>,
}

/// Content-free lifecycle information suitable for chat and API inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompactionEventSummary {
    /// compacting, completed, skipped, or failed.
    pub state: String,
    /// Strategy recorded by the runtime, not a forecast of the next provider call.
    pub strategy: String,
    /// Whether this observation installed a checkpoint rather than only masking a request.
    pub checkpoint_id: Option<String>,
    /// Replaced journal boundary when available.
    pub source_sequence: Option<i64>,
    /// Input token estimate or native provider measurement, when available.
    pub tokens_before: Option<u64>,
    /// Output token estimate or native provider measurement, when available.
    pub tokens_after: Option<u64>,
    /// Recorded elapsed reduction time.
    pub duration_ms: Option<u64>,
}

impl CompactionEventSummary {
    /// Project safe counters only; provider payloads and error internals are excluded.
    pub fn from_event(event: &crate::events::EventData) -> Option<Self> {
        use crate::events::EventData::*;
        let (
            state,
            strategy,
            checkpoint_id,
            source_sequence,
            tokens_before,
            tokens_after,
            duration_ms,
        ) = match event {
            ContextCompacting(d) => (
                "compacting",
                d.strategy.clone(),
                None,
                d.source_sequence,
                d.tokens_before,
                None,
                None,
            ),
            ContextCompacted(d) => (
                "completed",
                d.strategy_used.clone(),
                d.checkpoint_id.clone(),
                d.source_sequence,
                d.tokens_before,
                d.tokens_after,
                Some(d.duration_ms),
            ),
            ContextCompactionSkipped(d) => (
                "skipped",
                d.strategy.clone(),
                None,
                d.source_sequence,
                Some(d.tokens_observed),
                None,
                None,
            ),
            ContextCompactionFailed(d) => (
                "failed",
                d.strategy.clone(),
                None,
                d.source_sequence,
                Some(d.tokens_before),
                None,
                None,
            ),
            _ => return None,
        };
        Some(Self {
            state: state.into(),
            strategy,
            checkpoint_id,
            source_sequence,
            tokens_before,
            tokens_after,
            duration_ms,
        })
    }
}

/// Credential-free metadata for an installed checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextCheckpoint {
    /// Public-safe identifier for correlating the checkpoint with trace events.
    pub id: String,
    /// portable_summary, provider_native, legacy_summary, or legacy_trim.
    pub kind: String,
    /// Exact journal boundary replaced by this checkpoint.
    pub source_sequence: i64,
    /// Provider scope for native checkpoints; portable notes have no provider scope.
    pub provider: Option<String>,
    /// Model scope for native checkpoints; portable notes have no model scope.
    pub model: Option<String>,
}

pub(crate) fn resolve_policy(
    configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> Option<Arc<dyn CompactionPolicy>> {
    configs.iter().find_map(|config| {
        let capability = registry.get(config.capability_id())?;
        let policy = capability.compaction_policy(config.config_value())?;
        let legacy_budget = configs
            .iter()
            .find(|c| c.capability_id() == "infinity_context")
            .and_then(|c| {
                c.config_value()
                    .get("context_budget_tokens")
                    .and_then(serde_json::Value::as_u64)
            });
        if config.capability_id() == "compaction"
            && let (Some(legacy), Some(working)) = (legacy_budget, policy.working_context_tokens())
        {
            let mut normalized = config.config_value().clone();
            if normalized.is_null() {
                normalized = serde_json::json!({});
            }
            normalized["context_budget_tokens"] = serde_json::json!(working.min(legacy as usize));
            capability.compaction_policy(&normalized)
        } else {
            Some(policy)
        }
    })
}

impl CompactionStatus {
    /// Describe resolved configuration without reading history or executing tools.
    pub fn from_configs(configs: &[AgentCapabilityConfig], registry: &CapabilityRegistry) -> Self {
        let policy = resolve_policy(configs, registry);
        let settings = policy.as_ref().map(|policy| policy.settings());
        let history_retrieval_available = configs
            .iter()
            .any(|c| c.capability_id() == "infinity_context")
            && !configs.iter().any(|config| {
                registry.get(config.capability_id()).is_some_and(|cap| {
                    cap.user_hooks_with_config(config.config_value())
                        .iter()
                        .any(|hook| {
                            hook.event == crate::user_hook_types::HookEvent::UserPromptSubmit
                        })
                })
            });
        Self {
            enabled: policy.is_some(),
            strategy: settings.as_ref().map(|s| s.strategy.to_string()),
            proactive: policy.as_ref().is_some_and(|p| p.proactive_enabled()),
            context_budget_tokens: policy.as_ref().and_then(|p| p.working_context_tokens()),
            budget_percent: settings.map(|s| s.budget_percent),
            history_retrieval_available,
            checkpoint_store_available: None,
            checkpoint: None,
            last_event: None,
        }
    }

    /// Read the portable checkpoint and native records for the supplied model.
    /// `model` must identify the model whose scope the caller is inspecting;
    /// absence leaves native compatibility unknown rather than guessing it.
    pub async fn inspect_checkpoint(
        &mut self,
        store: Option<&dyn CompactionCheckpointStore>,
        session: SessionId,
        model: Option<(&str, &str)>,
    ) -> crate::error::Result<()> {
        self.checkpoint_store_available = Some(store.is_some());
        self.checkpoint = None;
        let Some(store) = store else {
            return Ok(());
        };
        let mut selected = store
            .get_latest_format(
                session,
                PORTABLE_COMPACTION_PROVIDER,
                PORTABLE_COMPACTION_MODEL,
                PORTABLE_COMPACTION_FORMAT_VERSION,
            )
            .await?
            .filter(CompactionCheckpoint::is_portable);
        if let Some((provider, model)) = model {
            for format in [1, 2] {
                if let Some(native) = store
                    .get_latest_format(session, provider, model, format)
                    .await?
                    && selected
                        .as_ref()
                        .is_none_or(|old| native.source_sequence > old.source_sequence)
                    && !matches!(native.payload, CompactionCheckpointPayload::Trimmed)
                {
                    selected = Some(native);
                }
            }
        }
        self.checkpoint = selected.as_ref().map(ContextCheckpoint::from_checkpoint);
        Ok(())
    }
}

impl ContextCheckpoint {
    fn from_checkpoint(checkpoint: &CompactionCheckpoint) -> Self {
        let portable = checkpoint.is_portable();
        Self {
            id: checkpoint.id.to_string(),
            kind: match &checkpoint.payload {
                CompactionCheckpointPayload::Portable { .. } => "portable_summary",
                CompactionCheckpointPayload::ProviderOpaque { .. } => "provider_native",
                CompactionCheckpointPayload::Summary { .. } => "legacy_summary",
                CompactionCheckpointPayload::Trimmed => "legacy_trim",
            }
            .into(),
            source_sequence: checkpoint.source_sequence,
            provider: (!portable).then(|| checkpoint.provider_type.clone()),
            model: (!portable).then(|| checkpoint.model.clone()),
        }
    }
}

#[cfg(all(test, feature = "builtins"))]
mod tests {
    use super::*;
    use crate::compaction_checkpoint::{InMemoryCompactionCheckpointStore, PortableContinuity};

    fn portable(session_id: SessionId, source_sequence: i64) -> CompactionCheckpoint {
        CompactionCheckpoint {
            id: uuid::Uuid::new_v4(),
            session_id,
            source_sequence,
            provider_type: PORTABLE_COMPACTION_PROVIDER.into(),
            model: PORTABLE_COMPACTION_MODEL.into(),
            format_version: PORTABLE_COMPACTION_FORMAT_VERSION,
            payload: CompactionCheckpointPayload::Portable {
                continuity: PortableContinuity {
                    text: "private continuity text".into(),
                    pinned: vec!["private pin".into()],
                    producer_provider: "private producer".into(),
                    producer_model: "private model".into(),
                    source_fingerprint: [0; 32],
                    prompt_version: 1,
                    usage: serde_json::json!({}),
                },
            },
        }
    }

    #[tokio::test]
    async fn inspection_is_content_free_session_and_model_scoped_and_read_only() {
        let store = InMemoryCompactionCheckpointStore::default();
        let session = SessionId::new();
        let portable = portable(session, 100);
        store.install(portable.clone()).await.unwrap();
        let native = CompactionCheckpoint {
            id: uuid::Uuid::new_v4(),
            session_id: session,
            source_sequence: 200,
            provider_type: "openai".into(),
            model: "model-a".into(),
            format_version: 1,
            payload: CompactionCheckpointPayload::Summary {
                text: "private native summary".into(),
            },
        };
        store.install(native.clone()).await.unwrap();
        store
            .install(CompactionCheckpoint {
                session_id: SessionId::new(),
                source_sequence: 900,
                ..portable.clone()
            })
            .await
            .unwrap();
        let mut status = CompactionStatus::from_configs(&[], &CapabilityRegistry::new());
        status
            .inspect_checkpoint(Some(&store), session, Some(("openai", "model-b")))
            .await
            .unwrap();
        assert_eq!(
            status.checkpoint.as_ref().unwrap().id,
            portable.id.to_string()
        );
        status
            .inspect_checkpoint(Some(&store), session, Some(("openai", "model-a")))
            .await
            .unwrap();
        assert_eq!(
            status.checkpoint.as_ref().unwrap().id,
            native.id.to_string()
        );
        let json = serde_json::to_string(&status).unwrap();
        for secret in [
            "private continuity text",
            "private pin",
            "private producer",
            "private native summary",
        ] {
            assert!(!json.contains(secret));
        }
        assert_eq!(
            store
                .get_latest_format(session, "openai", "model-a", 1)
                .await
                .unwrap(),
            Some(native)
        );
        assert_eq!(
            store
                .get_latest_format(
                    session,
                    PORTABLE_COMPACTION_PROVIDER,
                    PORTABLE_COMPACTION_MODEL,
                    PORTABLE_COMPACTION_FORMAT_VERSION
                )
                .await
                .unwrap(),
            Some(portable)
        );
    }

    #[tokio::test]
    async fn inspection_does_not_treat_trim_or_invalid_portable_as_continuity() {
        let store = InMemoryCompactionCheckpointStore::default();
        let session = SessionId::new();
        let mut invalid = portable(session, 100);
        if let CompactionCheckpointPayload::Portable { continuity } = &mut invalid.payload {
            continuity.text.clear();
        }
        store.install(invalid).await.unwrap();
        store
            .install(CompactionCheckpoint {
                id: uuid::Uuid::new_v4(),
                session_id: session,
                source_sequence: 200,
                provider_type: "openai".into(),
                model: "model-a".into(),
                format_version: 1,
                payload: CompactionCheckpointPayload::Trimmed,
            })
            .await
            .unwrap();
        let mut status = CompactionStatus::from_configs(&[], &CapabilityRegistry::new());
        status
            .inspect_checkpoint(Some(&store), session, Some(("openai", "model-a")))
            .await
            .unwrap();
        assert!(status.checkpoint.is_none());
        assert_eq!(status.checkpoint_store_available, Some(true));
        status
            .inspect_checkpoint(None, session, None)
            .await
            .unwrap();
        assert_eq!(status.checkpoint_store_available, Some(false));
    }

    #[test]
    fn inspection_uses_the_same_legacy_budget_normalization_as_execution() {
        let mut registry = CapabilityRegistry::new();
        registry.register(crate::builtins::CompactionCapability);
        registry.register(crate::builtins::InfinityContextCapability);
        let configs = vec![
            AgentCapabilityConfig::new("compaction"),
            AgentCapabilityConfig::with_config(
                "infinity_context",
                serde_json::json!({"context_budget_tokens":8192}),
            ),
        ];
        let status = CompactionStatus::from_configs(&configs, &registry);
        assert!(status.enabled && status.proactive && status.history_retrieval_available);
        assert_eq!(status.context_budget_tokens, Some(8192));
        assert_eq!(status.strategy.as_deref(), Some("auto"));
    }
}
