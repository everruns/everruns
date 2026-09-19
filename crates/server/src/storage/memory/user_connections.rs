// In-memory storage: User Connections

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_provider::typed_id::SessionId;
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
        connections.sort_by_key(|connection| connection.provider.clone());
        Ok(connections)
    }

    pub async fn list_user_mcp_connections(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Vec<UserMcpConnectionRow>> {
        let servers = self.mcp_servers.read();
        let mut rows = self
            .user_connections
            .read()
            .values()
            .filter(|connection| connection.user_id == user_id)
            .filter_map(|connection| {
                servers
                    .values()
                    .find(|server| {
                        server.org_id == org_id
                            && connection.provider
                                == everruns_core::mcp_oauth_provider_id_for_uuid(server.id.uuid())
                    })
                    .map(|server| UserMcpConnectionRow {
                        provider: connection.provider.clone(),
                        provider_username: connection.provider_username.clone(),
                        scopes: connection.scopes.clone(),
                        connected_at: connection.created_at,
                        server_id: server.id,
                        server_name: server.name.clone(),
                        server_url: server.url.clone(),
                        server_status: server.status.clone(),
                    })
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.server_name.to_lowercase());
        Ok(rows)
    }

    pub async fn update_user_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<UserConnectionRow>> {
        let mut connections = self.user_connections.write();
        let Some(connection) = connections.get_mut(&input.connection_id) else {
            return Ok(None);
        };
        if connection.connection_type != "oauth" {
            return Ok(None);
        }
        connection.access_token_encrypted = Some(input.access_token_encrypted);
        connection.refresh_token_encrypted = Some(input.refresh_token_encrypted);
        connection.expires_at = input.expires_at;
        if input.scopes.is_some() {
            connection.scopes = input.scopes;
        }
        connection.updated_at = Self::now();
        Ok(Some(connection.clone()))
    }

    /// Get encrypted connection token for a session.
    ///
    /// If the session has an `agent_identity_id`, checks `agent_identity_connections`
    /// first; falls back to `user_connections` for the session's resolved owner user.
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
        let agent_identity_id = session.agent_identity_id;
        let resolved_owner_user_id = session.resolved_owner_user_id;
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

        let Some(owner_user_id) = resolved_owner_user_id else {
            return Ok(None);
        };

        let connections = self.user_connections.read();
        Ok(connections
            .values()
            .find(|conn| {
                conn.user_id == owner_user_id
                    && conn.provider == provider
                    && conn.access_token_encrypted.is_some()
            })
            .and_then(|conn| conn.access_token_encrypted.clone()))
    }

    /// Whether a human actually initiated this session.
    ///
    /// True only when the owning principal is itself a `user` principal. See
    /// the Postgres twin for why the denormalized `resolved_owner_user_id` is
    /// not the right signal (EVE-1029).
    pub async fn session_has_human_initiator(&self, session_id: SessionId) -> Result<bool> {
        let sessions = self.sessions.read();
        let Some(session) = sessions.get(&session_id) else {
            return Ok(false);
        };
        let owner_principal_id = session.owner_principal_id;
        drop(sessions);

        let principals = self.principals.read();
        Ok(principals
            .get(&owner_principal_id)
            .is_some_and(|p| p.kind == "user"))
    }

    /// Get the agent identity's connection for a session/provider pair.
    ///
    /// Reads identity connections only — never a user connection (EVE-1029).
    pub async fn get_agent_identity_connection_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_agent_identity_connection_row_for_session(session_id, provider)
            .await?
            .and_then(|row| row.access_token_encrypted))
    }

    pub async fn get_agent_identity_connection_row_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<AgentIdentityConnectionRow>> {
        let sessions = self.sessions.read();
        let Some(session) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let Some(identity_id) = session.agent_identity_id else {
            return Ok(None);
        };
        drop(sessions);

        let id_connections = self.agent_identity_connections.read();
        Ok(id_connections
            .values()
            .find(|c| c.agent_identity_id == identity_id && c.provider == provider)
            .cloned())
    }

    /// Get the invoking user's own connection row for a session/provider pair.
    ///
    /// Reads user connections only — an identity grant for the same provider
    /// neither satisfies nor suppresses this lookup (EVE-1029).
    pub async fn get_owner_user_connection_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        let sessions = self.sessions.read();
        let Some(session) = sessions.get(&session_id) else {
            return Ok(None);
        };
        let Some(owner_user_id) = session.resolved_owner_user_id else {
            return Ok(None);
        };
        drop(sessions);

        let connections = self.user_connections.read();
        let mut matched: Vec<_> = connections
            .values()
            .filter(|conn| {
                conn.user_id == owner_user_id
                    && conn.provider == provider
                    && conn.access_token_encrypted.is_some()
            })
            .cloned()
            .collect();
        matched.sort_by_key(|conn| conn.created_at);
        Ok(matched.into_iter().next())
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
        let agent_identity_id = session.agent_identity_id;
        let resolved_owner_user_id = session.resolved_owner_user_id;
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

        let Some(owner_user_id) = resolved_owner_user_id else {
            return Ok(None);
        };

        let connections = self.user_connections.read();
        Ok(connections
            .values()
            .find(|c| c.user_id == owner_user_id && c.provider == provider)
            .and_then(|conn| conn.provider_metadata.as_ref().cloned()))
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
        let agent_identity_id = session.agent_identity_id;
        let resolved_owner_user_id = session.resolved_owner_user_id;
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

        let Some(owner_user_id) = resolved_owner_user_id else {
            return Ok(None);
        };
        let connections = self.user_connections.read();
        if connections
            .values()
            .any(|conn| conn.user_id == owner_user_id && conn.provider == provider)
        {
            return Ok(Some(owner_user_id));
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
    /// Checks agent identity connections first, falls back to the session owner user connection.
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
        let agent_identity_id = session.agent_identity_id;
        let resolved_owner_user_id = session.resolved_owner_user_id;
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

        let Some(owner_user_id) = resolved_owner_user_id else {
            return Ok(None);
        };

        let connections = self.user_connections.read();
        Ok(connections
            .values()
            .find(|conn| {
                conn.user_id == owner_user_id
                    && conn.provider == provider
                    && conn.installation_id.is_some()
            })
            .and_then(|conn| conn.installation_id))
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

    pub async fn get_user_id_by_installation_id(
        &self,
        provider: &str,
        installation_id: i64,
    ) -> Result<Option<Uuid>> {
        Ok(self
            .user_connections
            .read()
            .values()
            .find(|c| c.provider == provider && c.installation_id == Some(installation_id))
            .map(|c| c.user_id))
    }

    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        let mut connections = self.user_connections.write();
        let before = connections.len();
        connections.retain(|_, c| !(c.user_id == user_id && c.provider == provider));
        Ok(connections.len() < before)
    }
}
