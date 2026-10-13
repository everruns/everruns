mod branding;

use std::sync::Arc;
use std::time::Duration;

use crate::domains::agent_channels::record::slack_provisioning::{
    SlackAppCredentials, SlackAppProvisioner, SlackProvisioningConnectionStatus,
    SlackProvisioningError, SlackProvisioningResult,
};
use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::{
    EncryptionService, OrgSlackConnectionRow, RotateOrgSlackConnection, SlackGrantKind,
    StorageBackend, UpsertOrgSlackConnection,
};

use everruns_integrations::slack_channel::web_api::SLACK_API_BASE;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const ROTATE_BEFORE_EXPIRY: chrono::Duration = chrono::Duration::minutes(5);
const ROTATION_WAIT_ATTEMPTS: usize = 40;
const ROTATION_WAIT: Duration = Duration::from_millis(50);
const AUTH_ERROR_CODES: &[&str] = &[
    "invalid_auth",
    "not_authed",
    "token_expired",
    "token_revoked",
];

/// Where Slack's OAuth consent screen lives.
const SLACK_OAUTH_AUTHORIZE_URL: &str = "https://slack.com/oauth/v2/authorize";
/// User scopes the manager app consent asks for (EVE-1102).
///
/// Decision: `managed_apps:install` is deliberately not requested. Slack
/// reserves it for partners, and what a grant without it can do is unknown, so
/// agent apps keep their own one-click consent. Adding it later removes that
/// consent without changing this connection's storage or refresh.
const MANAGER_APP_USER_SCOPES: &[&str] = &["app_configurations:read", "app_configurations:write"];
/// How long a started manager-app consent stays valid.
const MANAGER_CONNECT_STATE_TTL: chrono::Duration = chrono::Duration::minutes(30);
/// Binds a sealed state to this flow, so a state minted for another
/// encrypted round trip can never be replayed here.
const MANAGER_CONNECT_STATE_PURPOSE: &str = "slack_manager_connect";

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
/// forged or altered; it names the organization the grant is stored under.
#[derive(Debug, Serialize, Deserialize)]
struct ManagerConnectState {
    purpose: String,
    org_id: i64,
    /// Unix seconds after which the state is refused.
    expires_at: i64,
}

pub struct SlackProvisioningSetup {
    pub provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    pub connection_manager: Option<Arc<SlackApiProvisioner>>,
}

