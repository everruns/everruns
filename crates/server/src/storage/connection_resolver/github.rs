//! GitHub tokens for one declared use (knowledge/integrations/github-model.md):
//! the identity the use acts as, minted down to the repositories and
//! permissions it asked for.
//!
//! Decision: the identity is chosen only by `acts_as`, never by who happens to
//! be speaking, so a person's turn cannot make a bot use that person's
//! connection, or the reverse. Under `user_or_service` the person answers when
//! they have a connection and the agent otherwise; the answer names which.
//! The App that mints is the one the connection belongs to: the agent's own App
//! when its identity created one, otherwise the deployment's shared App.

use super::*;
use crate::github_apps::{
    AppCredentials, EVERRUNS_APP_CONNECTION_TYPE, GITHUB_APP_CONNECTION_TYPE, TokenScope,
};
use everruns_contracts::runtime::{GitHubAccess, McpResolvedCredential};
use everruns_core::McpServerActsAs;

const PROVIDER: &str = "github";
type ScopedGitHubToken = (
    String,
    Option<everruns_contracts::runtime::GitHubInstallationScope>,
);

/// Which of the three GitHub identities (D1) a connection is. Derived from the
/// row rather than stored in its own column: `connection_type` marks an agent
/// App's installation (`github_app`) and a shared App installation an org
/// admin linked (`everruns_app`, D3); any other row holds a person's own
/// token. Rows that held a shared App installation before D3 (`oauth` with an
/// installation) were moved to org links by step 3b's migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubIdentityKind {
    AgentApp,
    EverrunsApp,
    Person,
}

impl GitHubIdentityKind {
    pub(crate) fn of(row: &VirtualUserConnectionRow) -> Self {
        if row.connection_type == GITHUB_APP_CONNECTION_TYPE {
            Self::AgentApp
        } else if row.connection_type == EVERRUNS_APP_CONNECTION_TYPE {
            Self::EverrunsApp
        } else {
            Self::Person
        }
    }
}

/// The token scope for a use. GitHub limits a token to repositories of the
/// installation's own account, so every repository must share one owner.
pub(super) fn token_scope(access: &GitHubAccess) -> Result<TokenScope> {
    let mut owner: Option<&str> = None;
    let mut repositories = Vec::with_capacity(access.repositories.len());
    for repository in &access.repositories {
        let Some((repo_owner, name)) = repository.split_once('/') else {
            return Err(AgentLoopError::store(format!(
                "GitHub repository must be owner/repo, got {repository}"
            )));
        };
        if repo_owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(AgentLoopError::store(format!(
                "GitHub repository must be owner/repo, got {repository}"
            )));
        }
        match owner {
            Some(owner) if !owner.eq_ignore_ascii_case(repo_owner) => {
                return Err(AgentLoopError::store(
                    "One GitHub token covers repositories of one owner",
                ));
            }
            _ => owner = Some(repo_owner),
        }
        repositories.push(name.to_string());
    }
    for level in access.permissions.values() {
        if !matches!(level.as_str(), "read" | "write") {
            return Err(AgentLoopError::store(format!(
                "GitHub permission level must be read or write, got {level}"
            )));
        }
    }
    Ok(TokenScope {
        repositories,
        permissions: access.permissions.clone(),
    })
}

impl GitHubAppTokenMinter {
    /// The shared App's credentials in the per-agent client's form.
    fn credentials(&self) -> Result<AppCredentials> {
        let app_id = self
            .app_id
            .trim()
            .parse()
            .map_err(|_| AgentLoopError::store("GITHUB_APP_ID is not a number"))?;
        Ok(AppCredentials {
            app_id,
            // Escaped newlines are common in Docker and CI env values.
            private_key_pem: self.private_key.replace("\\n", "\n"),
        })
    }
}

