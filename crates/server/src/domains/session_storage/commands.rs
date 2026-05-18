use super::queries as q;
use super::types::{BatchSetSecretsResponse, KeyValueInfo, SecretInfo};
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

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

        for name in self.secrets.keys() {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 255 {
                return Err(CommandError::bad_request(format!(
                    "Secret name must be between 1 and 255 non-whitespace characters: '{name}'"
                )));
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