pub fn configure(
    supervisor: &mut crate::background::supervised_task::TaskSupervisor,
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    custom: Option<Arc<dyn SlackAppProvisioner>>,
) -> anyhow::Result<SlackProvisioningSetup> {
    if let Some(provisioner) = custom {
        return Ok(SlackProvisioningSetup {
            provisioner: Some(provisioner),
            connection_manager: None,
        });
    }
    let Some(encryption) = encryption else {
        return Ok(SlackProvisioningSetup {
            provisioner: None,
            connection_manager: None,
        });
    };
    let manager = Arc::new(
        SlackApiProvisioner::new(db, encryption)?.with_manager_app(SlackManagerApp::from_env()),
    );
    let provisioner: Arc<dyn SlackAppProvisioner> = manager.clone();
    let rotation_manager = manager.clone();
    supervisor.track(
        "slack_token_rotation",
        tokio::spawn(async move {
            loop {
                if let Err(error) = rotation_manager.rotate_due_connections().await {
                    tracing::warn!(%error, "Slack token rotation sweep failed");
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }),
    );
    Ok(SlackProvisioningSetup {
        provisioner: Some(provisioner),
        connection_manager: Some(manager),
    })
}

#[derive(Clone)]
pub struct SlackApiProvisioner {
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    client: reqwest::Client,
    api_base: String,
    manager_app: Option<SlackManagerApp>,
}

impl std::fmt::Debug for SlackApiProvisioner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackApiProvisioner")
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

impl SlackApiProvisioner {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Arc<EncryptionService>,
    ) -> anyhow::Result<Self> {
        Self::with_api_base(db, encryption, SLACK_API_BASE)
    }

    /// A provisioner talking to `api_base` instead of Slack. For tests.
    pub fn with_api_base(
        db: Arc<StorageBackend>,
        encryption: Arc<EncryptionService>,
        api_base: &str,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            db,
            encryption,
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            api_base: api_base.trim_end_matches('/').to_string(),
            manager_app: None,
        })
    }

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
    pub fn manager_authorize_url(
        &self,
        org_id: i64,
        redirect_uri: &str,
    ) -> SlackProvisioningResult<String> {
        let manager = self
            .manager_app
            .as_ref()
            .ok_or(SlackProvisioningError::Unavailable)?;
        let state = ManagerConnectState {
            purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
            org_id,
            expires_at: (chrono::Utc::now() + MANAGER_CONNECT_STATE_TTL).timestamp(),
        };
        let bytes =
            serde_json::to_vec(&state).map_err(|_| malformed_response("manager connect state"))?;
        let sealed = self.encryption.encrypt(&bytes).map_err(|error| {
            SlackProvisioningError::Unreachable(format!(
                "could not seal Slack connect state: {error}"
            ))
        })?;
        let sealed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed);
        Ok(format!(
            "{SLACK_OAUTH_AUTHORIZE_URL}?client_id={}&scope=&user_scope={}&state={}&redirect_uri={}",
            urlencoding::encode(&manager.client_id),
            urlencoding::encode(&MANAGER_APP_USER_SCOPES.join(",")),
            urlencoding::encode(&sealed),
            urlencoding::encode(redirect_uri),
        ))
    }

    /// The organization a consent callback's `state` was minted for, or `None`
    /// when it is forged, altered, minted for another flow, or expired.
    pub fn open_manager_state(&self, sealed: &str) -> Option<i64> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sealed.trim())
            .ok()?;
        let state: ManagerConnectState =
            serde_json::from_slice(&self.encryption.decrypt(&bytes).ok()?).ok()?;
        (state.purpose == MANAGER_CONNECT_STATE_PURPOSE
            && state.expires_at >= chrono::Utc::now().timestamp())
        .then_some(state.org_id)
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

    /// Connect the workspace a configuration refresh token belongs to.
    ///
    /// The token is rotated before anything is stored, which both proves it
    /// works and tells us which workspace it is for — Slack's rotate response
    /// carries `team_id`. Connecting a workspace that is already connected
    /// replaces its tokens rather than adding a second row.
    pub async fn connect(
        &self,
        org_id: i64,
        refresh_token: &str,
    ) -> SlackProvisioningResult<OrgSlackConnectionRow> {
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(SlackProvisioningError::Rejected(
                "invalid_refresh_token".to_string(),
            ));
        }
        let rotated = self.rotate(refresh_token).await?;
        let team_id = rotated
            .team_id
            .clone()
            .filter(|team| !team.is_empty())
            .ok_or_else(|| malformed_response("tooling.tokens.rotate"))?;
        let team_name = self.workspace_name(&rotated.access_token).await;
        self.db
            .upsert_org_slack_connection(UpsertOrgSlackConnection {
                org_id,
                team_id,
                team_name,
                access_token_encrypted: self.encrypt(&rotated.access_token)?,
                refresh_token_encrypted: self.encrypt(&rotated.refresh_token)?,
                access_token_expires_at: rotated.expires_at,
                grant_kind: SlackGrantKind::ConfigToken,
            })
            .await
            .map_err(storage_error)
    }

    pub async fn list_connections(
        &self,
        org_id: i64,
    ) -> SlackProvisioningResult<Vec<OrgSlackConnectionRow>> {
        self.db
            .list_org_slack_connections(org_id)
            .await
            .map_err(storage_error)
    }

    pub async fn test_connection(&self, org_id: i64, id: Uuid) -> SlackProvisioningResult<()> {
        let row = self.connection_by_id(org_id, id).await?;
        self.manifest_call(
            row,
            "/apps.manifest.validate",
            serde_json::json!({
                "manifest": {
                    "display_information": {"name": "Everruns connection test"}
                }
            }),
        )
        .await?;
        Ok(())
    }

    /// Forget a workspace's configuration token.
    ///
    /// Agent apps already installed there keep working: each holds its own
    /// bot token. What goes is the ability to create or update apps in it.
    pub async fn clear_connection(&self, org_id: i64, id: Uuid) -> SlackProvisioningResult<bool> {
        self.db
            .delete_org_slack_connection(org_id, id)
            .await
            .map_err(storage_error)
    }

    pub async fn rotate_due_connections(&self) -> anyhow::Result<()> {
        let due = self
            .db
            .list_due_org_slack_connections(chrono::Utc::now() + ROTATE_BEFORE_EXPIRY)
            .await?;
        for row in due {
            let (org_id, connection_id) = (row.org_id, row.id);
            if let Err(error) = self.rotate_row(row).await {
                tracing::warn!(org_id, %connection_id, %error, "Slack connection rotation failed");
            }
        }
        Ok(())
    }

    /// Best effort: the configuration token may not be allowed to read team
    /// info, and a missing name is a cosmetic gap, not a failed connection.
    async fn workspace_name(&self, access_token: &str) -> Option<String> {
        match self
            .post("/auth.test", Some(access_token), serde_json::json!({}))
            .await
        {
            Ok(body) => body
                .get("team")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            Err(error) => {
                tracing::debug!(%error, "Slack workspace name unavailable");
                None
            }
        }
    }

    async fn manifest_call(
        &self,
        row: OrgSlackConnectionRow,
        method: &str,
        body: serde_json::Value,
    ) -> SlackProvisioningResult<serde_json::Value> {
        let (org_id, id) = (row.org_id, row.id);
        let access_token = self.token_for_request(row).await?;
        match self.post(method, Some(&access_token), body.clone()).await {
            Err(SlackProvisioningError::Rejected(code)) if is_auth_error(&code) => {
                let row = self.connection_by_id(org_id, id).await?;
                let replacement = self.rotate_row(row).await?;
                self.post(method, Some(&replacement), body).await
            }
            result => result,
        }
    }

    async fn token_for_request(
        &self,
        row: OrgSlackConnectionRow,
    ) -> SlackProvisioningResult<String> {
        if row
            .access_token_expires_at
            .is_some_and(|expiry| expiry > chrono::Utc::now() + ROTATE_BEFORE_EXPIRY)
        {
            return self.decrypt_required(row.access_token_encrypted.as_deref());
        }
        self.rotate_row(row).await
    }

    async fn connection_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> SlackProvisioningResult<OrgSlackConnectionRow> {
        let row = self
            .db
            .get_org_slack_connection(org_id, id)
            .await
            .map_err(storage_error)?
            .ok_or(SlackProvisioningError::WorkspaceNotConnected)?;
        usable(row)
    }

    /// The connection an app should be created in or reaped from.
    ///
    /// Naming a workspace selects it. Naming none is allowed only while the
    /// organization has exactly one, so that a caller can never silently land
    /// an agent in the wrong workspace once a second is connected.
    async fn connection_for(
        &self,
        org_id: i64,
        team_id: Option<&str>,
    ) -> SlackProvisioningResult<OrgSlackConnectionRow> {
        let rows = self
            .db
            .list_org_slack_connections(org_id)
            .await
            .map_err(storage_error)?;
        let row = match team_id {
            Some(team_id) => rows
                .into_iter()
                .find(|row| row.team_id.as_deref() == Some(team_id))
                .ok_or(SlackProvisioningError::WorkspaceNotConnected)?,
            None => {
                let mut rows = rows.into_iter();
                match (rows.next(), rows.next()) {
                    (None, _) => return Err(SlackProvisioningError::OrgNotConnected),
                    (Some(only), None) => only,
                    (Some(_), Some(_)) => return Err(SlackProvisioningError::WorkspaceRequired),
                }
            }
        };
        usable(row)
    }

    async fn rotate_row(&self, row: OrgSlackConnectionRow) -> SlackProvisioningResult<String> {
        let original_generation = row.token_generation;
        let Some(claimed) = self
            .db
            .claim_org_slack_connection_rotation(row.id, original_generation)
            .await
            .map_err(storage_error)?
        else {
            return self
                .wait_for_rotation(row.org_id, row.id, original_generation)
                .await;
        };
        let refresh_token = self.decrypt_required(claimed.refresh_token_encrypted.as_deref())?;
        let refreshed = match claimed.grant_kind() {
            SlackGrantKind::ConfigToken => self.rotate(&refresh_token).await,
            SlackGrantKind::ManagerApp => self.refresh_manager_grant(&refresh_token).await,
        };
        let rotated = match refreshed {
            Ok(rotated) => rotated,
            Err(SlackProvisioningError::Rejected(code)) if code == "invalid_refresh_token" => {
                self.mark_reconnect(claimed.id, claimed.token_generation)
                    .await?;
                return Err(SlackProvisioningError::ReconnectRequired);
            }
            Err(SlackProvisioningError::Unreachable(_)) => {
                self.mark_reconnect(claimed.id, claimed.token_generation)
                    .await?;
                return Err(SlackProvisioningError::ReconnectRequired);
            }
            Err(error) => {
                self.release_claim(&claimed).await?;
                return Err(error);
            }
        };
        let replacement = self
            .db
            .rotate_org_slack_connection(RotateOrgSlackConnection {
                id: claimed.id,
                expected_generation: claimed.token_generation,
                team_id: rotated.team_id.clone().filter(|team| !team.is_empty()),
                access_token_encrypted: self.encrypt(&rotated.access_token)?,
                refresh_token_encrypted: self.encrypt(&rotated.refresh_token)?,
                access_token_expires_at: rotated.expires_at,
            })
            .await
            .map_err(storage_error)?
            .ok_or_else(|| {
                SlackProvisioningError::Unreachable(
                    "Slack credentials changed during rotation".to_string(),
                )
            })?;
        self.decrypt_required(replacement.access_token_encrypted.as_deref())
    }

    async fn wait_for_rotation(
        &self,
        org_id: i64,
        id: Uuid,
        original_generation: i64,
    ) -> SlackProvisioningResult<String> {
        for _ in 0..ROTATION_WAIT_ATTEMPTS {
            tokio::time::sleep(ROTATION_WAIT).await;
            let row = self
                .db
                .get_org_slack_connection(org_id, id)
                .await
                .map_err(storage_error)?
                .ok_or(SlackProvisioningError::WorkspaceNotConnected)?;
            if row.state == "reconnect_required" {
                return Err(SlackProvisioningError::ReconnectRequired);
            }
            if row.state == "connected" && row.token_generation > original_generation {
                return self.decrypt_required(row.access_token_encrypted.as_deref());
            }
        }
        Err(SlackProvisioningError::Unreachable(
            "Timed out waiting for Slack credential rotation".to_string(),
        ))
    }

    async fn release_claim(&self, claimed: &OrgSlackConnectionRow) -> SlackProvisioningResult<()> {
        self.db
            .rotate_org_slack_connection(RotateOrgSlackConnection {
                id: claimed.id,
                expected_generation: claimed.token_generation,
                team_id: None,
                access_token_encrypted: claimed
                    .access_token_encrypted
                    .clone()
                    .ok_or_else(missing_credentials)?,
                refresh_token_encrypted: claimed
                    .refresh_token_encrypted
                    .clone()
                    .ok_or_else(missing_credentials)?,
                access_token_expires_at: claimed
                    .access_token_expires_at
                    .ok_or_else(missing_credentials)?,
            })
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    async fn mark_reconnect(
        &self,
        id: Uuid,
        claimed_generation: i64,
    ) -> SlackProvisioningResult<()> {
        self.db
            .mark_org_slack_reconnect_required(id, claimed_generation)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    /// Refresh a manager app grant. Slack issues it as a rotating user token,
    /// refreshed through `oauth.v2.access` with the manager app's client pair
    /// rather than `tooling.tokens.rotate`.
    async fn refresh_manager_grant(
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

    async fn rotate(&self, refresh_token: &str) -> SlackProvisioningResult<IssuedTokens> {
        // Slack reads `tooling.tokens.rotate` arguments only from a form body:
        // a JSON body is ignored and the call fails with `invalid_arguments`.
        // The method authenticates with the refresh token alone.
        let request = self
            .client
            .post(format!("{}/tooling.tokens.rotate", self.api_base))
            .form(&[("refresh_token", refresh_token)]);
        let response = self.send(request).await?;
        let rotated: RotateResponse = serde_json::from_value(response)
            .map_err(|_| malformed_response("tooling.tokens.rotate"))?;
        if rotated.token.is_empty()
            || rotated.refresh_token.is_empty()
            || rotated.exp <= chrono::Utc::now().timestamp()
        {
            return Err(malformed_response("tooling.tokens.rotate"));
        }
        Ok(IssuedTokens {
            expires_at: chrono::DateTime::from_timestamp(rotated.exp, 0)
                .ok_or_else(|| malformed_response("tooling.tokens.rotate"))?,
            access_token: rotated.token,
            refresh_token: rotated.refresh_token,
            team_id: rotated.team_id,
        })
    }

    async fn post(
        &self,
        method: &str,
        access_token: Option<&str>,
        body: serde_json::Value,
    ) -> SlackProvisioningResult<serde_json::Value> {
        let mut request = self
            .client
            .post(format!("{}{}", self.api_base, method))
            .json(&body);
        if let Some(access_token) = access_token {
            request = request.bearer_auth(access_token);
        }
        self.send(request).await
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> SlackProvisioningResult<serde_json::Value> {
        let response = request.send().await.map_err(|error| {
            SlackProvisioningError::Unreachable(format!("request failed: {error}"))
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SlackProvisioningError::Rejected("ratelimited".into()));
        }
        let value: serde_json::Value = response.json().await.map_err(|error| {
            SlackProvisioningError::Unreachable(format!("malformed response: {error}"))
        })?;
        if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            return Ok(value);
        }
        if let Some(code) = value.get("error").and_then(serde_json::Value::as_str) {
            return Err(SlackProvisioningError::Rejected(code.to_string()));
        }
        Err(SlackProvisioningError::Unreachable(format!(
            "Slack returned HTTP {} without an error code",
            status.as_u16()
        )))
    }

    fn encrypt(&self, value: &str) -> SlackProvisioningResult<Vec<u8>> {
        self.encryption.encrypt_string(value).map_err(|error| {
            SlackProvisioningError::Unreachable(format!(
                "could not encrypt Slack credentials: {error}"
            ))
        })
    }

    fn decrypt_required(&self, value: Option<&[u8]>) -> SlackProvisioningResult<String> {
        self.encryption
            .decrypt_to_string(value.ok_or_else(missing_credentials)?)
            .map_err(|error| {
                SlackProvisioningError::Unreachable(format!(
                    "could not decrypt Slack credentials: {error}"
                ))
            })
    }
}

#[derive(Deserialize)]
struct RotateResponse {
    token: String,
    refresh_token: String,
    exp: i64,
    #[serde(default)]
    team_id: Option<String>,
}

/// A fresh access/refresh pair, whichever Slack method issued it.
struct IssuedTokens {
    access_token: String,
    refresh_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    team_id: Option<String>,
}

impl IssuedTokens {
    /// Read an OAuth token object (`access_token`, `refresh_token`,
    /// `expires_in`).
    ///
    /// A grant without a refresh token comes from a manager app with token
    /// rotation off. It is refused with its own code: the connection store
    /// requires an expiry, and a non-expiring token over every app in the
    /// workspace is not one to hold.
    fn from_oauth(value: &serde_json::Value, method: &str) -> SlackProvisioningResult<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let access_token = text("access_token").ok_or_else(|| malformed_response(method))?;
        let (Some(refresh_token), Some(expires_in)) = (
            text("refresh_token"),
            value.get("expires_in").and_then(serde_json::Value::as_i64),
        ) else {
            return Err(SlackProvisioningError::Rejected(
                "token_rotation_disabled".to_string(),
            ));
        };
        if expires_in <= 0 {
            return Err(malformed_response(method));
        }
        Ok(Self {
            access_token,
            refresh_token,
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_in),
            team_id: None,
        })
    }
}

