// LLM provider and model rows, and the LLM generation reconciliation projection.

use crate::kernel_imports::contracts::typed_id::{ModelId, ProviderId};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct ProviderRow {
    pub id: ProviderId,
    pub org_id: i64,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub api_key_set: bool,
    pub status: String,
    pub settings: sqlx::types::JsonValue,
    /// Host-managed flag (EVE-810). When true, the OSS providers API refuses
    /// PATCH/DELETE on this row; the host owns it.
    pub managed: bool,
    /// When models were last synced from provider API
    pub last_synced_at: Option<DateTime<Utc>>,
    /// When someone last reviewed the discovered models; later discoveries
    /// that are still disabled count as new.
    pub models_reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct ModelRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Last time model was seen in provider API response
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Model with provider info joined
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ModelWithProviderRow {
    pub id: ModelId,
    pub org_id: i64,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: sqlx::types::JsonValue,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Last time model was seen in provider API response
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider_name: String,
    pub provider_type: String,
    /// Joined from `providers.api_key_set`. Used to derive `healthy` on
    /// the public model shape; stays internal to the storage layer.
    pub provider_api_key_set: bool,
    /// Joined from `providers.status`.
    pub provider_status: String,
}

/// LLM Provider with decrypted API key (used by worker activities)
#[derive(Debug, Clone)]
pub struct ProviderWithApiKey {
    pub id: ProviderId,
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    /// Decrypted API key (only available when needed for LLM calls)
    pub api_key: Option<String>,
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateProviderRow {
    pub name: String,
    pub provider_type: String,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UpdateProvider {
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateModelRow {
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_favorite: bool,
    /// Whether this model is enabled (visible in UI model pickers)
    pub enabled: bool,
    /// How the model was added: manual, discovered, or predefined
    pub source: String,
    /// Raw metadata from provider API response
    pub provider_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UpdateModel {
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<String>,
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub is_favorite: Option<bool>,
    /// Update enabled flag
    pub enabled: Option<bool>,
    /// Update last_seen_at timestamp (for sync tracking)
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Update provider metadata
    pub provider_metadata: Option<serde_json::Value>,
}

/// Minimal projection returned by `list_unreconciled_llm_generations`.
/// Contains only the fields needed to perform a reconciliation lookup.
#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
pub struct UnreconciledGeneration {
    pub id: uuid::Uuid,
    pub org_id: i64,
    pub provider_response_id: String,
}
