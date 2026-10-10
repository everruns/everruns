//! API-layer implementation of the OAuth catalog preset connection check
//! (`domains::mcp_servers::connection_check`).
//!
//! It runs exactly the discovery and dynamic client registration the first
//! sign-in runs ([`ensure_mcp_oauth_registration_via`]), through the same
//! egress, so "Ready to connect" predicts the login. A successful
//! registration is kept, so the first sign-in skips it. Failures are
//! classified with the connect-error codes the login redirect uses
//! ([`ConnectErrorCode`]); the detail stays in the server log.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use uuid::Uuid;

use super::AppState;
use super::connect_errors::ConnectErrorCode;
use super::mcp_oauth::{discover_oauth_server_metadata, ensure_mcp_oauth_registration_via};
use crate::domains::mcp_servers::connection_check::McpOAuthConnectionChecker;
use crate::domains::mcp_servers::record::{McpConnectionCheck, McpConnectionCheckStatus};
use crate::domains::mcp_servers::{McpServerService, McpServerSettings};
use crate::kernel_imports::{McpServerAuthMode, mcp_oauth_provider_id_for_uuid};

/// Runs the connection check with the OAuth connect flow's state.
pub struct OAuthConnectionChecker {
    state: AppState,
}

impl OAuthConnectionChecker {
    pub fn shared(state: AppState) -> Arc<dyn McpOAuthConnectionChecker> {
        Arc::new(Self { state })
    }
}

#[async_trait]
impl McpOAuthConnectionChecker for OAuthConnectionChecker {
    async fn check_and_record(&self, org_id: i64, server_id: Uuid) -> McpConnectionCheck {
        let Some(row) = self.load_oauth_preset(org_id, server_id).await else {
            return McpConnectionCheck::not_checked();
        };
        let settings = McpServerService::settings_from_row(&row);
        let egress = HostRecordingEgress::new(self.state.mcp_service.egress_service());
        let outcome = probe(&self.state, &egress, &row, settings).await;
        let (check, registered) = match outcome {
            Ok(registered) => (
                McpConnectionCheck {
                    status: McpConnectionCheckStatus::Ready,
                    checked_at: Some(chrono::Utc::now()),
                    host: None,
                    reason: None,
                },
                Some(registered),
            ),
            Err(error) => {
                let check = failed_check(&error, egress.last_host());
                tracing::warn!(
                    mcp_server_id = %server_id,
                    status = %error.0,
                    error = %error.1,
                    connection_check = ?check.status,
                    "MCP catalog connection check failed"
                );
                (check, None)
            }
        };
        self.record(org_id, &row, check.clone(), registered).await;
        check
    }
}

impl OAuthConnectionChecker {
    async fn load_oauth_preset(
        &self,
        org_id: i64,
        server_id: Uuid,
    ) -> Option<crate::storage::McpServerRow> {
        let row = self
            .state
            .db
            .get_mcp_server(org_id, server_id)
            .await
            .inspect_err(|error| tracing::warn!(%error, "MCP connection check: load failed"))
            .ok()
            .flatten()?;
        let checkable = matches!(row.status.as_str(), "active" | "disabled")
            && McpServerService::settings_from_row(&row).auth_mode == McpServerAuthMode::OAuth;
        checkable.then_some(row)
    }

    /// Persist onto the preset as it is now, not as it was when the check
    /// started: an edit made meanwhile is kept, and a preset that stopped
    /// being this OAuth server records nothing. A registration another
    /// sign-in stored meanwhile wins over this one, because a login already
    /// in flight is bound to that client id.
    async fn record(
        &self,
        org_id: i64,
        checked: &crate::storage::McpServerRow,
        check: McpConnectionCheck,
        registered: Option<McpServerSettings>,
    ) {
        let Some(current) = self.load_oauth_preset(org_id, checked.id.uuid()).await else {
            return;
        };
        if current.url != checked.url {
            return;
        }
        let mut settings = McpServerService::settings_from_row(&current);
        let current_client = settings
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.client_id.as_ref());
        if current_client.is_none()
            && let Some(registered) = registered
        {
            settings.oauth = registered.oauth;
        }
        settings.connection_check = Some(check);
        let value = match serde_json::to_value(&settings) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "MCP connection check: settings did not serialize");
                return;
            }
        };
        if let Err(error) = self
            .state
            .db
            .update_mcp_server_settings_any_owner(org_id, current.id.uuid(), value)
            .await
        {
            tracing::warn!(%error, "MCP connection check: result not recorded");
        }
    }
}