impl DbConnectionResolver {
    /// Credentials of the agent's own App, when `row`'s identity created one.
    async fn agent_app_credentials(
        &self,
        scope: OAuthScope,
        row: &VirtualUserConnectionRow,
    ) -> Result<Option<AppCredentials>> {
        let Some(app) = self
            .db
            .get_github_app_for_identity(self.scope_org_id(scope).await?, row.virtual_user_id)
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to resolve agent GitHub App: {e}"))
            })?
        else {
            return Ok(None);
        };
        let private_key_pem = self
            .encryption
            .decrypt_to_string(&app.private_key_encrypted)
            .map_err(|e| AgentLoopError::store(format!("Failed to decrypt GitHub App key: {e}")))?;
        Ok(Some(AppCredentials {
            app_id: app.app_id,
            private_key_pem,
        }))
    }

    /// Credentials of the App that minted `row`'s installation, chosen by its
    /// kind. An agent App's installation mints only with that App's key and a
    /// shared App installation only with the shared key; neither stands in for
    /// the other when one is missing.
    async fn installation_credentials(
        &self,
        scope: OAuthScope,
        row: &VirtualUserConnectionRow,
    ) -> Result<Option<AppCredentials>> {
        match GitHubIdentityKind::of(row) {
            GitHubIdentityKind::AgentApp => self.agent_app_credentials(scope, row).await,
            GitHubIdentityKind::EverrunsApp => self
                .github_app
                .as_ref()
                .map(GitHubAppTokenMinter::credentials)
                .transpose(),
            GitHubIdentityKind::Person => Ok(None),
        }
    }

    /// Resolve a GitHub token for one use; see `get_github_token`.
    pub(super) async fn github_token(
        &self,
        session: SessionId,
        access: &GitHubAccess,
    ) -> Result<Option<McpResolvedCredential>> {
        let token_scope = token_scope(access)?;
        for identity in access.acts_as.resolution_order() {
            let row = match identity {
                McpServerActsAs::User => match self.runtime_subject(session).await? {
                    Some(id) => self
                        .db
                        .get_virtual_user_connection(id, PROVIDER)
                        .await
                        .map_err(|e| AgentLoopError::store(e.to_string()))?,
                    None => None,
                },
                McpServerActsAs::Service => self.service_connection(session, PROVIDER).await?,
                McpServerActsAs::None | McpServerActsAs::UserOrService => None,
            };
            let Some(row) = row else {
                continue;
            };
            if let Some((token, github_installation_scope)) = self
                .scoped_github_credential(
                    OAuthScope::Session(session),
                    row,
                    &token_scope,
                    Some(access),
                )
                .await?
            {
                return Ok(Some(McpResolvedCredential {
                    token,
                    acted_as: *identity,
                    github_installation_scope,
                }));
            }
        }
        Ok(None)
    }

    /// Resolve a GitHub token acting as `owner`; see
    /// `get_github_token_for_owner`.
    pub(super) async fn github_token_for_owner(
        &self,
        org_id: i64,
        owner: Uuid,
        access: &GitHubAccess,
    ) -> Result<Option<String>> {
        let token_scope = token_scope(access)?;
        if let Some(row) = self
            .db
            .get_virtual_user_connection(VirtualUserId::from_uuid(owner), PROVIDER)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
            && let Some(token) = self
                .scoped_github_token(OAuthScope::Org(org_id), row, &token_scope)
                .await?
        {
            return Ok(Some(token));
        }
        // A sync is a service use on the org identity (D3): without a
        // connection of the owner's own, the org's link for the repository's
        // account answers. Syncs that relied on a person's own installation
        // keep working after step 3b moved it to the org.
        let Some(account) = access
            .repositories
            .first()
            .and_then(|repository| repository.split_once('/'))
            .map(|(account, _)| account)
        else {
            return Ok(None);
        };
        let Some(link) = self
            .db
            .list_everruns_app_links(org_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
            .into_iter()
            .find(|link| {
                link.provider_username
                    .as_deref()
                    .is_some_and(|login| login.eq_ignore_ascii_case(account))
            })
        else {
            return Ok(None);
        };
        self.scoped_github_token(OAuthScope::Org(org_id), link, &token_scope)
            .await
    }

    /// A token for `row` limited to `token_scope`; an empty scope gets
    /// everything the installation holds.
    pub(super) async fn scoped_github_token(
        &self,
        scope: OAuthScope,
        row: VirtualUserConnectionRow,
        token_scope: &TokenScope,
    ) -> Result<Option<String>> {
        Ok(self
            .scoped_github_credential(scope, row, token_scope, None)
            .await?
            .map(|(token, _)| token))
    }

    async fn scoped_github_credential(
        &self,
        scope: OAuthScope,
        row: VirtualUserConnectionRow,
        token_scope: &TokenScope,
        request: Option<&GitHubAccess>,
    ) -> Result<Option<ScopedGitHubToken>> {
        if !self.authorize_connection(scope, &row, request).await? {
            return Ok(None);
        }
        let installation_id = match (GitHubIdentityKind::of(&row), row.installation_id) {
            (GitHubIdentityKind::Person, _) => {
                // A person's own token cannot be narrowed; it reaches what
                // the person can.
                return Ok(self
                    .person_github_token(row)
                    .await?
                    .map(|token| (token, None)));
            }
            (_, Some(installation_id)) => installation_id,
            (_, None) => return Ok(None),
        };
        let Some(credentials) = self.installation_credentials(scope, &row).await? else {
            return Ok(None);
        };
        self.github_apps
            .mint_scoped_installation_credential(&credentials, installation_id, token_scope)
            .await
            .map(|credential| Some(credential.into_credential()))
            .map_err(|e| AgentLoopError::store(format!("Failed to mint GitHub token: {e}")))
    }
}

