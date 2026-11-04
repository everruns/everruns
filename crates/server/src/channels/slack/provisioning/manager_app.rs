//! The deployment's Slack manager app: an organization connects a workspace
//! by one OAuth consent instead of pasting a configuration token (EVE-1102).
//! The grant lands in the same connection store as a pasted token; only how
//! it is refreshed differs.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{IssuedTokens, SlackApiProvisioner, malformed_response, storage_error};
use crate::domains::agent_channels::record::slack_provisioning::{
    SlackProvisioningError, SlackProvisioningResult,
};
use crate::storage::{OrgSlackConnectionRow, SlackGrantKind, UpsertOrgSlackConnection};

/// Where Slack's OAuth consent screen lives.
pub(super) const SLACK_OAUTH_AUTHORIZE_URL: &str = "https://slack.com/oauth/v2/authorize";
/// User scopes the manager app consent asks for (EVE-1102).
///
/// Decision: `managed_apps:install` is deliberately not requested. Slack
/// reserves it for partners, and what a grant without it can do is unknown, so
/// agent apps keep their own one-click consent. Adding it later removes that
/// consent without changing this connection's storage or refresh.
pub(super) const MANAGER_APP_USER_SCOPES: &[&str] =
    &["app_configurations:read", "app_configurations:write"];
/// How long a started manager-app consent stays valid.
pub(crate) const MANAGER_CONNECT_STATE_TTL: chrono::Duration = chrono::Duration::minutes(30);
/// Binds a sealed state to this flow, so a state minted for another
/// encrypted round trip can never be replayed here.
pub(super) const MANAGER_CONNECT_STATE_PURPOSE: &str = "slack_manager_connect";

/// The deployment's Slack manager app: one distributed app whose OAuth consent
/// grants an organization's workspace connection, so no admin pastes a
/// configuration token (EVE-1102).
///
/// Optional. Without it, workspaces connect by configuration token as before.
/// Slack only lets enrolled manager apps request the `app_configurations`
/// scopes, so configuring one is useful only once Slack has enabled manager app
/// support for it.
#[derive(Clone)]
pub struct SlackManagerApp {
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for SlackManagerApp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackManagerApp")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl SlackManagerApp {
    pub const CLIENT_ID_ENV: &'static str = "SLACK_MANAGER_APP_CLIENT_ID";
    pub const CLIENT_SECRET_ENV: &'static str = "SLACK_MANAGER_APP_CLIENT_SECRET";

    pub fn from_env() -> Option<Self> {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        match (read(Self::CLIENT_ID_ENV), read(Self::CLIENT_SECRET_ENV)) {
            (Some(client_id), Some(client_secret)) => Some(Self {
                client_id,
                client_secret,
            }),
            (None, None) => None,
            _ => {
                tracing::warn!(
                    "Slack manager app ignored: set both {} and {}",
                    Self::CLIENT_ID_ENV,
                    Self::CLIENT_SECRET_ENV
                );
                None
            }
        }
    }
}

/// State carried through Slack's consent redirect. Encrypted, so it cannot be
/// forged or altered. The ledger binds this payload to a secret that stays in
/// the initiating browser, never in Slack's redirect URL.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ManagerConnectState {
    pub(super) purpose: String,
    pub(crate) org_id: i64,
    pub(crate) transaction_id: Uuid,
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
}

fn manager_connect_hash(
    state: &ManagerConnectState,
    browser_token: &str,
) -> SlackProvisioningResult<Vec<u8>> {
    let payload = serde_json::to_vec(&(state, browser_token))
        .map_err(|_| malformed_response("manager connect state"))?;
    Ok(Sha256::digest(payload).to_vec())
}

impl SlackApiProvisioner {
    pub fn with_manager_app(mut self, manager_app: Option<SlackManagerApp>) -> Self {
        self.manager_app = manager_app;
        self
    }

    /// Whether workspaces can connect by Slack consent instead of a pasted token.
    pub fn manager_app_configured(&self) -> bool {
        self.manager_app.is_some()
    }