/// The check itself. An unregistered preset gets the full discovery and
/// registration. A registered one already has its client, so the check only
/// proves its sign-in service still answers: its authorization-server
/// metadata when the issuer is known, otherwise (manually configured
/// endpoints) there is nothing to fetch and the stored setup stands.
async fn probe(
    state: &AppState,
    egress: &dyn EgressService,
    row: &crate::storage::McpServerRow,
    settings: McpServerSettings,
) -> Result<McpServerSettings, (StatusCode, String)> {
    let oauth = settings.oauth.as_ref();
    let registered = oauth.is_some_and(|oauth| {
        oauth.client_id.is_some()
            && oauth.authorization_endpoint.is_some()
            && oauth.token_endpoint.is_some()
    });
    if registered {
        if let Some(issuer) = oauth.and_then(|oauth| oauth.issuer.clone()) {
            discover_oauth_server_metadata(egress, &issuer).await?;
        }
        return Ok(settings);
    }
    let provider = mcp_oauth_provider_id_for_uuid(row.id.uuid());
    let (_, _, settings) =
        ensure_mcp_oauth_registration_via(state, egress, row, settings, &provider).await?;
    Ok(settings)
}

/// Map a failed check to its recorded status. The reason is one of a fixed
/// set of sentences: provider bodies and upstream messages never reach it.
fn failed_check(error: &(StatusCode, String), host: Option<String>) -> McpConnectionCheck {
    let (status, reason) = match ConnectErrorCode::classify(error) {
        ConnectErrorCode::BlockedByNetworkPolicy => (
            McpConnectionCheckStatus::BlockedByNetworkPolicy,
            "This host is not on the allowed network list",
        ),
        ConnectErrorCode::ProviderUnreachable | ConnectErrorCode::ProviderRefused => (
            McpConnectionCheckStatus::Unreachable,
            "The server's sign-in service did not answer correctly",
        ),
        ConnectErrorCode::Failed if error.1.contains("registration_endpoint") => (
            McpConnectionCheckStatus::Failed,
            "The sign-in service does not support automatic client registration",
        ),
        ConnectErrorCode::Failed => (
            McpConnectionCheckStatus::Failed,
            "The server's sign-in setup could not be completed",
        ),
    };
    // The host names where a request went; it explains nothing for a
    // setup that failed before or apart from the network.
    let host = host.filter(|_| status != McpConnectionCheckStatus::Failed);
    McpConnectionCheck {
        status,
        checked_at: Some(chrono::Utc::now()),
        host,
        reason: Some(reason.to_string()),
    }
}

/// Delegating egress that remembers the host of the last request, so a
/// failure names the host that refused (the issuer can differ from the MCP
/// server). Only the host is kept: no path, query, or body.
struct HostRecordingEgress {
    inner: Arc<dyn EgressService>,
    last_host: Mutex<Option<String>>,
}

impl HostRecordingEgress {
    fn new(inner: Arc<dyn EgressService>) -> Self {
        Self {
            inner,
            last_host: Mutex::new(None),
        }
    }

    fn remember(&self, url: &str) {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string));
        *self.last_host.lock().unwrap_or_else(|e| e.into_inner()) = host;
    }

    fn last_host(&self) -> Option<String> {
        self.last_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[async_trait]
impl EgressService for HostRecordingEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.remember(&request.url);
        self.inner.send(request).await
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        self.remember(&request.url);
        self.inner.send_stream(request).await
    }
}
