//! The worker's web search lookups: the organization's search settings and,
//! when a plan step runs, one search account's key.
//!
//! THREAT[TM-TOOL-041]: the key command answers only for an organization
//! connection of the caller's own org whose provider has a search driver
//! (`WEB_SEARCH_ACCOUNT_PROVIDERS`). A sandbox provider account, another
//! org's account or a user connection is `null`, so a worker cannot use this
//! command to read keys web search does not need. Fixed-text failures; the key
//! travels only in the answer, which `Command::run` never logs.

use super::owned_session;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use everruns_contracts::runtime::web_search::{WEB_SEARCH_ACCOUNT_PROVIDERS, WebSearchOrgSettings};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetWebSearchSettings {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_get_web_search_settings",
    category = "connections",
    description = "Internal: the session organization's web search defaults and search accounts (no keys).",
    method = "GET",
    path = "/internal/sessions/{session_id}/web_search/settings",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetWebSearchSettings {
    type Output = WebSearchOrgSettings;

    async fn execute(self, ctx: &Ctx) -> Result<WebSearchOrgSettings, CommandError> {
        owned_session(ctx, &self.session_id).await?;
        crate::domains::organizations::web_search::web_search_settings(&ctx.db, ctx.org_id())
            .await
            .map_err(|error| {
                tracing::error!(%error, "Failed to load web search settings");
                CommandError::internal(anyhow::anyhow!("Failed to load web search settings"))
            })
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetWebSearchAccountKey {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The organization search account (connection id).
    pub account_id: Uuid,
}

#[command(
    name = "worker_get_web_search_account_key",
    category = "connections",
    description = "Internal: decrypt one organization search account's key for a session of that organization; null when it is not a search account of the org.",
    method = "GET",
    path = "/internal/sessions/{session_id}/web_search/accounts/{account_id}/key",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetWebSearchAccountKey {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        owned_session(ctx, &self.session_id).await?;
        let failed =
            || CommandError::internal(anyhow::anyhow!("Failed to resolve search account key"));
        let Some(row) = ctx
            .db
            .get_organization_connection(ctx.org_id(), self.account_id)
            .await
            .map_err(|_| failed())?
        else {
            return Ok(None);
        };
        if !WEB_SEARCH_ACCOUNT_PROVIDERS.contains(&row.provider.as_str()) {
            return Ok(None);
        }
        let Some(encrypted) = row.access_token_encrypted else {
            return Ok(None);
        };
        let encryption = ctx
            .encryption
            .as_ref()
            .ok_or_else(|| CommandError::unavailable("Encryption not configured"))?;
        encryption
            .decrypt_to_string(&encrypted)
            .map(Some)
            .map_err(|error| {
                tracing::error!(%error, "Failed to decrypt search account key");
                failed()
            })
    }
}
