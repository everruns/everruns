use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_platform::slack_provisioning::{
    SlackAppCredentials, SlackAppProvisioner, SlackProvisioningConnectionStatus,
    SlackProvisioningError, SlackProvisioningResult,
};
use serde::Deserialize;

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

    pub async fn connect(&self, org_id: i64, refresh_token: &str) -> SlackProvisioningResult<()> {
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(SlackProvisioningError::Rejected(
                "invalid_refresh_token".to_string(),
            ));
        }
        let rotated = self.rotate_with(None, refresh_token).await?;
        self.db
            .upsert_org_slack_connection(UpsertOrgSlackConnection {
                org_id,
                access_token_encrypted: self.encrypt(&rotated.token)?,
                refresh_token_encrypted: self.encrypt(&rotated.refresh_token)?,
                access_token_expires_at: rotated.expires_at()?,
            })
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    pub async fn test_connection(&self, org_id: i64) -> SlackProvisioningResult<()> {
        self.manifest_call(
            org_id,
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

    pub async fn clear_connection(&self, org_id: i64) -> SlackProvisioningResult<()> {
        self.db
            .delete_org_slack_connection(org_id)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    pub async fn rotate_due_connections(&self) -> anyhow::Result<()> {
        let due = self
            .db
            .list_due_org_slack_connections(chrono::Utc::now() + ROTATE_BEFORE_EXPIRY)
            .await?;
        for row in due {
            let org_id = row.org_id;
            if let Err(error) = self.rotate_row(row).await {
                tracing::warn!(org_id, %error, "Slack connection rotation failed");
            }
        }
        Ok(())
    }

    async fn manifest_call(
        &self,
        org_id: i64,
        method: &str,
        body: serde_json::Value,
    ) -> SlackProvisioningResult<serde_json::Value> {
        let access_token = self.token_for_request(org_id).await?;
        match self.post(method, Some(&access_token), body.clone()).await {
            Err(SlackProvisioningError::Rejected(code)) if is_auth_error(&code) => {
                let row = self.connection(org_id).await?;
                let replacement = self.rotate_row(row).await?;
                self.post(method, Some(&replacement), body).await
            }
            result => result,
        }
    }

    async fn token_for_request(&self, org_id: i64) -> SlackProvisioningResult<String> {
        let row = self.connection(org_id).await?;
        if row
            .access_token_expires_at
            .is_some_and(|expiry| expiry > chrono::Utc::now() + ROTATE_BEFORE_EXPIRY)
        {
            return self.decrypt_required(row.access_token_encrypted.as_deref());
        }
        self.rotate_row(row).await
    }

    async fn connection(&self, org_id: i64) -> SlackProvisioningResult<OrgSlackConnectionRow> {
        let row = self
            .db
            .get_org_slack_connection(org_id)
            .await
            .map_err(storage_error)?
            .ok_or(SlackProvisioningError::OrgNotConnected)?;
        if row.state == "reconnect_required" {
            return Err(SlackProvisioningError::ReconnectRequired);
        }
        Ok(row)
    }

    async fn rotate_row(&self, row: OrgSlackConnectionRow) -> SlackProvisioningResult<String> {
        let original_generation = row.token_generation;
        let Some(claimed) = self
            .db
            .claim_org_slack_connection_rotation(row.org_id, original_generation)
            .await
            .map_err(storage_error)?
        else {
            return self
                .wait_for_rotation(row.org_id, original_generation)
                .await;
        };
        let access_token = self.decrypt_required(claimed.access_token_encrypted.as_deref())?;
        let refresh_token = self.decrypt_required(claimed.refresh_token_encrypted.as_deref())?;
        let rotated = match self.rotate_with(Some(&access_token), &refresh_token).await {
            Ok(rotated) => rotated,
            Err(SlackProvisioningError::Rejected(code)) if code == "invalid_refresh_token" => {
                self.mark_reconnect(claimed.org_id, claimed.token_generation)
                    .await?;
                return Err(SlackProvisioningError::ReconnectRequired);
            }
            Err(SlackProvisioningError::Unreachable(_)) => {
                self.mark_reconnect(claimed.org_id, claimed.token_generation)
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
                org_id: claimed.org_id,
                expected_generation: claimed.token_generation,
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
        original_generation: i64,
    ) -> SlackProvisioningResult<String> {
        for _ in 0..ROTATION_WAIT_ATTEMPTS {
            tokio::time::sleep(ROTATION_WAIT).await;
            let row = self.connection(org_id).await?;
            if row.token_generation >= original_generation + 2 {
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
                org_id: claimed.org_id,
                expected_generation: claimed.token_generation,
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
        org_id: i64,
        claimed_generation: i64,
    ) -> SlackProvisioningResult<()> {
        self.db
            .mark_org_slack_reconnect_required(org_id, claimed_generation)
            .await
            .map_err(storage_error)?;
        Ok(())
    }

    async fn rotate_with(
        &self,
        access_token: Option<&str>,
        refresh_token: &str,
    ) -> SlackProvisioningResult<RotateResponse> {
        let response = self
            .post(
                "/tooling.tokens.rotate",
                access_token,
                serde_json::json!({"refresh_token": refresh_token}),
            )
            .await?;
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
        manifest_yaml: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        let response = self
            .manifest_call(
                org_id,
                "/apps.manifest.create",
                serde_json::json!({"manifest": manifest_yaml}),
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

    async fn delete_app(&self, org_id: i64, app_id: &str) -> SlackProvisioningResult<()> {
        self.manifest_call(
            org_id,
            "/apps.manifest.delete",
            serde_json::json!({"app_id": app_id}),
        )
        .await?;
        Ok(())
    }

    async fn connection_status(
        &self,
        org_id: i64,
    ) -> SlackProvisioningResult<SlackProvisioningConnectionStatus> {
        let row = self
            .db
            .get_org_slack_connection(org_id)
            .await
            .map_err(storage_error)?;
        Ok(SlackProvisioningConnectionStatus {
            connected: row.as_ref().is_some_and(|row| row.state == "connected"),
            reconnect_required: row
                .as_ref()
                .is_some_and(|row| row.state == "reconnect_required"),
        })
    }

    fn name(&self) -> &'static str {
        "OrgSlackAppProvisioner"
    }
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
    use wiremock::matchers::{body_json, method, path};
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

    #[tokio::test]
    async fn connecting_rotates_before_persisting_and_never_stores_plaintext() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tooling.tokens.rotate"))
            .and(body_json(
                serde_json::json!({"refresh_token": "input-refresh"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "token": "new-access",
                "refresh_token": "new-refresh",
                "exp": chrono::Utc::now().timestamp() + 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (provisioner, db) = provisioner(&server);

        provisioner.connect(41, "input-refresh").await.unwrap();

        let row = db
            .get_org_slack_connection(41)
            .await
            .unwrap()
            .expect("stored connection");
        assert!(
            !row.refresh_token_encrypted
                .unwrap()
                .windows(b"new-refresh".len())
                .any(|part| part == b"new-refresh")
        );
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
        let encryption = encryption();
        let db = Arc::new(StorageBackend::in_memory());
        db.upsert_org_slack_connection(UpsertOrgSlackConnection {
            org_id: 41,
            access_token_encrypted: encryption.encrypt_string("expired-access").unwrap(),
            refresh_token_encrypted: encryption.encrypt_string("invalid-refresh").unwrap(),
            access_token_expires_at: chrono::Utc::now() - chrono::Duration::minutes(1),
        })
        .await
        .unwrap();
        let provisioner =
            SlackApiProvisioner::with_api_base(db.clone(), encryption, &server.uri()).unwrap();

        assert!(matches!(
            provisioner.create_app(41, "{}").await,
            Err(SlackProvisioningError::ReconnectRequired)
        ));
        let row = db
            .get_org_slack_connection(41)
            .await
            .unwrap()
            .expect("connection retained");
        assert_eq!(row.state, "reconnect_required");
        assert!(row.access_token_encrypted.is_none());
        assert!(row.refresh_token_encrypted.is_none());
    }

    #[tokio::test]
    async fn connection_status_is_scoped_to_the_requested_organization() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        let encryption = encryption();
        db.upsert_org_slack_connection(UpsertOrgSlackConnection {
            org_id: 41,
            access_token_encrypted: encryption.encrypt_string("access").unwrap(),
            refresh_token_encrypted: encryption.encrypt_string("refresh").unwrap(),
            access_token_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();

        assert!(provisioner.connection_status(41).await.unwrap().connected);
        assert!(!provisioner.connection_status(42).await.unwrap().connected);
    }
}
