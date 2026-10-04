use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_platform::slack_provisioning::{
    SlackAppCredentials, SlackAppProvisioner, SlackProvisioningConnectionStatus,
    SlackProvisioningError, SlackProvisioningResult,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::storage::{
    EncryptionService, OrgSlackConnectionRow, RotateOrgSlackConnection, StorageBackend,
    UpsertOrgSlackConnection,
};

const SLACK_API_BASE: &str = "https://slack.com/api";
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

pub struct SlackProvisioningSetup {
    pub provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    pub connection_manager: Option<Arc<SlackApiProvisioner>>,
}

pub fn configure(
    supervisor: &mut crate::supervised_task::TaskSupervisor,
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
    let manager = Arc::new(SlackApiProvisioner::new(db, encryption)?);
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

    fn with_api_base(
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
        })
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
        let team_name = self.workspace_name(&rotated.token).await;
        self.db
            .upsert_org_slack_connection(UpsertOrgSlackConnection {
                org_id,
                team_id,
                team_name,
                access_token_encrypted: self.encrypt(&rotated.token)?,
                refresh_token_encrypted: self.encrypt(&rotated.refresh_token)?,
                access_token_expires_at: rotated.expires_at()?,
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
        let rotated = match self.rotate(&refresh_token).await {
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
                access_token_encrypted: self.encrypt(&rotated.token)?,
                refresh_token_encrypted: self.encrypt(&rotated.refresh_token)?,
                access_token_expires_at: rotated.expires_at()?,
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

    async fn rotate(&self, refresh_token: &str) -> SlackProvisioningResult<RotateResponse> {
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
        Ok(rotated)
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

impl RotateResponse {
    fn expires_at(&self) -> SlackProvisioningResult<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self.exp, 0)
            .ok_or_else(|| malformed_response("tooling.tokens.rotate"))
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

    fn provisioner(server: &MockServer) -> (SlackApiProvisioner, Arc<StorageBackend>) {
        let db = Arc::new(StorageBackend::in_memory());
        (
            SlackApiProvisioner::with_api_base(db.clone(), encryption(), &server.uri())
                .expect("provisioner"),
            db,
        )
    }

    fn stored(
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
        let manifest = crate::api::slack_events::build_manifest_yaml(
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
}
