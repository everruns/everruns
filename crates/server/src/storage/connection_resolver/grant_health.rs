//! Whether an MCP OAuth grant can keep producing tokens, read without using it.
//!
//! A grant that can no longer produce a token (its provider refused the
//! refresh, its server rejected it, or it cannot be renewed and its access
//! token expired) used to vanish: resolution returned nothing, the server's
//! tools dropped out of every turn, and the console still said "Connected".
//!
//! Decisions:
//! - Such a grant is marked, never deleted (`provider_metadata.reconnect_required`
//!   with a reason and time). Signing in again replaces the row and so clears
//!   the mark; a refresh clears it too. Resolution returns nothing for a
//!   marked grant without asking the provider again.
//! - A refresh token saved before grants were bound to their server (#5239)
//!   is still never sent: the audience would be guessed from settings that may
//!   have changed. Its access token works until it expires, and Health warns
//!   [`EXPIRY_WARNING`] ahead, so a person can sign in again before the tools go.
//! - A transient refresh failure (network, 5xx) marks nothing: the next use
//!   tries again.
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::{DbConnectionResolver, McpRefreshGrant, OAuthScope, Result, VirtualUserConnectionRow};

/// `provider_metadata` key of the mark on a grant that needs sign-in again.
pub const RECONNECT_REQUIRED: &str = "reconnect_required";
/// `provider_metadata` key of the last time a server rejected the grant's
/// access token; a second rejection soon after marks the grant.
pub(super) const REJECTED_AT: &str = "rejected_at";
/// How long after one rejection a second one means the refresh did not help.
pub(super) const REJECTION_WINDOW: Duration = Duration::minutes(10);
/// How far ahead an expiry that nothing can renew is warned about.
pub const EXPIRY_WARNING: Duration = Duration::days(3);

/// Why a grant needs sign-in again. Stored as the mark's `reason`.
pub mod reason {
    /// The refresh token was saved before grants were bound to their server.
    pub const UNBOUND: &str = "unbound";
    /// The server's sign-in settings changed since the grant was made.
    pub const SETTINGS_CHANGED: &str = "settings_changed";
    /// The provider issued no refresh token.
    pub const NO_REFRESH_TOKEN: &str = "no_refresh_token";
    /// The provider refused the refresh (`invalid_grant`).
    pub const REFUSED: &str = "refused";
    /// The server kept rejecting the access token.
    pub const REJECTED: &str = "rejected";
}

/// What a grant can still do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantHealth {
    /// It produces tokens, renewing them as needed.
    Usable,
    /// Its access token works until `at`, and nothing can renew it. Health
    /// warns about it only within [`EXPIRY_WARNING`]; the agent's MCP panel
    /// shows it at once.
    ExpiresSoon { at: DateTime<Utc>, reason: String },
    /// It produces no token until someone signs in again.
    NeedsSignIn {
        reason: String,
        since: Option<DateTime<Utc>>,
    },
}

/// The mark on a grant that needs sign-in again: its reason and time.
pub fn reconnect_mark(row: &VirtualUserConnectionRow) -> Option<(String, Option<DateTime<Utc>>)> {
    let mark = row.provider_metadata.as_ref()?.get(RECONNECT_REQUIRED)?;
    let reason = mark
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(reason::REFUSED)
        .to_owned();
    let at = mark
        .get("at")
        .and_then(serde_json::Value::as_str)
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
    Some((reason, at))
}

/// Whether a server rejected the grant's access token within
/// [`REJECTION_WINDOW`] of `now`.
pub(super) fn rejected_recently(row: &VirtualUserConnectionRow, now: DateTime<Utc>) -> bool {
    row.provider_metadata
        .as_ref()
        .and_then(|m| m.get(REJECTED_AT))
        .and_then(serde_json::Value::as_str)
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| now - at.with_timezone(&Utc) < REJECTION_WINDOW)
}

