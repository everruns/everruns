use std::collections::HashMap;

use anyhow::Result;
use everruns_contracts::typed_id::AgentId;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::{AgentAvatarVariantRow, SetAgentAvatar};

#[derive(Clone, Debug)]
pub(super) struct StoredAvatar {
    agent_id: Uuid,
    variants: HashMap<String, AgentAvatarVariantRow>,
}

impl InMemoryDatabase {
    pub async fn set_agent_avatar(&self, input: SetAgentAvatar) -> Result<Option<Uuid>> {
        let mut agents = self.agents.write();
        let Some(agent) = agents
            .get_mut(&AgentId::from_uuid(input.agent_id))
            .filter(|agent| agent.org_id == input.org_id && agent.status != "deleted")
        else {
            return Ok(None);
        };
        let avatar_id = Uuid::now_v7();
        let variants = input
            .variants
            .into_iter()
            .map(|v| {
                (
                    v.variant,
                    AgentAvatarVariantRow {
                        content_type: v.content_type,
                        data: v.data,
                    },
                )
            })
            .collect();
        let mut avatars = self.agent_avatars.write();
        avatars.retain(|_, stored| stored.agent_id != input.agent_id);
        avatars.insert(
            avatar_id,
            StoredAvatar {
                agent_id: input.agent_id,
                variants,
            },
        );
        agent.avatar_id = Some(avatar_id);
        agent.updated_at = Self::now();
        Ok(Some(avatar_id))
    }

    pub async fn clear_agent_avatar(&self, org_id: i64, agent_id: Uuid) -> Result<bool> {
        let mut agents = self.agents.write();
        let Some(agent) = agents
            .get_mut(&AgentId::from_uuid(agent_id))
            .filter(|agent| agent.org_id == org_id)
        else {
            return Ok(false);
        };
        self.agent_avatars
            .write()
            .retain(|_, stored| stored.agent_id != agent_id);
        let had = agent.avatar_id.take().is_some();
        if had {
            agent.updated_at = Self::now();
        }
        Ok(had)
    }

    pub async fn get_agent_avatar_variant(
        &self,
        avatar_id: Uuid,
        variant: &str,
    ) -> Result<Option<AgentAvatarVariantRow>> {
        Ok(self
            .agent_avatars
            .read()
            .get(&avatar_id)
            .and_then(|stored| stored.variants.get(variant).cloned()))
    }
}