#[derive(Deserialize)]
struct CreateAppResponse {
    app_id: String,
    credentials: CreateAppCredentials,
}

#[derive(Deserialize)]
struct CreateAppCredentials {
    client_id: String,
    client_secret: String,
    signing_secret: String,
}

#[async_trait]
impl SlackAppProvisioner for SlackApiProvisioner {
    async fn update_branding(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        app_id: &str,
        name: &str,
        description: Option<&str>,
    ) -> SlackProvisioningResult<()> {
        let row = self.connection_for(org_id, team_id).await?;
        let exported = self
            .manifest_call(
                row,
                "/apps.manifest.export",
                serde_json::json!({"app_id":app_id}),
            )
            .await?;
        let mut manifest = exported
            .get("manifest")
            .filter(|m| m.is_object())
            .cloned()
            .ok_or_else(|| malformed_response("apps.manifest.export"))?;
        branding::patch(&mut manifest, name, description)?;
        let manifest = serde_json::to_string(&manifest)
            .map_err(|_| malformed_response("apps.manifest.export"))?;
        let row = self.connection_for(org_id, team_id).await?;
        self.manifest_call(
            row,
            "/apps.manifest.update",
            serde_json::json!({"app_id":app_id,"manifest":manifest}),
        )
        .await?;
        Ok(())
    }