impl DbConnectionResolver {
    /// A person's own GitHub token (D1), refreshed when it is about to expire.
    /// Refreshing is single-flight per connection, because GitHub rotates the
    /// refresh token on use. A failed refresh reads as no token, so the use
    /// asks the person to connect GitHub again.
    async fn person_github_token(&self, row: VirtualUserConnectionRow) -> Result<Option<String>> {
        let fresh = |row: &VirtualUserConnectionRow| {
            row.expires_at
                .is_none_or(|expiry| expiry > Utc::now() + OAUTH_REFRESH_SKEW)
        };
        let Some(access_token) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        if fresh(&row) {
            return self.decrypt(access_token, "GitHub token").map(Some);
        }
        let Some(client) = self
            .github_app
            .as_ref()
            .and_then(|minter| minter.user_client.clone())
        else {
            return Ok(None);
        };
        let lock = self
            .refresh_locks
            .get_with(format!("github-person:{}", row.id), || {
                Arc::new(Mutex::new(()))
            });
        let _guard = lock.lock().await;
        // Another use may have refreshed it while this one waited.
        let Some(row) = self
            .db
            .get_virtual_user_connection(row.virtual_user_id, PROVIDER)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
            .filter(|current| current.id == row.id)
        else {
            return Ok(None);
        };
        if fresh(&row) {
            return row
                .access_token_encrypted
                .as_deref()
                .map(|value| self.decrypt(value, "GitHub token"))
                .transpose();
        }
        let Some(refresh_token) = row.refresh_token_encrypted.as_deref() else {
            return Ok(None);
        };
        let refresh_token = self.decrypt(refresh_token, "GitHub refresh token")?;
        let token = match crate::github_user_token::GitHubUserTokens::new(
            self.github_apps.endpoints().clone(),
        )
        .refresh(&client, &refresh_token)
        .await
        {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!(connection_id = %row.id, error = %e, "GitHub token refresh failed");
                return Ok(None);
            }
        };
        let encrypt = |value: &str| {
            self.encryption
                .encrypt_string(value)
                .map_err(|e| AgentLoopError::store(format!("Failed to encrypt GitHub token: {e}")))
        };
        // Compare the encrypted snapshot after the exchange. A disconnect or
        // reconnect during refresh must never recreate the old connection.
        let fresh_access = encrypt(&token.access_token)?;
        let fresh_refresh = match token.refresh_token.as_deref() {
            Some(v) => Some(encrypt(v)?),
            None => row.refresh_token_encrypted.clone(),
        };
        let updated=sqlx::query("UPDATE virtual_user_connections SET access_token_encrypted=$2,refresh_token_encrypted=$3,expires_at=$4,updated_at=now() WHERE id=$1 AND access_token_encrypted=$5 AND refresh_token_encrypted IS NOT DISTINCT FROM $6 AND EXISTS(SELECT 1 FROM resource_access r WHERE r.id=$1 AND r.revoked_at IS NULL)")
        .bind(row.id).bind(fresh_access).bind(fresh_refresh).bind(token.expires_at).bind(row.access_token_encrypted).bind(row.refresh_token_encrypted).execute(self.db.pool()).await.map_err(|_|AgentLoopError::store("Failed to persist refreshed GitHub connection"))?;
        if updated.rows_affected() != 1 {
            return Ok(None);
        }
        Ok(Some(token.access_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(repositories: &[&str], permissions: &[(&str, &str)]) -> GitHubAccess {
        GitHubAccess {
            acts_as: McpServerActsAs::Service,
            repositories: repositories.iter().map(|r| r.to_string()).collect(),
            permissions: permissions
                .iter()
                .map(|(n, l)| (n.to_string(), l.to_string()))
                .collect(),
        }
    }

    #[test]
    fn scope_keeps_repository_names_and_permissions() {
        let scope = token_scope(&access(
            &["acme/app", "Acme/api"],
            &[("pull_requests", "write"), ("contents", "read")],
        ))
        .unwrap();
        assert_eq!(scope.repositories, vec!["app", "api"]);
        assert_eq!(scope.permissions["pull_requests"], "write");
    }

    #[test]
    fn scope_refuses_mixed_owners_bad_names_and_levels() {
        assert!(token_scope(&access(&["acme/app", "other/app"], &[])).is_err());
        assert!(token_scope(&access(&["acme"], &[])).is_err());
        assert!(token_scope(&access(&["acme/a/b"], &[])).is_err());
        assert!(token_scope(&access(&[], &[("contents", "admin")])).is_err());
    }

    #[test]
    fn empty_access_is_unlimited() {
        assert_eq!(
            token_scope(&access(&[], &[])).unwrap(),
            TokenScope::default()
        );
    }

    #[test]
    fn kind_follows_the_connection_type_and_installation() {
        let row = |connection_type: &str, installation_id: Option<i64>| VirtualUserConnectionRow {
            id: Uuid::nil(),
            virtual_user_id: VirtualUserId::from_uuid(Uuid::nil()),
            provider: PROVIDER.to_string(),
            owner_scope: "user".to_string(),
            name: None,
            connection_type: connection_type.to_string(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id,
            provider_metadata: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert_eq!(
            GitHubIdentityKind::of(&row(GITHUB_APP_CONNECTION_TYPE, Some(1))),
            GitHubIdentityKind::AgentApp
        );
        assert_eq!(
            GitHubIdentityKind::of(&row("oauth", Some(1))),
            GitHubIdentityKind::Person
        );
        assert_eq!(
            GitHubIdentityKind::of(&row(EVERRUNS_APP_CONNECTION_TYPE, Some(1))),
            GitHubIdentityKind::EverrunsApp
        );
        assert_eq!(
            GitHubIdentityKind::of(&row("oauth", None)),
            GitHubIdentityKind::Person
        );
    }
}
