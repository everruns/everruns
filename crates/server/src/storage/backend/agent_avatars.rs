use anyhow::Result;
use uuid::Uuid;

use super::StorageBackend;
use crate::storage::{AgentAvatarVariantRow, SetAgentAvatar};

impl StorageBackend {
    pub async fn set_agent_avatar(&self, input: SetAgentAvatar) -> Result<Option<Uuid>> {
        dispatch!(self, set_agent_avatar, input)
    }

    pub async fn clear_agent_avatar(&self, org_id: i64, agent_id: Uuid) -> Result<bool> {
        dispatch!(self, clear_agent_avatar, org_id, agent_id)
    }

    pub async fn get_agent_avatar_variant(
        &self,
        avatar_id: Uuid,
        variant: &str,
    ) -> Result<Option<AgentAvatarVariantRow>> {
        dispatch!(self, get_agent_avatar_variant, avatar_id, variant)
    }
}
