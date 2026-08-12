use super::queries as q;
use super::types::{BatchSetSecretsResponse, KeyValueInfo, SecretInfo};
use crate::domains::common::*;
use everruns_platform::capabilities::{
    is_internal_session_kv_key, is_internal_session_secret_name,
};
use serde::Deserialize;
use utoipa::ToSchema;

const MAX_SECRET_COUNT_PER_REQUEST: usize = 100;
const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionStorage {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ListSessionStorage {
    type Output = Vec<KeyValueInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_storage",
            category: "session_storage",
            description: "List all key-value pairs stored for a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/storage/keys",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<KeyValueInfo>, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let keys = ctx
            .db
            .list_session_keys(session_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        let mut items = Vec::with_capacity(keys.len());
        for key_info in keys {
            if is_internal_session_kv_key(&key_info.key) {
                continue;
            }

            let value = ctx
                .db
                .get_session_key_value(session_id.uuid(), &key_info.key)
                .await
                .map_err(classify_anyhow)?
                .map(|row| row.value)
                .unwrap_or_default();

            items.push(KeyValueInfo {
                key: key_info.key,
                value,
                created_at: key_info.created_at.to_rfc3339(),
                updated_at: key_info.updated_at.to_rfc3339(),
            });
        }

        Ok(items)
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionStorage>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionSecrets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ListSessionSecrets {
    type Output = Vec<SecretInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_secrets",
            category: "session_storage",
            description: "List all secrets stored for a session without revealing values.",
            method: "GET",
            path: "/v1/sessions/{session_id}/storage/secrets",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SecretInfo>, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let secrets = ctx
            .db
            .list_session_secrets(session_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        Ok(secrets
            .into_iter()
            .filter(|row| !is_internal_session_secret_name(&row.name))
            .map(|row| SecretInfo {
                name: row.name,
                created_at: row.created_at.to_rfc3339(),
                updated_at: row.updated_at.to_rfc3339(),
            })
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionSecrets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSetSessionSecrets {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub secrets: std::collections::HashMap<String, String>,
}

impl Command for BatchSetSessionSecrets {
    type Output = BatchSetSecretsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "batch_set_session_secrets",
            category: "session_storage",
            description: "Encrypt and store multiple session secrets in one request.",
            method: "PUT",
            path: "/v1/sessions/{session_id}/storage/secrets",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<BatchSetSecretsResponse, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let encryption = ctx.encryption.as_ref().ok_or_else(|| {
            CommandError::bad_request(
                "Encryption not configured. Set SECRETS_ENCRYPTION_KEY environment variable.",
            )
        })?;

        if self.secrets.is_empty() {
            return Ok(BatchSetSecretsResponse { count: 0 });
        }
        if self.secrets.len() > MAX_SECRET_COUNT_PER_REQUEST {
            return Err(CommandError::bad_request(format!(
                "At most {MAX_SECRET_COUNT_PER_REQUEST} secrets can be stored per request"
            )));
        }

        for (name, value) in &self.secrets {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 255 {
                return Err(CommandError::bad_request(format!(
                    "Secret name must be between 1 and 255 non-whitespace characters: '{name}'"
                )));
            }
            if is_internal_session_secret_name(name) {
                return Err(CommandError::bad_request(
                    "Secret name is reserved for internal use",
                ));
            }
            if value.is_empty() || value.len() > MAX_SECRET_VALUE_BYTES {
                return Err(CommandError::bad_request(
                    "Secret values must be between 1 byte and 64 KiB",
                ));
            }
        }

        for (name, value) in &self.secrets {
            let encrypted = encryption
                .encrypt_string(value)
                .map_err(CommandError::internal)?;
            ctx.db
                .upsert_session_secret(crate::storage::models::UpsertSessionSecret {
                    session_id,
                    name: name.clone(),
                    value_encrypted: encrypted,
                })
                .await
                .map_err(classify_anyhow)?;
        }

        Ok(BatchSetSecretsResponse {
            count: self.secrets.len(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<BatchSetSessionSecrets>() }

#[derive(Debug, Deserialize, ToSchema)]
/// Delete one encrypted secret from a session's private storage.
pub struct DeleteSessionSecret {
    /// Session that owns the secret.
    #[schema(example = "session_01933b5a000070008000000000000001")]
    pub session_id: String,
    /// Exact secret name to delete.
    #[schema(example = "SERVICE_TOKEN")]
    pub name: String,
}

impl Command for DeleteSessionSecret {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_session_secret",
            category: "session_storage",
            description: "Delete a user-managed encrypted session secret by name.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/storage/secrets/{name}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
        if is_internal_session_secret_name(&self.name) {
            return Err(CommandError::not_found("Secret"));
        }
        ctx.db
            .delete_session_secret(session_id.uuid(), &self.name)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSessionSecret>() }
