// Session storage domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

pub use everruns_contracts::runtime::{SecretAuth, SecretBinding, SecretScope};

/// Batch secret set request
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSetSecretsRequest {
    /// Map of secret names to values. Names are case-sensitive; values are stored encrypted
    /// and never returned by list/read endpoints. Existing keys are overwritten.
    /// Example: `{"OPENAI_API_KEY": "sk-...", "GITHUB_TOKEN": "ghp_..."}`.
    pub secrets: HashMap<String, String>,
    /// Optional bindings keyed by secret name. A bound secret is write-only:
    /// the agent's shell sees `$NAME` as a placeholder, and the platform puts
    /// the value on requests to the bound URL only. A name set without a
    /// binding becomes an ordinary secret again.
    #[serde(default)]
    pub bindings: HashMap<String, SecretBinding>,
}

/// Batch secret set response
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchSetSecretsResponse {
    /// Number of secrets stored
    pub count: usize,
}

/// Key-value entry info (key and timestamps, no value)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyValueInfo {
    /// The key name
    pub key: String,
    /// The stored value
    pub value: String,
    /// When the key was created
    pub created_at: String,
    /// When the key was last updated
    pub updated_at: String,
}

/// Secret entry info (name and timestamps only, no value)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecretInfo {
    /// The secret name
    pub name: String,
    /// Where the value may be sent, for a bound (write-only) secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<SecretBinding>,
    /// When the secret was created
    pub created_at: String,
    /// When the secret was last updated
    pub updated_at: String,
}

/// An agent-scoped bound secret, without its value.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentSecretInfo {
    /// Env var name the agent's shell sees.
    pub name: String,
    /// Where the value may be sent, and how.
    pub binding: SecretBinding,
    /// When the secret was created
    pub created_at: String,
    /// When the secret was last updated
    pub updated_at: String,
}
