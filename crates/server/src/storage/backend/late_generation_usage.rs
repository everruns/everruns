use anyhow::Result;
use uuid::Uuid;

use super::StorageBackend;
use crate::storage::{
    CreatePendingUsageGeneration, GenerationUsageSnapshot, LateGenerationUsage,
    PendingUsageGeneration,
};

impl StorageBackend {
    pub async fn create_pending_usage_generation(
        &self,
        input: CreatePendingUsageGeneration,
    ) -> Result<Uuid> {
        dispatch!(self, create_pending_usage_generation, input)
    }

    pub async fn list_pending_usage_generations(
        &self,
        max_attempts: i32,
        limit: i64,
    ) -> Result<Vec<PendingUsageGeneration>> {
        dispatch!(self, list_pending_usage_generations, max_attempts, limit)
    }

    /// Writes late usage and clears `usage_pending` in one statement. Returns
    /// whether this call applied it: `false` means the usage was already
    /// applied, so the caller must not meter it again.
    pub async fn apply_late_generation_usage(
        &self,
        id: Uuid,
        usage: &LateGenerationUsage,
    ) -> Result<bool> {
        dispatch!(self, apply_late_generation_usage, id, usage)
    }

    pub async fn get_generation_usage(&self, id: Uuid) -> Result<Option<GenerationUsageSnapshot>> {
        dispatch!(self, get_generation_usage, id)
    }
}
