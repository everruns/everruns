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

    /// Get encrypted connection token for a session's org member (in-memory equivalent).
    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        // Find the session to get org_id
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        drop(sessions);

        // Find org members
        let members = self.organization_members.read();
        let user_ids: Vec<Uuid> = members
            .iter()
            .filter(|((oid, _), _)| *oid == org_id)
            .map(|((_, uid), _)| *uid)
            .collect();
        drop(members);

        // Find first matching connection with an encrypted token
        let connections = self.user_connections.read();
        for uid in user_ids.clone() {
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
        drop(sessions);

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

    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        // Look up session → org → members → connections (same join as token lookup)
        let sessions = self.sessions.read();
        let session = match sessions.get(&session_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let org_id = session.org_id;
        drop(sessions);

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
