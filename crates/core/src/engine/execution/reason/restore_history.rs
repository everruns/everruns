//! The history a reason phase sends: the suffix after its compaction
//! checkpoint when one restores, or the whole history when assembly loaded
//! only a suffix ([`crate::HistoryFloor`]) and no checkpoint restores.

use std::sync::Arc;

use crate::AgentCapabilityConfig;
use crate::engine::capabilities::{CapabilityRegistry, collect_message_filters_only_with_context};
use crate::engine::driver_registry::ChatDriver;
use crate::engine::error::Result;
use crate::engine::message::RuntimeMessage;
use crate::engine::message_retriever::MessageRetriever;
use crate::engine::typed_id::SessionId;
use crate::engine::{
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore, MessageQuery,
};

pub(super) struct Request<'a> {
    pub(super) session_id: SessionId,
    pub(super) store: Option<&'a Arc<dyn CompactionCheckpointStore>>,
    pub(super) retriever: &'a dyn MessageRetriever,
    pub(super) registry: &'a CapabilityRegistry,
    pub(super) capability_configs: &'a [AgentCapabilityConfig],
    pub(super) driver: &'a dyn ChatDriver,
    pub(super) provider_type: &'a str,
    pub(super) model: &'a str,
    /// Compaction is configured, or the provider reduces history itself.
    pub(super) compaction: bool,
    pub(super) provider_managed: bool,
    pub(super) native_reasoning_compaction: bool,
    /// Assembly loaded only the history after this sequence.
    pub(super) message_floor: Option<i64>,
}

pub(super) struct Restored {
    pub(super) messages: Vec<RuntimeMessage>,
    pub(super) source_sequence: Option<i64>,
    pub(super) checkpoint: Option<CompactionCheckpoint>,
    /// Messages after the checkpoint, before post-load filters.
    pub(super) suffix_message_count: usize,
    /// Persisted sequence of each restored message, when known.
    pub(super) sequences: std::collections::HashMap<crate::engine::typed_id::MessageId, i64>,
}

/// The history to send instead of the assembled one, if any.
pub(super) async fn restore(request: Request<'_>) -> Result<Option<Restored>> {
    let filters = || {
        collect_message_filters_only_with_context(
            request.capability_configs,
            request.registry,
            request.provider_managed,
        )
    };
    let format = super::super::provider_checkpoint::format_version(request.provider_managed);
    if request.compaction
        && let Some(store) = request.store
        && let Some(checkpoint) = store
            .get_latest_format(
                request.session_id,
                request.provider_type,
                request.model,
                format,
            )
            .await?
        && super::super::provider_checkpoint::is_restorable(
            &checkpoint,
            request.driver,
            request.provider_type,
            request.model,
            request.provider_managed,
            request.native_reasoning_compaction,
        )
    {
        let filters = filters();
        let mut query =
            MessageQuery::new(request.session_id).after_sequence(checkpoint.source_sequence);
        filters.apply_message_filters(&mut query);
        let history = request.retriever.load_filtered_history(query).await?;
        let mut messages = history.messages;
        let suffix_message_count = messages.len();
        filters.apply_post_load_filters(&mut messages);
        if let CompactionCheckpointPayload::Summary { text } = &checkpoint.payload {
            messages.insert(
                0,
                RuntimeMessage::system(format!(
                    "[CONVERSATION_SUMMARY]\n{text}\n[/CONVERSATION_SUMMARY]"
                )),
            );
        }
        return Ok(Some(Restored {
            messages,
            source_sequence: history.source_sequence,
            checkpoint: Some(checkpoint),
            suffix_message_count,
            sequences: history.sequences,
        }));
    }
    if request.message_floor.is_none() {
        return Ok(None);
    }
    // No checkpoint for this model (another model's was the floor): the
    // model needs the whole history.
    let filters = filters();
    let mut query = MessageQuery::new(request.session_id);
    filters.apply_message_filters(&mut query);
    let mut history = request.retriever.load_filtered_history(query).await?;
    filters.apply_post_load_filters(&mut history.messages);
    Ok(Some(Restored {
        messages: super::transcript::order_native_results(history.messages),
        source_sequence: history.source_sequence,
        checkpoint: None,
        suffix_message_count: 0,
        sequences: history.sequences,
    }))
}