    async fn set_app_icon(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        app_id: &str,
        png: Vec<u8>,
    ) -> SlackProvisioningResult<()> {
        let row = self.connection_for(org_id, team_id).await?;
        let (connection_org, connection_id) = (row.org_id, row.id);
        // `apps.icon.set` takes the image as a multipart file; the form is
        // rebuilt for the one retry after a token rotation.
        let form = |png: Vec<u8>| {
            let file = reqwest::multipart::Part::bytes(png)
                .file_name("avatar.png")
                .mime_str("image/png")
                .expect("static mime type parses");
            reqwest::multipart::Form::new()
                .text("app_id", app_id.to_string())
                .part("file", file)
        };
        let url = format!("{}/apps.icon.set", self.api_base);
        let access_token = self.token_for_request(row).await?;
        let first = self
            .send(
                self.client
                    .post(&url)
                    .bearer_auth(&access_token)
                    .multipart(form(png.clone())),
            )
            .await;
        match first {
            Err(SlackProvisioningError::Rejected(code)) if is_auth_error(&code) => {
                let row = self.connection_by_id(connection_org, connection_id).await?;
                let replacement = self.rotate_row(row).await?;
                self.send(
                    self.client
                        .post(&url)
                        .bearer_auth(&replacement)
                        .multipart(form(png)),
                )
                .await?;
                Ok(())
            }
            result => result.map(|_| ()),
        }
    }

