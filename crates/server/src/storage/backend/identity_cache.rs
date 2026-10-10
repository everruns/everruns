// Fields that never change once a row exists, cached per backend.
//
// Decision: every turn, several event listeners and worker reads load a whole
// session row only for its org or parent session, and resolve an agent's
// public id again. Those fields are set at insert and never updated, so
// caching them needs no invalidation and stays correct across replicas. Only
// found rows are cached; the TTL just bounds memory and keeps deleted rows
// from lingering.

use std::time::Duration;

use anyhow::Result;
use everruns_contracts::typed_id::{AgentId, SessionId};
use moka::future::Cache;

use super::StorageBackend;

const CAPACITY: u64 = 10_000;
const TTL: Duration = Duration::from_secs(600);

/// A session's org and parent session, neither of which ever changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLineage {
    pub org_id: i64,
    pub parent_session_id: Option<SessionId>,
}

#[derive(Clone)]
pub(super) struct IdentityCache {
    sessions: Cache<SessionId, SessionLineage>,
    agent_public_ids: Cache<(i64, AgentId), String>,
}

impl IdentityCache {
    pub(super) fn new() -> Self {
        Self {
            sessions: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(TTL)
                .build(),
            agent_public_ids: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(TTL)
                .build(),
        }
    }
}

impl StorageBackend {
    /// The session's org and parent, read once and then served from memory.
    pub async fn session_lineage(&self, id: SessionId) -> Result<Option<SessionLineage>> {
        if let Some(lineage) = self.identity.sessions.get(&id).await {
            return Ok(Some(lineage));
        }
        let Some(row) = self.get_session_unscoped(id).await? else {
            return Ok(None);
        };
        let lineage = SessionLineage {
            org_id: row.org_id,
            parent_session_id: row.parent_session_id,
        };
        self.identity.sessions.insert(id, lineage).await;
        Ok(Some(lineage))
    }

    pub(super) async fn cached_agent_public_id(&self, org_id: i64, id: AgentId) -> Option<String> {
        self.identity.agent_public_ids.get(&(org_id, id)).await
    }

    pub(super) async fn remember_agent_public_id(&self, org_id: i64, id: AgentId, public_id: &str) {
        self.identity
            .agent_public_ids
            .insert((org_id, id), public_id.to_string())
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;

    #[tokio::test]
    async fn missing_session_is_not_cached() {
        let db = StorageBackend::test_database();
        let id = SessionId::new();
        assert_eq!(db.session_lineage(id).await.unwrap(), None);
        assert!(db.identity.sessions.get(&id).await.is_none());
    }
}
