//! Read-only checkpoint inspection for Framework context views.
use super::*;

impl InProcessRuntime {
    /// Describe the effective policy and saved checkpoint for an assembled model.
    /// This reads metadata only and never pays for a summary or changes history.
    pub async fn context_compaction(
        &self,
        context: &AssembledTurnContext,
    ) -> Result<crate::compaction_status::CompactionStatus> {
        let mut status = crate::compaction_status::CompactionStatus::from_configs(
            &context.resolved_capability_configs,
            &self.host_composition.capability_registry(),
        );
        status
            .inspect_checkpoint(
                Some(self.compaction_checkpoint_store.as_ref()),
                context.snapshot.session_id,
                Some((context.model.provider_type.as_str(), &context.model.model)),
            )
            .await?;
        Ok(status)
    }
}
