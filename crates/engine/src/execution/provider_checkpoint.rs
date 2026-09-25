use crate::{
    ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION, COMPACTION_CHECKPOINT_FORMAT_VERSION,
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore,
    ProviderOpaqueContext,
};
use everruns_provider::driver_registry::ProviderCheckpointCandidate;
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct ProviderCheckpointInstall {
    pub(super) checkpoint_id: Option<String>,
    pub(super) checkpoint_bytes: Option<u64>,
}

pub(super) fn format_version(provider_managed_reduction: bool) -> u32 {
    if provider_managed_reduction {
        ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION
    } else {
        COMPACTION_CHECKPOINT_FORMAT_VERSION
    }
}

pub(super) fn is_restorable(
    checkpoint: &CompactionCheckpoint,
    driver: &dyn crate::ChatDriver,
    provider_type: &str,
    model: &str,
    provider_managed_reduction: bool,
    native_reasoning_compaction: bool,
) -> bool {
    if !checkpoint.is_compatible_format(
        provider_type,
        model,
        format_version(provider_managed_reduction),
    ) {
        return false;
    }

    match (&checkpoint.payload, provider_managed_reduction) {
        (CompactionCheckpointPayload::ProviderOpaque { context }, true)
            if matches!(
                context,
                ProviderOpaqueContext::AnthropicMessagesPrefix { .. }
            ) =>
        {
            if driver.validate_provider_opaque_context(context) {
                true
            } else {
                tracing::warn!(
                    checkpoint_id = %checkpoint.id,
                    provider = provider_type,
                    model,
                    "ReasonAtom: ignored invalid provider-managed compaction checkpoint"
                );
                false
            }
        }
        (
            CompactionCheckpointPayload::ProviderOpaque {
                context:
                    ProviderOpaqueContext::OpenResponsesCompact {
                        reasoning_state, ..
                    },
            },
            false,
        ) => reasoning_state.is_none() || native_reasoning_compaction,
        (CompactionCheckpointPayload::Summary { .. }, false) => true,
        _ => false,
    }
}

pub(super) fn reasoning_state(
    checkpoint: Option<&CompactionCheckpoint>,
) -> Option<&everruns_provider::reasoning_updates::ReasoningState> {
    let CompactionCheckpointPayload::ProviderOpaque {
        context:
            ProviderOpaqueContext::OpenResponsesCompact {
                reasoning_state, ..
            },
    } = &checkpoint?.payload
    else {
        return None;
    };
    reasoning_state.as_ref()
}
pub(super) async fn install_candidate(
    store: Option<&dyn CompactionCheckpointStore>,
    source_sequence: Option<i32>,
    candidate: Option<ProviderCheckpointCandidate>,
    session_id: SessionId,
    provider_type: &str,
    model: &str,
) -> ProviderCheckpointInstall {
    let Some(candidate) = candidate else {
        return ProviderCheckpointInstall::default();
    };
    let checkpoint_bytes = serde_json::to_vec(&candidate.context)
        .ok()
        .map(|bytes| bytes.len() as u64);
    let (Some(store), Some(source_sequence)) = (store, source_sequence) else {
        tracing::warn!(
            session_id = %session_id,
            "ReasonAtom: completed output had no durable checkpoint destination"
        );
        return ProviderCheckpointInstall {
            checkpoint_id: None,
            checkpoint_bytes,
        };
    };
    let checkpoint_id = Uuid::now_v7();
    let checkpoint = CompactionCheckpoint {
        id: checkpoint_id,
        session_id,
        source_sequence: i64::from(source_sequence),
        provider_type: provider_type.to_string(),
        model: model.to_string(),
        format_version: candidate.format_version,
        payload: CompactionCheckpointPayload::ProviderOpaque {
            context: candidate.context,
        },
    };
    let installed = match store.install(checkpoint).await {
        Ok(true) => {
            tracing::info!(
                session_id = %session_id,
                source_sequence,
                "ReasonAtom: installed provider-managed compaction checkpoint"
            );
            true
        }
        Ok(false) => {
            tracing::warn!(
                session_id = %session_id,
                source_sequence,
                "ReasonAtom: a newer provider-managed checkpoint is already installed"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                source_sequence,
                error = %error,
                "ReasonAtom: failed to install provider-managed checkpoint; preserving completed output"
            );
            false
        }
    };
    ProviderCheckpointInstall {
        checkpoint_id: installed.then(|| checkpoint_id.to_string()),
        checkpoint_bytes,
    }
}