    async fn set_app_icon_url(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        app_id: &str,
        icon_url: &str,
    ) -> SlackProvisioningResult<()> {
        let row = self.connection_for(org_id, team_id).await?;
        self.manifest_call(
            row,
            "/apps.icon.set",
            serde_json::json!({"app_id": app_id, "url": icon_url}),
        )
        .await?;
        Ok(())
    }

    async fn update_permissions(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        app_id: &str,
        scopes: &[&str],
    ) -> SlackProvisioningResult<()> {
        let row = self.connection_for(org_id, team_id).await?;
        let exported = self
            .manifest_call(
                row,
                "/apps.manifest.export",
                serde_json::json!({"app_id":app_id}),
            )
            .await?;
        let mut manifest = exported
            .get("manifest")
            .filter(|m| m.is_object())
            .cloned()
            .ok_or_else(|| malformed_response("apps.manifest.export"))?;
        if manifest.get("oauth_config").is_some_and(|v| !v.is_object())
            || manifest
                .pointer("/oauth_config/scopes")
                .is_some_and(|v| !v.is_object())
            || manifest
                .pointer("/oauth_config/scopes/bot")
                .is_some_and(|v| !v.is_array())
        {
            return Err(malformed_response("apps.manifest.export"));
        }
        let mut required = manifest
            .pointer("/oauth_config/scopes/bot")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for scope in scopes {
            let value = serde_json::Value::String((*scope).into());
            if !required.contains(&value) {
                required.push(value);
            }
        }
        // The update API replaces the manifest. Preserve the exported configuration
        // and add scopes only, including settings changed in Slack itself.
        manifest["oauth_config"]["scopes"]["bot"] = serde_json::Value::Array(required);
        let manifest = serde_json::to_string(&manifest)
            .map_err(|_| malformed_response("apps.manifest.export"))?;
        let row = self.connection_for(org_id, team_id).await?;
        self.manifest_call(
            row,
            "/apps.manifest.update",
            serde_json::json!({"app_id":app_id,"manifest":manifest}),
        )
        .await?;
        Ok(())
    }

    async fn create_app(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        manifest_yaml: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        let row = self.connection_for(org_id, team_id).await?;
        // Slack's manifest API requires a JSON-encoded string. The UI's
        // copy-paste flow accepts YAML, but sending that YAML to this API
        // fails with invalid_manifest even for an otherwise valid manifest.
        let manifest: serde_json::Value = serde_yaml::from_str(manifest_yaml).map_err(|_| {
            SlackProvisioningError::Unreachable("Could not encode Slack app manifest".to_string())
        })?;
        let manifest_json = serde_json::to_string(&manifest).map_err(|_| {
            SlackProvisioningError::Unreachable("Could not encode Slack app manifest".to_string())
        })?;
        let response = self
            .manifest_call(
                row,
                "/apps.manifest.create",
                serde_json::json!({"manifest": manifest_json}),
            )
            .await?;
        let response: CreateAppResponse = serde_json::from_value(response)
            .map_err(|_| malformed_response("apps.manifest.create"))?;
        Ok(SlackAppCredentials {
            app_id: response.app_id,
            client_id: response.credentials.client_id,
            client_secret: response.credentials.client_secret,
            signing_secret: response.credentials.signing_secret,
        })
    }

    async fn delete_app(
        &self,
        org_id: i64,
        team_id: Option<&str>,
        app_id: &str,
    ) -> SlackProvisioningResult<()> {
        let row = self.connection_for(org_id, team_id).await?;
        self.manifest_call(
            row,
            "/apps.manifest.delete",
            serde_json::json!({"app_id": app_id}),
        )
        .await?;
        Ok(())
    }

    /// Connected if any workspace is usable; reconnect required only if some
    /// workspace needs it and none is usable, so one stale workspace does not
    /// block agents bound for a healthy one.
    async fn connection_status(
        &self,
        org_id: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        let rows = self.list_connections(org_id).await?;
        let connected = rows
            .iter()
            .any(|row| matches!(row.state.as_str(), "connected" | "rotating"));
        Ok(SlackProvisioningConnectionStatus {
            connected,
            reconnect_required: !connected
                && rows.iter().any(|row| row.state == "reconnect_required"),
        })
    }

    fn name(&self) -> &'static str {
        "OrgSlackAppProvisioner"
    }
}

fn usable(row: OrgSlackConnectionRow) -> SlackProvisioningResult<OrgSlackConnectionRow> {
    if row.state == "reconnect_required" {
        return Err(SlackProvisioningError::ReconnectRequired);
    }
    Ok(row)
}

fn malformed_response(method: &str) -> SlackProvisioningError {
    SlackProvisioningError::Unreachable(format!("Slack returned a malformed {method} response"))
}

fn missing_credentials() -> SlackProvisioningError {
    SlackProvisioningError::ReconnectRequired
}

fn storage_error(error: anyhow::Error) -> SlackProvisioningError {
    tracing::error!(%error, "Slack connection storage failed");
    SlackProvisioningError::Unreachable("Slack connection storage failed".to_string())
}