impl DbConnectionResolver {
    /// What the MCP OAuth grant `row` (for catalog server `server_id` in
    /// `org_id`) can still do. Reads only; never refreshes.
    pub async fn mcp_grant_health(
        &self,
        org_id: i64,
        server_id: Uuid,
        row: &VirtualUserConnectionRow,
    ) -> Result<GrantHealth> {
        if let Some((reason, since)) = reconnect_mark(row) {
            return Ok(GrantHealth::NeedsSignIn { reason, since });
        }
        let Some(expires_at) = row.expires_at.filter(|_| row.connection_type == "oauth") else {
            return Ok(GrantHealth::Usable);
        };
        let Some(reason) = self
            .unrenewable_reason(OAuthScope::Org(org_id), server_id, row)
            .await?
        else {
            return Ok(GrantHealth::Usable);
        };
        let now = Utc::now();
        Ok(if expires_at <= now {
            GrantHealth::NeedsSignIn {
                reason: reason.to_owned(),
                since: Some(expires_at),
            }
        } else {
            GrantHealth::ExpiresSoon {
                at: expires_at,
                reason: reason.to_owned(),
            }
        })
    }

    /// Why `row`'s refresh token cannot renew it, or `None` when it can.
    pub(super) async fn unrenewable_reason(
        &self,
        scope: OAuthScope,
        server_id: Uuid,
        row: &VirtualUserConnectionRow,
    ) -> Result<Option<&'static str>> {
        let Some(encrypted) = row.refresh_token_encrypted.as_deref() else {
            return Ok(Some(reason::NO_REFRESH_TOKEN));
        };
        let config = match scope {
            OAuthScope::Session(session) => self.oauth_client_config(session, server_id).await?,
            OAuthScope::Org(org) => self.oauth_client_config_in_org(org, server_id).await?,
        };
        let Some(config) = config else {
            return Ok(Some(reason::SETTINGS_CHANGED));
        };
        let value = self.decrypt(encrypted, "OAuth refresh token")?;
        Ok(unusable_grant_reason(&value, &config.config_hash))
    }

    /// Mark `row` as needing sign-in again for `reason`, unless it changed
    /// since it was read. Best effort: a failed write only logs.
    pub(super) async fn mark_reconnect_required(
        &self,
        row: &VirtualUserConnectionRow,
        reason: &'static str,
    ) {
        let Some(expected) = row.access_token_encrypted.as_deref() else {
            return;
        };
        match self
            .db
            .mark_runtime_connection_reconnect_required(row.id, expected, reason)
            .await
        {
            Ok(true) => tracing::warn!(
                connection_id = %row.id,
                provider = %row.provider,
                reason,
                "MCP OAuth grant needs sign-in again"
            ),
            Ok(false) => {}
            Err(error) => tracing::warn!(
                connection_id = %row.id,
                %error,
                "Could not mark an MCP OAuth grant as needing sign-in"
            ),
        }
    }
}

/// Why the stored refresh value cannot be used with the server's current
/// settings (`config_hash`), or `None` when it can.
pub(super) fn unusable_grant_reason(value: &str, config_hash: &str) -> Option<&'static str> {
    if McpRefreshGrant::decode(value, config_hash).is_some() {
        return None;
    }
    // A grant envelope bound to other settings, or a plain token saved before
    // grants were bound at all.
    match serde_json::from_str::<McpRefreshGrant>(value) {
        Ok(grant) if !grant.resource.is_empty() => Some(reason::SETTINGS_CHANGED),
        _ => Some(reason::UNBOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_legacy_refresh_token_is_unbound() {
        assert_eq!(
            unusable_grant_reason("plain-refresh-token", "hash"),
            Some(reason::UNBOUND)
        );
    }

    #[test]
    fn a_grant_for_other_settings_is_reported_as_changed_settings() {
        let grant = McpRefreshGrant {
            refresh_token: "r".into(),
            resource: "https://mcp.example".into(),
            config_hash: "old".into(),
        }
        .encode();
        assert_eq!(
            unusable_grant_reason(&grant, "new"),
            Some(reason::SETTINGS_CHANGED)
        );
        assert_eq!(unusable_grant_reason(&grant, "old"), None);
    }
}
