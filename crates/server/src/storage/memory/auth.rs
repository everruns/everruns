// In-memory storage: Personal Access Tokens, CLI Auth Sessions, Refresh Tokens

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Personal Access Tokens
    // ============================================

    pub async fn create_personal_access_token(
        &self,
        input: CreatePersonalAccessTokenRow,
    ) -> Result<PersonalAccessTokenRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = PersonalAccessTokenRow {
            id,
            user_id: input.user_id,
            name: input.name,
            token_hash: input.token_hash,
            token_prefix: input.token_prefix,
            scopes: serde_json::to_value(&input.scopes)?,
            expires_at: input.expires_at,
            last_used_at: None,
            created_at: now,
            metadata: input.metadata,
        };
        self.personal_access_tokens.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_personal_access_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessTokenRow>> {
        Ok(self
            .personal_access_tokens
            .read()
            .values()
            .find(|t| t.token_hash == token_hash)
            .cloned())
    }

    pub async fn list_personal_access_tokens_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<PersonalAccessTokenRow>> {
        let tokens = self.personal_access_tokens.read();
        let mut result: Vec<_> = tokens
            .values()
            .filter(|t| t.user_id == user_id)
            .cloned()
            .collect();
        result.sort_by_key(|token| std::cmp::Reverse(token.created_at));
        Ok(result)
    }

    pub async fn update_personal_access_token_last_used(&self, id: Uuid) -> Result<()> {
        if let Some(token) = self.personal_access_tokens.write().get_mut(&id) {
            token.last_used_at = Some(Self::now());
        }
        Ok(())
    }

    pub async fn delete_personal_access_token(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let mut tokens = self.personal_access_tokens.write();
        if let Some(token) = tokens.get(&id)
            && token.user_id == user_id
        {
            tokens.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    // ============================================
    // CLI Auth Sessions
    // ============================================

    pub async fn create_cli_auth_session(
        &self,
        input: CreateCliAuthSessionRow,
    ) -> Result<CliAuthSessionRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = CliAuthSessionRow {
            id,
            state: input.state,
            exchange_code: input.exchange_code,
            user_id: None,
            redirect_port: input.redirect_port,
            completed: false,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.cli_auth_sessions.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_cli_auth_session_by_state(
        &self,
        state: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let now = Self::now();
        Ok(self
            .cli_auth_sessions
            .read()
            .values()
            .find(|s| s.state == state && s.expires_at > now)
            .cloned())
    }

    pub async fn get_cli_auth_session_by_exchange_code(
        &self,
        code: &str,
    ) -> Result<Option<CliAuthSessionRow>> {
        let now = Self::now();
        Ok(self
            .cli_auth_sessions
            .read()
            .values()
            .find(|s| s.exchange_code == code && s.expires_at > now)
            .cloned())
    }

    pub async fn complete_cli_auth_session(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        if let Some(session) = self.cli_auth_sessions.write().get_mut(&id)
            && !session.completed
        {
            session.user_id = Some(user_id);
            session.completed = true;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_expired_cli_auth_sessions(&self) -> Result<u64> {
        let now = Self::now();
        let mut sessions = self.cli_auth_sessions.write();
        let before = sessions.len();
        sessions.retain(|_, s| s.expires_at > now);
        Ok((before - sessions.len()) as u64)
    }

    pub async fn delete_cli_auth_session(&self, id: Uuid) -> Result<bool> {
        Ok(self.cli_auth_sessions.write().remove(&id).is_some())
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = RefreshTokenRow {
            id,
            user_id: input.user_id,
            token_hash: input.token_hash,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.refresh_tokens.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        Ok(self
            .refresh_tokens
            .read()
            .values()
            .find(|t| t.token_hash == token_hash)
            .cloned())
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        Ok(self.refresh_tokens.write().remove(&id).is_some())
    }

    /// EVE-454 / TM-AUTH-018: atomic consume mirroring the Postgres
    /// `DELETE … RETURNING`. Performs the lookup and removal under a single
    /// write lock so concurrent callers cannot both observe the row.
    pub async fn consume_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        let now = Self::now();
        let mut tokens = self.refresh_tokens.write();
        let id = tokens
            .iter()
            .find(|(_, t)| t.token_hash == token_hash && t.expires_at > now)
            .map(|(id, _)| *id);
        Ok(id.and_then(|id| tokens.remove(&id)))
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        let now = Self::now();
        let mut tokens = self.refresh_tokens.write();
        let to_remove: Vec<Uuid> = tokens
            .iter()
            .filter(|(_, t)| t.expires_at < now)
            .map(|(id, _)| *id)
            .collect();
        let count = to_remove.len() as u64;
        for id in to_remove {
            tokens.remove(&id);
        }
        Ok(count)
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        let mut tokens = self.refresh_tokens.write();
        let to_remove: Vec<Uuid> = tokens
            .iter()
            .filter(|(_, t)| t.user_id == user_id)
            .map(|(id, _)| *id)
            .collect();
        let count = to_remove.len() as u64;
        for id in to_remove {
            tokens.remove(&id);
        }
        Ok(count)
    }

    // ============================================
    // OAuth Clients (MCP OAuth 2.1)
    // ============================================

    pub async fn create_oauth_client(&self, input: CreateOAuthClientRow) -> Result<OAuthClientRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = OAuthClientRow {
            id,
            client_id: input.client_id,
            client_secret_hash: input.client_secret_hash,
            client_name: input.client_name,
            redirect_uris: input.redirect_uris,
            created_at: now,
        };
        self.oauth_clients.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_oauth_client_by_client_id(
        &self,
        client_id: &str,
    ) -> Result<Option<OAuthClientRow>> {
        Ok(self
            .oauth_clients
            .read()
            .values()
            .find(|c| c.client_id == client_id)
            .cloned())
    }

    // ============================================
    // OAuth Authorization Codes
    // ============================================

    pub async fn create_oauth_authorization_code(
        &self,
        input: CreateOAuthAuthorizationCodeRow,
    ) -> Result<OAuthAuthorizationCodeRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = OAuthAuthorizationCodeRow {
            id,
            code_hash: input.code_hash,
            client_id: input.client_id,
            user_id: input.user_id,
            org_id: input.org_id,
            redirect_uri: input.redirect_uri,
            code_challenge: input.code_challenge,
            code_challenge_method: input.code_challenge_method,
            scope: input.scope,
            consumed: false,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.oauth_authorization_codes
            .write()
            .insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_oauth_authorization_code_by_hash(
        &self,
        code_hash: &str,
    ) -> Result<Option<OAuthAuthorizationCodeRow>> {
        let now = Self::now();
        Ok(self
            .oauth_authorization_codes
            .read()
            .values()
            .find(|c| c.code_hash == code_hash && c.expires_at > now)
            .cloned())
    }

    pub async fn consume_oauth_authorization_code(&self, id: Uuid) -> Result<bool> {
        if let Some(code) = self.oauth_authorization_codes.write().get_mut(&id)
            && !code.consumed
        {
            code.consumed = true;
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_expired_oauth_authorization_codes(&self) -> Result<u64> {
        let now = Self::now();
        let mut codes = self.oauth_authorization_codes.write();
        let before = codes.len();
        codes.retain(|_, c| c.expires_at > now);
        Ok((before - codes.len()) as u64)
    }

    // ============================================
    // OAuth Refresh Tokens
    // ============================================

    pub async fn create_oauth_refresh_token(
        &self,
        input: CreateOAuthRefreshTokenRow,
    ) -> Result<OAuthRefreshTokenRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = OAuthRefreshTokenRow {
            id,
            token_hash: input.token_hash,
            client_id: input.client_id,
            user_id: input.user_id,
            org_id: input.org_id,
            scope: input.scope,
            expires_at: input.expires_at,
            created_at: now,
        };
        self.oauth_refresh_tokens.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_oauth_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OAuthRefreshTokenRow>> {
        let now = Self::now();
        Ok(self
            .oauth_refresh_tokens
            .read()
            .values()
            .find(|t| t.token_hash == token_hash && t.expires_at > now)
            .cloned())
    }

    pub async fn consume_oauth_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<OAuthRefreshTokenRow>> {
        let now = Self::now();
        let mut tokens = self.oauth_refresh_tokens.write();
        let id = tokens
            .iter()
            .find(|(_, t)| t.token_hash == token_hash && t.expires_at > now)
            .map(|(id, _)| *id);
        Ok(id.and_then(|id| tokens.remove(&id)))
    }

    pub async fn delete_oauth_refresh_token(&self, id: Uuid) -> Result<bool> {
        Ok(self.oauth_refresh_tokens.write().remove(&id).is_some())
    }

    pub async fn delete_expired_oauth_refresh_tokens(&self) -> Result<u64> {
        let now = Self::now();
        let mut tokens = self.oauth_refresh_tokens.write();
        let before = tokens.len();
        tokens.retain(|_, t| t.expires_at > now);
        Ok((before - tokens.len()) as u64)
    }
}
