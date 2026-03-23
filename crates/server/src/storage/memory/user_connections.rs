// In-memory storage: User Connections

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::SessionId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // User Connections
    // ============================================

    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();

        // Remove existing connection for same user+provider (app-level uniqueness)
        let mut connections = self.user_connections.write();
        connections.retain(|_, c| !(c.user_id == input.user_id && c.provider == input.provider));

        let row = UserConnectionRow {
            id,
            user_id: input.user_id,
            provider: input.provider,
            connection_type: input.connection_type,
            provider_user_id: input.provider_user_id,
            provider_username: input.provider_username,
            access_token_encrypted: input.access_token_encrypted,
            refresh_token_encrypted: input.refresh_token_encrypted,
            scopes: input.scopes,
            expires_at: input.expires_at,
            installation_id: input.installation_id,
            provider_metadata: input.provider_metadata,
            created_at: now,
            updated_at: now,
        };
        connections.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| c.user_id == user_id && c.provider == provider)
            .cloned())
    }

    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        let mut connections: Vec<_> = self
            .user_connections
            .read()
            .values()
            .filter(|c| c.user_id == user_id)
            .cloned()
            .collect();
        connections.sort_by(|a, b| a.provider.cmp(&b.provider));
        Ok(connections)
    }

    /// Get encrypted connection token for a session.
    ///
    /// If the session has an `agent_identity_id`, checks `agent_identity_connections`
    /// first; falls back to `user_connections` via org membership.
    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        let agent_identity_id = session.agent_identity_id;
        drop(sessions);

        // Check identity connections first — if any exist for this provider,
        // they take full precedence (even if the specific credential field is absent).
        if let Some(identity_id) = agent_identity_id {
            let id_connections = self.agent_identity_connections.read();
            let has_any = id_connections
                .values()
                .any(|c| c.agent_identity_id == identity_id && c.provider == provider);
            if has_any {
                // Return the token if present, None otherwise (caller uses installation_id path)
                return Ok(id_connections
                    .values()
                    .find(|c| {
                        c.agent_identity_id == identity_id
                            && c.provider == provider
                            && c.access_token_encrypted.is_some()
                    })
                    .and_then(|c| c.access_token_encrypted.clone()));
            }
        }

        // Fall back to user connections via org membership
        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            for conn in connections.values() {
                if conn.user_id == uid
                    && conn.provider == provider
                    && conn.access_token_encrypted.is_some()
                {
                    return Ok(conn.access_token_encrypted.clone());
                }
            }
        }

        Ok(None)
    }

    /// Get provider metadata for a session/provider pair.
    /// Same resolution order as get_connection_token_for_session.
    pub async fn get_connection_metadata_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        let agent_identity_id = session.agent_identity_id;
        drop(sessions);

        if let Some(identity_id) = agent_identity_id {
            let id_connections = self.agent_identity_connections.read();
            if let Some(conn) = id_connections
                .values()
                .find(|c| c.agent_identity_id == identity_id && c.provider == provider)
            {
                return Ok(conn.provider_metadata.as_ref().cloned());
            }
        }

        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            if let Some(conn) = connections
                .values()
                .find(|c| c.user_id == uid && c.provider == provider)
            {
                return Ok(conn.provider_metadata.as_ref().cloned());
            }
        }

        Ok(None)
    }

    /// Resolve the user whose connection would be used for a session/provider pair.
    /// Returns None when the session uses an agent identity connection.
    pub async fn get_connection_user_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Uuid>> {
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        let agent_identity_id = session.agent_identity_id;
        drop(sessions);

        // If session has an identity connection, return None (no owning user)
        if let Some(identity_id) = agent_identity_id {
            let id_connections = self.agent_identity_connections.read();
            if id_connections
                .values()
                .any(|c| c.agent_identity_id == identity_id && c.provider == provider)
            {
                return Ok(None);
            }
        }

        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            if connections
                .values()
                .any(|conn| conn.user_id == uid && conn.provider == provider)
            {
                return Ok(Some(uid));
            }
        }

        Ok(None)
    }

    pub async fn get_connection_token_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| {
                c.user_id == user_id && c.provider == provider && c.access_token_encrypted.is_some()
            })
            .and_then(|c| c.access_token_encrypted.clone()))
    }

    /// Get the GitHub App installation ID for a session.
    /// Checks agent identity connections first, falls back to user connections.
    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        let agent_identity_id = session.agent_identity_id;
        drop(sessions);

        // Check identity connections first — if any exist for this provider,
        // they take full precedence (even if the specific credential field is absent).
        if let Some(identity_id) = agent_identity_id {
            let id_connections = self.agent_identity_connections.read();
            let has_any = id_connections
                .values()
                .any(|c| c.agent_identity_id == identity_id && c.provider == provider);
            if has_any {
                return Ok(id_connections
                    .values()
                    .find(|c| {
                        c.agent_identity_id == identity_id
                            && c.provider == provider
                            && c.installation_id.is_some()
                    })
                    .and_then(|c| c.installation_id));
            }
        }

        // Fall back to user connections via org membership
        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        let connections = self.user_connections.read();
        for uid in user_ids {
            for conn in connections.values() {
                if conn.user_id == uid
                    && conn.provider == provider
                    && conn.installation_id.is_some()
                {
                    return Ok(conn.installation_id);
                }
            }
        }

        Ok(None)
    }

    pub async fn get_installation_id_for_user(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<i64>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| c.user_id == user_id && c.provider == provider && c.installation_id.is_some())
            .and_then(|c| c.installation_id))
    }

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        let mut connections = self.user_connections.write();
        let before = connections.len();
        connections.retain(|_, c| !(c.user_id == user_id && c.provider == provider));
        Ok(connections.len() < before)
    }
}
