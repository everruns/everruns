//! ChatGPT loopback login and protected self-hosted credential transfer.
use super::{
    common::{ApiResult, ErrorResponse},
    providers::AppState,
};
use crate::domains::user_connections::chatgpt::ATTEMPTS;
pub(crate) use crate::domains::user_connections::chatgpt::cancel_attempt;
use crate::{auth::ResolvedOrg, domains::user_connections::chatgpt, storage::models::ProviderRow};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use everruns_contracts::typed_id::ProviderId;
use everruns_drivers::chatgpt::auth::TokenStore;
use everruns_drivers::chatgpt::{ChatGptRegistration, CodexAuth, login::LoginAttempt, oauth};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

/// Personal ChatGPT connection state, without access or refresh credentials.
#[derive(Serialize, utoipa::ToSchema)]
pub struct ConnectionStatus {
    /// Connection state: disconnected, connecting, connected, scope_required, or error.
    #[schema(example = "disconnected")]
    pub status: String,
    /// Verified account email, present after a successful sign-in.
    #[schema(example = "user@example.com")]
    pub email: Option<String>,
    /// Installation identifier to pass unchanged to the local login helper.
    #[schema(example = "urn:uuid:00000000-0000-0000-0000-000000000001")]
    pub host_id: String,
    /// Safe connection error or reconnect guidance, when applicable.
    #[schema(example = "ChatGPT sign-in did not complete. Retry sign-in.")]
    pub error: Option<String>,
    /// Non-secret host registration, containing the issuing client and verified subject.
    #[schema(value_type = Option<Object>, example = json!({"client_id":"app_example","subject":"example-subject","email":"user@example.com"}))]
    pub registration: Option<ChatGptRegistration>,
    /// Everruns user who owns and may manage this personal connection.
    #[schema(example = "00000000-0000-0000-0000-000000000001")]
    pub owner_user_id: Option<String>,
}
/// Browser destination for a newly created, bounded loopback login attempt.
#[derive(Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    /// Open this returned URL unchanged; it contains the attempt's PKCE, state, and nonce.
    #[schema(example = "https://auth.openai.com/oauth/authorize?client_id=app_example")]
    pub authorize_url: String,
}
/// Private credential handoff emitted by the local login helper for this installation.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct ImportedConnection {
    /// Credential document emitted by the local helper; accepted only after ID-token validation.
    #[schema(value_type = Object, example = json!({"access_token":"LOCAL_HELPER_ACCESS_TOKEN","refresh_token":"LOCAL_HELPER_REFRESH_TOKEN","client_id":"app_example","open_source":{"id_token":"LOCAL_HELPER_ID_TOKEN","scopes":[oauth::PLAN_SCOPE],"subject":"example-subject"}}))]
    pub auth: CodexAuth,
    /// Nonce from the same helper document, bound to its signed ID token.
    #[schema(example = "nonce-from-the-local-helper")]
    pub nonce: String,
    /// Installation identifier from the setup document downloaded for this provider.
    #[schema(example = "urn:uuid:00000000-0000-0000-0000-000000000001")]
    pub host_id: String,
}
async fn row(
    state: &AppState,
    org: &ResolvedOrg,
    id: &str,
) -> Result<ProviderRow, (StatusCode, Json<ErrorResponse>)> {
    let id: ProviderId = id
        .parse()
        .map_err(|_| ErrorResponse::not_found("Provider"))?;
    let row = state
        .db
        .get_provider(org.org_id, id.uuid())
        .await
        .map_err(|_| ErrorResponse::internal_error())?
        .filter(|r| {
            r.provider_type == "chatgpt"
                && chatgpt::visible(&r.settings, &org.into())
                && r.settings.get("chatgpt").is_some()
        })
        .ok_or_else(|| ErrorResponse::not_found("Provider"))?;
    Ok(row)
}
/// Read the owner's personal ChatGPT connection.
///
/// Returns connection state and non-secret installation metadata. Another user's
/// provider is hidden even from organization administrators.
#[utoipa::path(
    get,
    path = "/v1/providers/{provider_id}/chatgpt",
    params(("provider_id" = String, Path, description = "Personal provider ID")),
    responses(
        (status = 200, description = "Personal connection status", body = ConnectionStatus),
        (status = 404, description = "Provider not found or owned by another user"),
        (status = 400, description = "Invalid connection request"),
        (status = 502, description = "Provider exchange or revocation failed")
    ),
    tag = "providers"
)]
pub async fn status(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(id): Path<String>,
) -> ApiResult<ConnectionStatus> {
    let row = row(&state, &org, &id).await?;
    let value = &row.settings["chatgpt"];
    Ok(Json(ConnectionStatus {
        owner_user_id: value["owner_user_id"].as_str().map(str::to_owned),
        status: value["status"].as_str().unwrap_or("disconnected").into(),
        email: value["email"].as_str().map(str::to_owned),
        host_id: value["host_id"].as_str().unwrap_or_default().into(),
        error: value["error"].as_str().map(str::to_owned),
        registration: value
            .get("registration")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok()),
    }))
}
/// Start a personal ChatGPT sign-in.
///
/// Creates a bounded loopback callback listener and returns the authorization URL.
/// Requires deployment enablement, organization opt-in, encryption, and exact ownership.
#[utoipa::path(
    post,
    path = "/v1/providers/{provider_id}/chatgpt/login",
    params(("provider_id" = String, Path, description = "Personal provider ID")),
    responses(
        (status = 200, description = "Personal connection login", body = LoginResponse),
        (status = 404, description = "Provider not found or owned by another user"),
        (status = 400, description = "Invalid connection request"),
        (status = 502, description = "Provider exchange or revocation failed")
    ),
    tag = "providers"
)]
pub async fn login(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(id): Path<String>,
) -> ApiResult<LoginResponse> {
    if !org.feature_flags.chatgpt_plan {
        return Err(ErrorResponse::feature_not_enabled("chatgpt_plan"));
    }
    let mut row = row(&state, &org, &id).await?;
    let store = chatgpt::store(
        state.db.clone(),
        state.encryption.clone(),
        org.org_id,
        row.id,
    )
    .map_err(|_| {
        ErrorResponse::new("Encryption is required for ChatGPT sign-in.")
            .into_response(StatusCode::SERVICE_UNAVAILABLE)
    })?;
    let mut attempts = ATTEMPTS
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .await;
    let key = (org.org_id, row.id);
    // THREAT[TM-DOS-046]: Bound process-local callback listeners and pending tasks.
    if attempts.len() >= 32 && !attempts.contains_key(&key) {
        return Err(
            ErrorResponse::new("Too many ChatGPT sign-ins are pending. Retry shortly.")
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }
    if attempts.contains_key(&key) {
        return Err(ErrorResponse::conflict(
            "A ChatGPT sign-in is already pending. Wait for it to finish.",
        ));
    }
    let registration: Option<ChatGptRegistration> = row.settings["chatgpt"]
        .get("registration")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok());
    let saved = store
        .load()
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    let hint = saved
        .as_ref()
        .and_then(|a| a.open_source.as_ref())
        .and_then(|g| g.id_token.as_deref());
    let consent = saved
        .as_ref()
        .and_then(|a| a.open_source.as_ref())
        .is_some_and(|g| !oauth::grants_plan_usage(&g.scopes));
    let attempt = LoginAttempt::start(
        oauth::Endpoints::production(),
        "Everruns",
        row.settings["chatgpt"]["host_id"]
            .as_str()
            .unwrap_or_default(),
        registration,
        hint,
        consent,
    )
    .await
    .map_err(|_| {
        ErrorResponse::new("Cannot start the local ChatGPT callback listener.")
            .into_response(StatusCode::SERVICE_UNAVAILABLE)
    })?;
    let url = attempt.authorize_url.clone();
    let lease = store
        .lock()
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    let mut settings = state
        .db
        .get_provider(org.org_id, row.id.uuid())
        .await
        .map_err(|_| ErrorResponse::internal_error())?
        .ok_or_else(|| ErrorResponse::not_found("Provider"))?
        .settings;
    settings["chatgpt"]["generation"] = json!(uuid::Uuid::new_v4());
    settings["chatgpt"]["status"] = "connecting".into();
    settings["chatgpt"]["error"] = serde_json::Value::Null;
    state
        .db
        .update_provider(
            org.org_id,
            row.id.uuid(),
            crate::storage::models::UpdateProvider {
                settings: Some(settings.clone()),
                ..chatgpt::empty_update()
            },
        )
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    row.settings = settings;
    drop(lease);
    let task = tokio::spawn(async move {
        let result = match attempt.finish().await {
            Ok(auth) => save_connection(&state, &row, auth).await,
            Err(error) => Err(error),
        };
        if result.is_err() {
            let expected = row.settings["chatgpt"]["generation"].clone();
            if let Ok(lease_store) = chatgpt::store(
                state.db.clone(),
                state.encryption.clone(),
                row.org_id,
                row.id,
            ) && let Ok(_lease) = lease_store.lock().await
                && let Ok(Some(row)) = state.db.get_provider(row.org_id, row.id.uuid()).await
                && row.settings["chatgpt"]["generation"] == expected
            {
                let mut settings = row.settings;
                settings["chatgpt"]["status"] = "error".into();
                settings["chatgpt"]["error"]="ChatGPT sign-in did not complete. Retry sign-in, or use a local login helper for a remote installation.".into();
                let _ = state
                    .db
                    .update_provider(
                        row.org_id,
                        row.id.uuid(),
                        crate::storage::models::UpdateProvider {
                            settings: Some(settings),
                            ..chatgpt::empty_update()
                        },
                    )
                    .await;
            }
        } else {
            // Personal models must never bootstrap an organization-wide default.
            let _ = state
                .sync_service
                .sync_provider(row.org_id, row.id.uuid())
                .await;
        }
        if let Some(attempts) = ATTEMPTS.get() {
            attempts.lock().await.remove(&key);
        }
    });
    attempts.insert(key, task.abort_handle());
    Ok(Json(LoginResponse { authorize_url: url }))
}
async fn save_connection(
    state: &AppState,
    row: &ProviderRow,
    auth: CodexAuth,
) -> anyhow::Result<()> {
    let store = chatgpt::store(
        state.db.clone(),
        state.encryption.clone(),
        row.org_id,
        row.id,
    )?;
    let _lease = store.lock().await?;
    let current = state
        .db
        .get_provider(row.org_id, row.id.uuid())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Provider removed during login"))?;
    anyhow::ensure!(
        current.settings["chatgpt"]["generation"] == row.settings["chatgpt"]["generation"],
        "This login attempt was canceled or replaced"
    );
    let grant = auth
        .open_source
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("A ChatGPT plan grant is required"))?;
    let registration = ChatGptRegistration {
        client_id: auth
            .client_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Missing issuing client"))?,
        subject: grant
            .subject
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Missing validated account subject"))?,
        email: auth.email.clone(),
    };
    if let Some(saved) = current.settings["chatgpt"].get("registration") {
        anyhow::ensure!(
            saved["client_id"] == registration.client_id
                && saved["subject"] == registration.subject,
            "The account does not match this registration"
        );
    }
    let scope = oauth::grants_plan_usage(&grant.scopes);
    let mut settings = current.settings;
    settings["chatgpt"]["registration"] = json!(registration);
    settings["chatgpt"]["email"] = json!(auth.email);
    settings["chatgpt"]["status"] = if scope { "connected" } else { "scope_required" }.into();
    settings["chatgpt"]["error"] = if scope {
        serde_json::Value::Null
    } else {
        json!("Plan use was not granted. Reconnect to allow it, or choose an API provider.")
    };
    store.save(auth).await?;
    state
        .db
        .update_provider(
            row.org_id,
            row.id.uuid(),
            crate::storage::models::UpdateProvider {
                settings: Some(settings),
                ..chatgpt::empty_update()
            },
        )
        .await?;
    Ok(())
}
/// Disconnect the owner's ChatGPT account.
///
/// Cancels pending sign-in and revokes a stored grant before clearing credentials.
/// A failed revocation retains credentials so the owner can retry.
#[utoipa::path(
    delete,
    path = "/v1/providers/{provider_id}/chatgpt",
    params(("provider_id" = String, Path, description = "Personal provider ID")),
    responses(
        (status = 200, description = "Personal connection disconnect", body = serde_json::Value),
        (status = 404, description = "Provider not found or owned by another user"),
        (status = 400, description = "Invalid connection request"),
        (status = 502, description = "Provider exchange or revocation failed")
    ),
    tag = "providers"
)]
pub async fn disconnect(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    let row = row(&state, &org, &id).await?;
    let change = credential_change(&state, &org, &row, "disconnect_chatgpt").await?;
    cancel_attempt(org.org_id, row.id).await;
    let store = chatgpt::store(
        state.db.clone(),
        state.encryption.clone(),
        org.org_id,
        row.id,
    )
    .map_err(|_| ErrorResponse::internal_error())?;
    chatgpt::disconnect(&store).await.map_err(|_|ErrorResponse::new("Revocation was not confirmed. The connection was retained. Retry or disconnect the app in ChatGPT settings.").into_response(StatusCode::BAD_GATEWAY))?;
    change.finish(&row.id.to_string()).await;
    Ok(Json(json!({"disconnected":true})))
}
/// Import a private login-helper document.
///
/// Verifies installation, signed identity, nonce, issuing client, and account binding
/// before persisting credentials. Only the provider's owner may import the document.
#[utoipa::path(
    post,
    path = "/v1/providers/{provider_id}/chatgpt/import",
    params(("provider_id" = String, Path, description = "Personal provider ID")),
    request_body = ImportedConnection,
    responses(
        (status = 200, description = "Personal connection import", body = serde_json::Value),
        (status = 404, description = "Provider not found or owned by another user"),
        (status = 400, description = "Invalid connection request"),
        (status = 502, description = "Provider exchange or revocation failed")
    ),
    tag = "providers"
)]
pub async fn import(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(id): Path<String>,
    Json(input): Json<ImportedConnection>,
) -> ApiResult<serde_json::Value> {
    if !org.feature_flags.chatgpt_plan {
        return Err(ErrorResponse::feature_not_enabled("chatgpt_plan"));
    }
    let mut row = row(&state, &org, &id).await?;
    if input.host_id
        != row.settings["chatgpt"]["host_id"]
            .as_str()
            .unwrap_or_default()
    {
        return Err(
            ErrorResponse::new("The login file belongs to another installation.")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }
    let mut auth = input.auth;
    let client = auth.client_id.as_deref().ok_or_else(|| {
        ErrorResponse::new("The local helper must include the issuing client.")
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let id = auth
        .open_source
        .as_ref()
        .and_then(|g| g.id_token.as_deref())
        .ok_or_else(|| {
            ErrorResponse::new("The local helper must include an ID token.")
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    let endpoints = oauth::Endpoints::production();
    let keys = oauth::fetch_jwks(&endpoints.jwks)
        .await
        .map_err(|_| ErrorResponse::bad_gateway())?;
    let identity = oauth::validate_id_token(
        id,
        &keys,
        &endpoints.issuer,
        client,
        &input.nonce,
        oauth::now_epoch_millis() / 1000,
    )
    .map_err(|_| {
        ErrorResponse::new("The local login's identity could not be verified.")
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    auth.email = identity.email;
    auth.open_source
        .as_mut()
        .ok_or_else(|| {
            ErrorResponse::new("Missing plan grant").into_response(StatusCode::BAD_REQUEST)
        })?
        .subject = Some(identity.subject);
    let change = credential_change(&state, &org, &row, "import_chatgpt").await?;
    cancel_attempt(org.org_id, row.id).await;
    let store = chatgpt::store(
        state.db.clone(),
        state.encryption.clone(),
        org.org_id,
        row.id,
    )
    .map_err(|_| ErrorResponse::internal_error())?;
    let lease = store
        .lock()
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    let current = state
        .db
        .get_provider(org.org_id, row.id.uuid())
        .await
        .map_err(|_| ErrorResponse::internal_error())?
        .ok_or_else(|| ErrorResponse::not_found("Provider"))?;
    row.settings = current.settings;
    row.settings["chatgpt"]["generation"] = json!(uuid::Uuid::new_v4());
    state
        .db
        .update_provider(
            org.org_id,
            row.id.uuid(),
            crate::storage::models::UpdateProvider {
                settings: Some(row.settings.clone()),
                ..chatgpt::empty_update()
            },
        )
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    drop(lease);
    save_connection(&state, &row, auth).await.map_err(|_| {
        ErrorResponse::new("Cannot import this account registration.")
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let _ = state
        .sync_service
        .sync_provider(org.org_id, row.id.uuid())
        .await;
    change.finish(&row.id.to_string()).await;
    Ok(Json(json!({"connected":true})))
}

/// Disconnect and import change the provider's credentials outside the
/// command layer, so they record its history entry themselves.
async fn credential_change(
    state: &AppState,
    org: &ResolvedOrg,
    row: &ProviderRow,
    operation: &'static str,
) -> Result<crate::domains::change_history::rest::RestChange, (StatusCode, Json<ErrorResponse>)> {
    use crate::domains::change_history::{ChangeAction, EntityKind, rest::RestChange};
    let (db, caller, id) = (state.db.clone(), org.into(), row.id.to_string());
    let (kind, action) = (EntityKind::Provider, ChangeAction::Updated);
    Ok(RestChange::begin(
        db,
        caller,
        operation,
        kind,
        action,
        Some(&id),
        &["credentials"],
    )
    .await?)
}
