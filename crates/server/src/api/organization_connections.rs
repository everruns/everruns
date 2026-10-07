//! Organization-owned provider accounts for managed sandboxes.

use crate::auth::middleware::OrgAdmin;
use crate::storage::CreateOrganizationConnectionRow;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use everruns_contracts::connector::ConnectorType;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    common::ErrorResponse,
    user_connections::{CreateApiKeyConnectionRequest, VerifyConnectionResponse},
};

pub type AppState = super::virtual_user_connections::AppState;

#[derive(Debug, Deserialize, ToSchema)]
/// Organization-owned sandbox provider account input.
pub struct SaveOrganizationConnectionRequest {
    /// Human-readable account name shown in Sandbox Template selectors.
    #[schema(example = "Production Daytona")]
    pub name: String,
    /// Provider credential and any provider-specific connection fields.
    #[serde(flatten)]
    pub credential: CreateApiKeyConnectionRequest,
}

#[derive(Debug, Serialize, ToSchema)]
/// Non-secret metadata for an organization-owned sandbox provider account.
pub struct OrganizationConnectionResponse {
    /// Stable connection identifier used by Sandbox Templates.
    #[schema(example = "00000000-0000-0000-0000-000000000002")]
    pub id: Uuid,
    /// Human-readable account name.
    #[schema(example = "Production Daytona")]
    pub name: String,
    /// Sandbox provider connector identifier.
    #[schema(example = "daytona")]
    pub provider: String,
    /// Provider identity returned while validating the credential, when available.
    #[schema(example = "platform-team")]
    pub provider_username: Option<String>,
    /// Time at which the provider account was first connected.
    #[schema(example = "2026-10-07T12:00:00Z")]
    pub connected_at: DateTime<Utc>,
    /// Time at which the account name or credential was last replaced.
    #[schema(example = "2026-10-07T12:30:00Z")]
    pub updated_at: DateTime<Utc>,
}