    /// Slack's consent URL for connecting a workspace through the manager app.
    ///
    /// The manager app has no bot of its own (`scope` is empty); everything it
    /// needs is a user grant over app configuration.
    pub(crate) async fn manager_authorize_url(
        &self,
        org_id: i64,
        redirect_uri: &str,
        browser_token: &str,
    ) -> SlackProvisioningResult<(String, Uuid)> {
        let manager = self
            .manager_app
            .as_ref()
            .ok_or(SlackProvisioningError::Unavailable)?;
        let state = ManagerConnectState {
            purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
            org_id,
            transaction_id: Uuid::new_v4(),
            expires_at: chrono::Utc::now() + MANAGER_CONNECT_STATE_TTL,
        };
        let bytes =
            serde_json::to_vec(&state).map_err(|_| malformed_response("manager connect state"))?;
        let sealed = self.encryption.encrypt(&bytes).map_err(|error| {
            SlackProvisioningError::Unreachable(format!(
                "could not seal Slack connect state: {error}"
            ))
        })?;
        let sealed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed);
        self.db
            .register_connection_setup(
                &state.transaction_id.to_string(),
                MANAGER_CONNECT_STATE_PURPOSE,
                &manager_connect_hash(&state, browser_token)?,
                state.expires_at,
            )
            .await
            .map_err(storage_error)?;
        Ok((
            format!(
                "{SLACK_OAUTH_AUTHORIZE_URL}?client_id={}&scope=&user_scope={}&state={}&redirect_uri={}",
                urlencoding::encode(&manager.client_id),
                urlencoding::encode(&MANAGER_APP_USER_SCOPES.join(",")),
                urlencoding::encode(&sealed),
                urlencoding::encode(redirect_uri),
            ),
            state.transaction_id,
        ))
    }

    /// The organization a consent callback's `state` was minted for, or `None`
    /// when it is forged, altered, minted for another flow, or expired.
    pub(crate) fn open_manager_state(&self, sealed: &str) -> Option<ManagerConnectState> {
        if sealed.len() > 2048 {
            return None;
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sealed.trim())
            .ok()?;
        let state: ManagerConnectState =
            serde_json::from_slice(&self.encryption.decrypt(&bytes).ok()?).ok()?;
        (state.purpose == MANAGER_CONNECT_STATE_PURPOSE && state.expires_at > chrono::Utc::now())
            .then_some(state)
    }

    /// THREAT[TM-SLACK-014]: wrong browsers cannot consume a pending consent;
    /// one atomic delete across API replicas wins before any code exchange.
    /// Consent errors and exchange failures are terminal: start a new consent
    /// rather than retrying a possibly spent Slack authorization code.
    pub(crate) async fn consume_manager_state(
        &self,
        state: &ManagerConnectState,
        browser_token: &str,
    ) -> SlackProvisioningResult<bool> {
        self.db
            .consume_connection_setup(
                &state.transaction_id.to_string(),
                MANAGER_CONNECT_STATE_PURPOSE,
                &manager_connect_hash(state, browser_token)?,
            )
            .await
            .map_err(storage_error)
    }

    /// Exchange a manager app consent code and store the grant as the
    /// organization's connection to that workspace.
    ///
    /// Connecting a workspace already connected by configuration token
    /// replaces it: one workspace, one connection, whichever way it came.
    pub async fn connect_with_manager_code(
        &self,
        org_id: i64,
        code: &str,
        redirect_uri: &str,
    ) -> SlackProvisioningResult<OrgSlackConnectionRow> {
        let manager = self
            .manager_app
            .as_ref()
            .ok_or(SlackProvisioningError::Unavailable)?;
        let request = self
            .client
            .post(format!("{}/oauth.v2.access", self.api_base))
            .form(&[
                ("client_id", manager.client_id.as_str()),
                ("client_secret", manager.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ]);
        let response = self.send(request).await?;
        let user = response
            .get("authed_user")
            .ok_or_else(|| malformed_response("oauth.v2.access"))?;
        let granted = user
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        // Without write access the grant cannot create an agent app, so it is
        // refused rather than stored as a connection that can never work.
        if !granted
            .split(',')
            .any(|scope| scope.trim() == "app_configurations:write")
        {
            return Err(SlackProvisioningError::Rejected(
                "missing_scope".to_string(),
            ));
        }
        let issued = IssuedTokens::from_oauth(user, "oauth.v2.access")?;
        let team_id = response
            .pointer("/team/id")
            .and_then(serde_json::Value::as_str)
            .filter(|team| !team.is_empty())
            .ok_or_else(|| malformed_response("oauth.v2.access"))?
            .to_string();
        let team_name = response
            .pointer("/team/name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        self.db
            .upsert_org_slack_connection(UpsertOrgSlackConnection {
                org_id,
                team_id,
                team_name,
                access_token_encrypted: self.encrypt(&issued.access_token)?,
                refresh_token_encrypted: self.encrypt(&issued.refresh_token)?,
                access_token_expires_at: issued.expires_at,
                grant_kind: SlackGrantKind::ManagerApp,
            })
            .await
            .map_err(storage_error)
    }

    /// Refresh a manager app grant. Slack issues it as a rotating user token,
    /// refreshed through `oauth.v2.access` with the manager app's client pair
    /// rather than `tooling.tokens.rotate`.
    pub(super) async fn refresh_manager_grant(
        &self,
        refresh_token: &str,
    ) -> SlackProvisioningResult<IssuedTokens> {
        // Without the manager app this deployment cannot refresh the grant.
        // Not `invalid_refresh_token`: the grant may still be good once the
        // configuration returns, so the claim is released, not wiped.
        let manager = self.manager_app.as_ref().ok_or_else(|| {
            SlackProvisioningError::Rejected("manager_app_not_configured".to_string())
        })?;
        let request = self
            .client
            .post(format!("{}/oauth.v2.access", self.api_base))
            .form(&[
                ("client_id", manager.client_id.as_str()),
                ("client_secret", manager.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ]);
        let response = self.send(request).await?;
        // A refreshed user token arrives at the top level; tolerate the
        // `authed_user` shape the initial exchange uses as well.
        let tokens = response
            .get("authed_user")
            .filter(|user| user.get("access_token").is_some())
            .unwrap_or(&response);
        let mut issued = IssuedTokens::from_oauth(tokens, "oauth.v2.access")?;
        issued.team_id = response
            .pointer("/team/id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        Ok(issued)
    }
}