fn is_auth_error(code: &str) -> bool {
    AUTH_ERROR_CODES.contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .expect("test encryption"),
        )
    }

    pub(super) fn provisioner(server: &MockServer) -> (SlackApiProvisioner, Arc<StorageBackend>) {
        let db = Arc::new(StorageBackend::test_database());
        (
            SlackApiProvisioner::with_api_base(db.clone(), encryption(), &server.uri())
                .expect("provisioner"),
            db,
        )
    }

    pub(super) fn stored(
        org_id: i64,
        team_id: &str,
        expires_in: chrono::Duration,
    ) -> UpsertOrgSlackConnection {
        let encryption = encryption();
        UpsertOrgSlackConnection {
            org_id,
            team_id: team_id.to_string(),
            team_name: None,
            access_token_encrypted: encryption.encrypt_string("access").unwrap(),
            refresh_token_encrypted: encryption.encrypt_string("refresh").unwrap(),
            access_token_expires_at: chrono::Utc::now() + expires_in,
            grant_kind: SlackGrantKind::ConfigToken,
        }
    }

    async fn mount_rotate(server: &MockServer, team_id: &str) {
        Mock::given(method("POST"))
            .and(path("/tooling.tokens.rotate"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "token": "new-access",
                "refresh_token": "new-refresh",
                "team_id": team_id,
                "user_id": "U1",
                "exp": chrono::Utc::now().timestamp() + 3600
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn permission_update_preserves_exported_manifest_and_unions_scopes() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        let manifest = serde_json::json!({
            "display_information":{"name":"Customized bot"},
            "settings":{"event_subscriptions":{"request_url":"https://example.test/events"}},
            "oauth_config":{"redirect_urls":["https://example.test/callback"],"scopes":{"bot":["chat:write","custom:read"]}}
        });
        Mock::given(method("POST"))
            .and(path("/apps.manifest.export"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok":true,"manifest":manifest})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apps.manifest.update"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        provisioner
            .update_permissions(1, Some("T1"), "A1", &["chat:write", "reactions:write"])
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(body["app_id"], "A1");
        let updated: serde_json::Value =
            serde_json::from_str(body["manifest"].as_str().unwrap()).unwrap();
        assert_eq!(
            updated["display_information"],
            manifest["display_information"]
        );
        assert_eq!(updated["settings"], manifest["settings"]);
        assert_eq!(
            updated["oauth_config"]["redirect_urls"],
            manifest["oauth_config"]["redirect_urls"]
        );
        assert_eq!(
            updated["oauth_config"]["scopes"]["bot"],
            serde_json::json!(["chat:write", "custom:read", "reactions:write"])
        );
    }

    #[tokio::test]
    async fn app_icon_is_uploaded_as_a_multipart_file_with_the_workspace_token() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/apps.icon.set"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        provisioner
            .set_app_icon(1, Some("T1"), "A1", b"\x89PNG-bytes".to_vec())
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        let content_type = request
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.starts_with("multipart/form-data"),
            "{content_type}"
        );
        assert!(request.headers.get("authorization").is_some());
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"app_id\""));
        assert!(body.contains("A1"));
        assert!(body.contains("name=\"file\"; filename=\"avatar.png\""));
        assert!(body.contains("PNG-bytes"));
    }

    #[tokio::test]
    async fn app_icon_can_be_set_from_a_public_url() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/apps.icon.set"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        let icon_url = "https://api.example.com/v1/avatars/avatar_1/square-512.png";
        provisioner
            .set_app_icon_url(1, Some("T1"), "A1", icon_url)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        let content_type = request
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.starts_with("application/json"),
            "{content_type}"
        );
        assert!(request.headers.get("authorization").is_some());
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["app_id"], "A1");
        assert_eq!(body["url"], icon_url);
    }

    #[tokio::test]
    async fn malformed_export_fails_without_mutating_manifest() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/apps.manifest.export"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok":true,"manifest":{"oauth_config":"invalid"}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            provisioner
                .update_permissions(1, Some("T1"), "A1", &["reactions:write"])
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn connecting_records_the_workspace_and_never_stores_plaintext() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tooling.tokens.rotate"))
            // Slack ignores a JSON body here and answers `invalid_arguments`.
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string("refresh_token=input-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "token": "new-access",
                "refresh_token": "new-refresh",
                "team_id": "T0123",
                "user_id": "U1",
                "exp": chrono::Utc::now().timestamp() + 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": true, "team": "Acme", "team_id": "T0123"}),
                ),
            )
            .mount(&server)
            .await;
        let (provisioner, _db) = provisioner(&server);

        let row = provisioner.connect(41, "input-refresh").await.unwrap();

        assert_eq!(row.team_id.as_deref(), Some("T0123"));
        assert_eq!(row.team_name.as_deref(), Some("Acme"));
        assert!(
            !row.refresh_token_encrypted
                .unwrap()
                .windows(b"new-refresh".len())
                .any(|part| part == b"new-refresh")
        );
    }

    #[tokio::test]
    async fn a_workspace_name_slack_will_not_reveal_does_not_fail_the_connection() {
        let server = MockServer::start().await;
        mount_rotate(&server, "T0123").await;
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": false, "error": "not_allowed_token_type"}),
                ),
            )
            .mount(&server)
            .await;
        let (provisioner, _db) = provisioner(&server);

        let row = provisioner.connect(41, "input-refresh").await.unwrap();

        assert_eq!(row.team_id.as_deref(), Some("T0123"));
        assert_eq!(row.team_name, None);
    }

    #[tokio::test]
    async fn invalid_refresh_token_marks_the_claimed_generation_for_reconnect() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tooling.tokens.rotate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": false, "error": "invalid_refresh_token"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = provisioner(&server);
        let row = db
            .upsert_org_slack_connection(stored(41, "T1", -chrono::Duration::minutes(1)))
            .await
            .unwrap();

        assert!(matches!(
            provisioner.create_app(41, None, "{}").await,
            Err(SlackProvisioningError::ReconnectRequired)
        ));
        let row = db
            .get_org_slack_connection(41, row.id)
            .await
            .unwrap()
            .expect("connection retained");
        assert_eq!(row.state, "reconnect_required");
        assert!(row.access_token_encrypted.is_none());
        assert!(row.refresh_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn with_several_workspaces_the_caller_must_name_one() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        db.upsert_org_slack_connection(stored(41, "T2", chrono::Duration::hours(1)))
            .await
            .unwrap();

        assert!(matches!(
            provisioner.create_app(41, None, "{}").await,
            Err(SlackProvisioningError::WorkspaceRequired)
        ));
        assert!(matches!(
            provisioner.create_app(41, Some("T9"), "{}").await,
            Err(SlackProvisioningError::WorkspaceNotConnected)
        ));
    }

    #[tokio::test]
    async fn app_creation_encodes_the_generated_yaml_as_json_for_slack() {
        let server = MockServer::start().await;
        let manifest = crate::channels::slack::events::build_manifest_yaml(
            "Support Agent",
            "Support Agent",
            Some("Answers questions"),
            "https://example.com/api/v1/channels/test/slack/events",
            "https://example.com/api/v1/channels/test/slack/interactivity",
            "https://example.com/api/v1/channels/test/slack/oauth/callback",
            true,
            &[],
        );
        let expected: serde_json::Value = serde_yaml::from_str(&manifest).unwrap();
        Mock::given(method("POST"))
            .and(path("/apps.manifest.create"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "manifest": serde_json::to_string(&expected).unwrap()
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "app_id": "A1",
                "credentials": {
                    "client_id": "c1",
                    "client_secret": "s1",
                    "signing_secret": "g1"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();

        let created = provisioner
            .create_app(41, Some("T1"), &manifest)
            .await
            .unwrap();
        assert_eq!(created.app_id, "A1");
    }

    #[tokio::test]
    async fn an_app_is_created_with_the_named_workspaces_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apps.manifest.create"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer access-T2",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "app_id": "A2",
                "credentials": {
                    "client_id": "c2",
                    "client_secret": "s2",
                    "signing_secret": "g2"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = provisioner(&server);
        let encryption = encryption();
        for team in ["T1", "T2"] {
            db.upsert_org_slack_connection(UpsertOrgSlackConnection {
                access_token_encrypted: encryption
                    .encrypt_string(&format!("access-{team}"))
                    .unwrap(),
                ..stored(41, team, chrono::Duration::hours(1))
            })
            .await
            .unwrap();
        }

        let created = provisioner.create_app(41, Some("T2"), "{}").await.unwrap();

        assert_eq!(created.app_id, "A2");
    }

    #[tokio::test]
    async fn connection_status_is_scoped_to_the_requested_organization() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(41, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();

        assert!(provisioner.connection_status(41).await.unwrap().connected);
        assert!(!provisioner.connection_status(42).await.unwrap().connected);
    }

    fn manager_app() -> Option<SlackManagerApp> {
        Some(SlackManagerApp {
            client_id: "111.222".to_string(),
            client_secret: "manager-secret".to_string(),
        })
    }

    fn manager_provisioner(server: &MockServer) -> (SlackApiProvisioner, Arc<StorageBackend>) {
        let (provisioner, db) = provisioner(server);
        (provisioner.with_manager_app(manager_app()), db)
    }

    fn query_param(url: &str, name: &str) -> String {
        reqwest::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| panic!("{name} missing from {url}"))
    }

    fn manager_grant(scope: &str) -> serde_json::Value {
        serde_json::json!({
            "ok": true,
            "app_id": "AMANAGER",
            "authed_user": {
                "id": "U1",
                "scope": scope,
                "access_token": "xoxe.xoxp-grant",
                "refresh_token": "xoxe-1-grant-refresh",
                "expires_in": 43200,
                "token_type": "user"
            },
            "team": {"id": "T0123", "name": "Acme"}
        })
    }

    #[tokio::test]
    async fn manager_consent_asks_only_for_app_configuration_and_names_the_org() {
        let server = MockServer::start().await;
        let (provisioner, _db) = manager_provisioner(&server);

        let url = provisioner
            .manager_authorize_url(
                41,
                "https://api.example.com/v1/slack/workspaces/oauth/callback",
            )
            .unwrap();

        assert!(url.starts_with(SLACK_OAUTH_AUTHORIZE_URL));
        assert_eq!(query_param(&url, "client_id"), "111.222");
        assert_eq!(query_param(&url, "scope"), "");
        assert_eq!(
            query_param(&url, "user_scope"),
            "app_configurations:read,app_configurations:write"
        );
        assert_eq!(
            query_param(&url, "redirect_uri"),
            "https://api.example.com/v1/slack/workspaces/oauth/callback"
        );
        let state = query_param(&url, "state");
        assert_eq!(provisioner.open_manager_state(&state), Some(41));
    }

    #[tokio::test]
    async fn a_forged_altered_foreign_or_expired_connect_state_names_no_org() {
        let server = MockServer::start().await;
        let (provisioner, _db) = manager_provisioner(&server);
        let seal = |state: &ManagerConnectState| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                encryption()
                    .encrypt(&serde_json::to_vec(state).unwrap())
                    .unwrap(),
            )
        };
        let valid = seal(&ManagerConnectState {
            purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
            org_id: 41,
            expires_at: chrono::Utc::now().timestamp() + 60,
        });
        assert_eq!(provisioner.open_manager_state(&valid), Some(41));

        let mut altered = valid.into_bytes();
        let last = altered.len() - 2;
        altered[last] = if altered[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            provisioner.open_manager_state(&String::from_utf8(altered).unwrap()),
            None
        );
        assert_eq!(provisioner.open_manager_state("not-a-state"), None);
        assert_eq!(
            provisioner.open_manager_state(&seal(&ManagerConnectState {
                purpose: "github_setup".to_string(),
                org_id: 41,
                expires_at: chrono::Utc::now().timestamp() + 60,
            })),
            None
        );
        assert_eq!(
            provisioner.open_manager_state(&seal(&ManagerConnectState {
                purpose: MANAGER_CONNECT_STATE_PURPOSE.to_string(),
                org_id: 41,
                expires_at: chrono::Utc::now().timestamp() - 1,
            })),
            None
        );
    }

    #[tokio::test]
    async fn without_a_manager_app_there_is_no_consent_to_start() {
        let server = MockServer::start().await;
        let (provisioner, _db) = provisioner(&server);
        assert!(!provisioner.manager_app_configured());
        assert!(matches!(
            provisioner.manager_authorize_url(41, "https://example.test/cb"),
            Err(SlackProvisioningError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn a_manager_grant_connects_the_workspace_it_was_granted_for() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string(
                "client_id=111.222&client_secret=manager-secret&code=consent-code&redirect_uri=https%3A%2F%2Fexample.test%2Fcb",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(manager_grant(
                "app_configurations:read,app_configurations:write",
            )))
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = manager_provisioner(&server);

        let row = provisioner
            .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
            .await
            .unwrap();

        assert_eq!(row.org_id, 41);
        assert_eq!(row.team_id.as_deref(), Some("T0123"));
        assert_eq!(row.team_name.as_deref(), Some("Acme"));
        assert_eq!(row.grant_kind(), SlackGrantKind::ManagerApp);
        assert_eq!(row.state, "connected");
        let stored = row.refresh_token_encrypted.clone().unwrap();
        assert!(
            !stored
                .windows(b"grant-refresh".len())
                .any(|part| part == b"grant-refresh")
        );
        assert!(
            row.access_token_expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::hours(11)
        );
        assert!(provisioner.connection_status(41).await.unwrap().connected);
        assert_eq!(db.list_org_slack_connections(41).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_grant_that_cannot_create_apps_is_refused_and_not_stored() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(manager_grant("app_configurations:read")),
            )
            .mount(&server)
            .await;
        let (provisioner, db) = manager_provisioner(&server);

        let result = provisioner
            .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
            .await;

        assert!(
            matches!(result, Err(SlackProvisioningError::Rejected(code)) if code == "missing_scope")
        );
        assert!(db.list_org_slack_connections(41).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_non_rotating_grant_is_refused_with_its_own_code() {
        let server = MockServer::start().await;
        let mut grant = manager_grant("app_configurations:read,app_configurations:write");
        grant["authed_user"]
            .as_object_mut()
            .unwrap()
            .retain(|key, _| key != "refresh_token" && key != "expires_in");
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(grant))
            .mount(&server)
            .await;
        let (provisioner, db) = manager_provisioner(&server);

        let result = provisioner
            .connect_with_manager_code(41, "consent-code", "https://example.test/cb")
            .await;

        assert!(
            matches!(result, Err(SlackProvisioningError::Rejected(code)) if code == "token_rotation_disabled")
        );
        assert!(db.list_org_slack_connections(41).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn slack_refusing_the_consent_code_surfaces_its_error_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": false, "error": "invalid_code"})),
            )
            .mount(&server)
            .await;
        let (provisioner, _db) = manager_provisioner(&server);

        assert!(matches!(
            provisioner
                .connect_with_manager_code(41, "stale", "https://example.test/cb")
                .await,
            Err(SlackProvisioningError::Rejected(code)) if code == "invalid_code"
        ));
    }

    fn stored_manager_grant(org_id: i64, expires_in: chrono::Duration) -> UpsertOrgSlackConnection {
        UpsertOrgSlackConnection {
            grant_kind: SlackGrantKind::ManagerApp,
            ..stored(org_id, "T1", expires_in)
        }
    }

    #[tokio::test]
    async fn an_expiring_manager_grant_refreshes_through_oauth_not_token_rotation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .and(body_string(
                "client_id=111.222&client_secret=manager-secret&grant_type=refresh_token&refresh_token=refresh",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "access_token": "refreshed-access",
                "refresh_token": "refreshed-refresh",
                "expires_in": 43200,
                "token_type": "user",
                "team": {"id": "T1"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/tooling.tokens.rotate"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apps.manifest.create"))
            .and(header("authorization", "Bearer refreshed-access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "app_id": "A1",
                "credentials": {"client_id": "c1", "client_secret": "s1", "signing_secret": "g1"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = manager_provisioner(&server);
        let row = db
            .upsert_org_slack_connection(stored_manager_grant(41, chrono::Duration::minutes(1)))
            .await
            .unwrap();

        let created = provisioner.create_app(41, None, "{}").await.unwrap();

        assert_eq!(created.app_id, "A1");
        let row = db
            .get_org_slack_connection(41, row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "connected");
        assert_eq!(row.grant_kind(), SlackGrantKind::ManagerApp);
        assert!(
            row.access_token_expires_at.unwrap() > chrono::Utc::now() + chrono::Duration::hours(11)
        );
    }

    #[tokio::test]
    async fn a_revoked_manager_grant_asks_for_reconnect() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth.v2.access"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"ok": false, "error": "invalid_refresh_token"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = manager_provisioner(&server);
        let row = db
            .upsert_org_slack_connection(stored_manager_grant(41, -chrono::Duration::minutes(1)))
            .await
            .unwrap();

        assert!(matches!(
            provisioner.create_app(41, None, "{}").await,
            Err(SlackProvisioningError::ReconnectRequired)
        ));
        let row = db
            .get_org_slack_connection(41, row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "reconnect_required");
        assert!(
            provisioner
                .connection_status(41)
                .await
                .unwrap()
                .reconnect_required
        );
    }

    #[tokio::test]
    async fn a_manager_grant_is_kept_when_the_deployment_drops_its_manager_app() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        let row = db
            .upsert_org_slack_connection(stored_manager_grant(41, -chrono::Duration::minutes(1)))
            .await
            .unwrap();

        assert!(matches!(
            provisioner.create_app(41, None, "{}").await,
            Err(SlackProvisioningError::Rejected(code)) if code == "manager_app_not_configured"
        ));
        let row = db
            .get_org_slack_connection(41, row.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "connected");
        assert!(row.refresh_token_encrypted.is_some());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
