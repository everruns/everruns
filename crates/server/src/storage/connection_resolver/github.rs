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
use crate::github_apps::{AppCredentials, TokenScope};
use everruns_contracts::runtime::{GitHubAccess, McpResolvedCredential};
use everruns_core::McpServerActsAs;

const PROVIDER: &str = "github";

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
    pub(super) async fn agent_app_credentials(
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

    /// Credentials of the App that owns `row`'s installation: the agent's own
    /// App when its identity created one, else the deployment's shared App.
    async fn github_app_credentials(
        &self,
        scope: OAuthScope,
        row: &VirtualUserConnectionRow,
    ) -> Result<Option<AppCredentials>> {
        if let Some(credentials) = self.agent_app_credentials(scope, row).await? {
            return Ok(Some(credentials));
        }
        self.github_app
            .as_ref()
            .map(GitHubAppTokenMinter::credentials)
            .transpose()
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
            if let Some(token) = self
                .scoped_github_token(OAuthScope::Session(session), row, &token_scope)
                .await?
            {
                return Ok(Some(McpResolvedCredential {
                    token,
                    acted_as: *identity,
                }));
            }
        }
        Ok(None)
    }

    async fn scoped_github_token(
        &self,
        scope: OAuthScope,
        row: VirtualUserConnectionRow,
        token_scope: &TokenScope,
    ) -> Result<Option<String>> {
        let Some(installation_id) = row.installation_id else {
            // A stored OAuth token (a person's own login) cannot be narrowed;
            // it reaches what the person can.
            return row
                .access_token_encrypted
                .as_deref()
                .map(|value| self.decrypt(value, "connection token"))
                .transpose();
        };
        let Some(credentials) = self.github_app_credentials(scope, &row).await? else {
            return Ok(None);
        };
        self.github_apps
            .mint_scoped_installation_token(&credentials, installation_id, token_scope)
            .await
            .map(Some)
            .map_err(|e| AgentLoopError::store(format!("Failed to mint GitHub token: {e}")))
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
}