impl OrganizationConnectionResponse {
    fn from_row(row: crate::storage::models::VirtualUserConnectionRow) -> Self {
        Self {
            id: row.id,
            name: row.name.unwrap_or_else(|| row.provider.clone()),
            provider: row.provider,
            provider_username: row.provider_username,
            connected_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs/{org}/organization-connections",
            get(list_connections),
        )
        .route(
            "/v1/orgs/{org}/organization-connections/providers/{provider}",
            post(create_connection),
        )
        .route(
            "/v1/orgs/{org}/organization-connections/{connection_id}",
            put(update_connection).delete(delete_connection),
        )
        .route(
            "/v1/orgs/{org}/organization-connections/{connection_id}/verify",
            post(verify_connection),
        )
        .with_state(state)
}

fn clean_name(name: &str) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let name = name.trim();
    if name.is_empty() || name.len() > 255 {
        return Err(
            ErrorResponse::new("Account name must be 1 to 255 characters")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }
    Ok(name.to_string())
}

fn provider<'a>(
    state: &'a AppState,
    id: &str,
) -> Result<&'a Arc<dyn everruns_contracts::connector::Connector>, (StatusCode, Json<ErrorResponse>)>
{
    let provider = state.connectors.get(id).ok_or_else(|| {
        ErrorResponse::new(format!("Unknown connector: {id}")).into_response(StatusCode::NOT_FOUND)
    })?;
    if provider.connection_type() != ConnectorType::ApiKey
        || !provider.capabilities().contains(&"sandbox_provisioning")
    {
        return Err(ErrorResponse::new(format!(
            "Provider '{id}' does not support organization sandbox accounts"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }
    Ok(provider)
}

async fn require_provider_enabled(
    state: &AppState,
    org_id: i64,
    provider_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let enabled = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org_id,
        &state.auth.feature_flag_policy,
    )
    .await
    .is_ok_and(|flags| flags.is_connector_enabled(provider_id));
    if !enabled {
        return Err(
            ErrorResponse::new(format!("Unknown connector: {provider_id}"))
                .into_response(StatusCode::NOT_FOUND),
        );
    }
    Ok(())
}

fn fields(body: &SaveOrganizationConnectionRequest) -> HashMap<String, String> {
    let mut fields = HashMap::from([("api_key".to_string(), body.credential.api_key.clone())]);
    for (key, value) in &body.credential.extra_fields {
        if let Some(value) = value.as_str() {
            fields.insert(key.clone(), value.to_string());
        }
    }
    fields
}

#[utoipa::path(get, path = "/v1/orgs/{org}/organization-connections", description = "List non-secret metadata for organization-owned sandbox provider accounts.", responses((status = 200, body = Vec<OrganizationConnectionResponse>)), tag = "sandbox-templates")]
pub async fn list_connections(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
) -> Result<Json<Vec<OrganizationConnectionResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let rows = state
        .db
        .list_organization_connections(org.org_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to list organization connections");
            ErrorResponse::new("Internal error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok(Json(
        rows.into_iter()
            .map(OrganizationConnectionResponse::from_row)
            .collect(),
    ))
}

#[utoipa::path(post, path = "/v1/orgs/{org}/organization-connections/providers/{provider}", description = "Create and validate an encrypted organization-owned sandbox provider account.", request_body = SaveOrganizationConnectionRequest, responses((status = 201, body = OrganizationConnectionResponse)), tag = "sandbox-templates")]
pub async fn create_connection(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org, provider_id)): Path<(String, String)>,
    Json(body): Json<SaveOrganizationConnectionRequest>,
) -> Result<(StatusCode, Json<OrganizationConnectionResponse>), (StatusCode, Json<ErrorResponse>)> {
    let name = clean_name(&body.name)?;
    require_provider_enabled(&state, org.org_id, &provider_id).await?;
    let provider = provider(&state, &provider_id)?;
    let validation = provider
        .validate_fields(&fields(&body))
        .await
        .map_err(|error| {
            ErrorResponse::new(format!("Credential validation failed: {error}"))
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        ErrorResponse::new("Encryption not configured")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;
    let encrypted = encryption
        .encrypt_string(&body.credential.api_key)
        .map_err(|error| {
            tracing::error!(%error, "Failed to encrypt organization connection");
            ErrorResponse::new("Failed to store connection")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    let row = state
        .db
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: org.org_id,
            name,
            provider: provider_id,
            access_token_encrypted: encrypted,
            provider_username: validation.provider_username,
            provider_metadata: validation.provider_metadata,
        })
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to create organization connection");
            ErrorResponse::new("Failed to store connection")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok((
        StatusCode::CREATED,
        Json(OrganizationConnectionResponse::from_row(row)),
    ))
}

#[utoipa::path(put, path = "/v1/orgs/{org}/organization-connections/{connection_id}", description = "Replace the name and credential for an organization-owned sandbox provider account.", request_body = SaveOrganizationConnectionRequest, responses((status = 200, body = OrganizationConnectionResponse)), tag = "sandbox-templates")]
pub async fn update_connection(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org, connection_id)): Path<(String, Uuid)>,
    Json(body): Json<SaveOrganizationConnectionRequest>,
) -> Result<Json<OrganizationConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let current = state
        .db
        .get_organization_connection(org.org_id, connection_id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Internal error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Connection not found").into_response(StatusCode::NOT_FOUND)
        })?;
    let name = clean_name(&body.name)?;
    require_provider_enabled(&state, org.org_id, &current.provider).await?;
    let validation = provider(&state, &current.provider)?
        .validate_fields(&fields(&body))
        .await
        .map_err(|error| {
            ErrorResponse::new(format!("Credential validation failed: {error}"))
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    let encrypted = state
        .encryption
        .as_ref()
        .ok_or_else(|| {
            ErrorResponse::new("Encryption not configured")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .encrypt_string(&body.credential.api_key)
        .map_err(|_| {
            ErrorResponse::new("Failed to store connection")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    let row = state
        .db
        .update_organization_connection(
            org.org_id,
            connection_id,
            &name,
            &encrypted,
            validation.provider_username.as_deref(),
            validation.provider_metadata.as_ref(),
        )
        .await
        .map_err(|_| {
            ErrorResponse::new("Failed to store connection")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Connection not found").into_response(StatusCode::NOT_FOUND)
        })?;
    Ok(Json(OrganizationConnectionResponse::from_row(row)))
}

#[utoipa::path(delete, path = "/v1/orgs/{org}/organization-connections/{connection_id}", description = "Delete an organization-owned provider account when no active sandbox configuration or lease references it.", responses((status = 204)), tag = "sandbox-templates")]
pub async fn delete_connection(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org, connection_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    match state
        .db
        .delete_organization_connection(org.org_id, connection_id)
        .await
    {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => {
            Err(ErrorResponse::new("Connection not found").into_response(StatusCode::NOT_FOUND))
        }
        Err(error)
            if error
                .downcast_ref::<crate::storage::repositories::OrganizationConnectionInUse>()
                .is_some() =>
        {
            tracing::warn!(%error, %connection_id, "Organization connection remains in use");
            Err(
                ErrorResponse::new("Connection is in use by a sandbox or template")
                    .into_response(StatusCode::CONFLICT),
            )
        }
        Err(error) => {
            tracing::error!(%error, %connection_id, "Failed to delete organization connection");
            Err(ErrorResponse::new("Internal error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

#[utoipa::path(post, path = "/v1/orgs/{org}/organization-connections/{connection_id}/verify", description = "Revalidate an organization-owned sandbox provider credential without returning the secret.", responses((status = 200, body = VerifyConnectionResponse)), tag = "sandbox-templates")]
pub async fn verify_connection(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org, connection_id)): Path<(String, Uuid)>,
) -> Result<Json<VerifyConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let row = state
        .db
        .get_organization_connection(org.org_id, connection_id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Internal error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Connection not found").into_response(StatusCode::NOT_FOUND)
        })?;
    let encrypted = row.access_token_encrypted.ok_or_else(|| {
        ErrorResponse::new("Credential missing").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;
    let key = state
        .encryption
        .as_ref()
        .ok_or_else(|| {
            ErrorResponse::new("Encryption not configured")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .decrypt_to_string(&encrypted)
        .map_err(|_| {
            ErrorResponse::new("Failed to verify connection")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    require_provider_enabled(&state, org.org_id, &row.provider).await?;
    let valid = provider(&state, &row.provider)?
        .validate(&key)
        .await
        .is_ok();
    Ok(Json(VerifyConnectionResponse {
        valid,
        error: (!valid).then(|| "Connection verification failed".to_string()),
    }))
}
