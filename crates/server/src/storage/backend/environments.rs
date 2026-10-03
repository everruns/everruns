//! Logical execution-environment storage across hosted and in-memory backends.

use super::*;
use crate::storage::{EnvironmentRecord, PgSandboxCheckpointStore};
use everruns_contracts::typed_id::SessionId;
use everruns_platform::ResolvedEnvironmentProfile;

impl StorageBackend {
    /// Pin the resolved profile exactly once for the lifetime of a Session.
    pub async fn pin_environment(
        &self,
        session_id: SessionId,
        profile_name: &str,
        profile: &ResolvedEnvironmentProfile,
    ) -> Result<EnvironmentRecord> {
        match self {
            Self::Postgres(db) => PgSandboxCheckpointStore::new(db.pool().clone())
                .pin_environment(session_id, profile_name, profile)
                .await
                .map_err(Into::into),
            Self::InMemory(db) => {
                if !db.sessions.read().contains_key(&session_id) {
                    anyhow::bail!("session not found while pinning environment");
                }
                let provider = profile
                    .target
                    .provider
                    .clone()
                    .unwrap_or_else(|| profile.target.kind.as_str().to_string());
                let mut environments = db.environments.write();
                if let Some(existing) = environments.get(&session_id) {
                    if existing.profile_name == profile_name
                        && existing.profile == *profile
                        && existing.provider == provider
                    {
                        return Ok(existing.clone());
                    }
                    anyhow::bail!("session environment is already pinned to a different profile");
                }
                let record = EnvironmentRecord {
                    id: uuid::Uuid::now_v7(),
                    session_id,
                    provider,
                    profile_name: profile_name.to_string(),
                    profile: profile.clone(),
                    desired_state: "ready".to_string(),
                    observed_state: "absent".to_string(),
                    generation: 1,
                    current_checkpoint_id: None,
                    last_activity_at: None,
                };
                environments.insert(session_id, record.clone());
                Ok(record)
            }
        }
    }

    /// Load the Session-owned logical Environment, if the Session selected one.
    pub async fn get_environment(
        &self,
        session_id: SessionId,
    ) -> Result<Option<EnvironmentRecord>> {
        match self {
            Self::Postgres(db) => PgSandboxCheckpointStore::new(db.pool().clone())
                .get_environment(session_id)
                .await
                .map_err(Into::into),
            Self::InMemory(db) => Ok(db.environments.read().get(&session_id).cloned()),
        }
    }
}
