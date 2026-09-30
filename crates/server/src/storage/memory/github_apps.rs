// In-memory storage: per-agent GitHub Apps

use super::super::github_app_rows::*;
use super::InMemoryDatabase;
use crate::kernel_imports::everruns_provider::typed_id::{AgentIdentityId, SessionId};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_github_app(&self, input: CreateGitHubAppRow) -> Result<GitHubAppRow> {
        let mut apps = self.github_apps.write();
        if apps.values().any(|app| {
            app.agent_identity_id == input.agent_identity_id || app.app_id == input.app_id
        }) {
            anyhow::bail!("duplicate key value violates unique constraint on github_apps");
        }
        let row = GitHubAppRow {
            id: input.id,
            org_id: input.org_id,
            agent_identity_id: input.agent_identity_id,
            app_id: input.app_id,
            slug: input.slug,
            name: input.name,
            html_url: input.html_url,
            owner_login: input.owner_login,
            client_id: input.client_id,
            client_secret_encrypted: input.client_secret_encrypted,
            private_key_encrypted: input.private_key_encrypted,
            webhook_secret_encrypted: input.webhook_secret_encrypted,
            created_by_user_id: input.created_by_user_id,
            created_at: Self::now(),
            updated_at: Self::now(),
        };
        apps.insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_github_app_unscoped(&self, id: Uuid) -> Result<Option<GitHubAppRow>> {
        Ok(self.github_apps.read().get(&id).cloned())
    }

    pub async fn get_github_app_for_identity(
        &self,
        org_id: i64,
        agent_identity_id: AgentIdentityId,
    ) -> Result<Option<GitHubAppRow>> {
        Ok(self
            .github_apps
            .read()
            .values()
            .find(|app| app.org_id == org_id && app.agent_identity_id == agent_identity_id)
            .cloned())
    }

    pub async fn get_identity_github_app_for_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<(GitHubAppRow, i64)>> {
        let Some(identity_id) = self
            .sessions
            .read()
            .get(&session_id)
            .and_then(|session| session.agent_identity_id)
        else {
            return Ok(None);
        };
        let Some(installation_id) = self
            .agent_identity_connections
            .read()
            .values()
            .find(|conn| conn.agent_identity_id == identity_id && conn.provider == "github")
            .and_then(|conn| conn.installation_id)
        else {
            return Ok(None);
        };
        Ok(self
            .github_apps
            .read()
            .values()
            .find(|app| app.agent_identity_id == identity_id)
            .cloned()
            .map(|app| (app, installation_id)))
    }

    pub async fn delete_github_app(&self, id: Uuid) -> Result<bool> {
        Ok(self.github_apps.write().remove(&id).is_some())
    }
}
