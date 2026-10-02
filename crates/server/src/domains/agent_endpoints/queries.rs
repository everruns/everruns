// Agent endpoint query helpers — shared by commands, ingress and the frozen
// App archival domain.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::models::UpdateEndpointByIdRow;
use everruns_durable::UpdateField;
use everruns_platform::{
    AgentEndpoint, AgentEndpointId, AgentVersionPolicy, EndpointAuthConfig, EndpointStatus,
    EndpointTransport,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::storage::models::AgentEndpointRow;

// ============================================================================
// Encryption helpers
// ============================================================================

/// Encrypt channel_config JSON to bytes. Returns None if encryption is not
/// configured (dev/test mode) or the config is empty.
pub fn encrypt_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<Option<Vec<u8>>> {
    if config.is_null() || (config.is_object() && config.as_object().unwrap().is_empty()) {
        return Ok(None);
    }
    let encryption = match encryption {
        Some(e) => e,
        None => return Ok(None),
    };
    let json = serde_json::to_string(config)?;
    Ok(Some(encryption.encrypt_string(&json)?))
}

/// Decrypt channel_config from encrypted bytes. The plaintext column is used
/// only when the row has no encrypted payload.
pub fn decrypt_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    encrypted: Option<&[u8]>,
    plaintext_fallback: &serde_json::Value,
) -> serde_json::Value {
    match (encrypted, encryption) {
        (None, _) => plaintext_fallback.clone(),
        (Some(_), None) => {
            tracing::error!(
                "channel_config is encrypted but encryption is unavailable; returning null"
            );
            serde_json::Value::Null
        }
        (Some(data), Some(enc)) => {
            let json = match enc.decrypt_to_string(data) {
                Ok(json) => json,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        "Failed to decrypt channel_config; returning null instead of plaintext fallback"
                    );
                    return serde_json::Value::Null;
                }
            };
            match serde_json::from_str(&json) {
                Ok(val) => val,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        "Failed to parse decrypted channel_config JSON; returning null"
                    );
                    serde_json::Value::Null
                }
            }
        }
    }
}

/// Prepare channel_config for storage: encrypt if possible, redact plaintext when encrypted.
pub fn prepare_channel_config(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<(serde_json::Value, Option<Vec<u8>>)> {
    let encrypted = encrypt_channel_config(encryption, config)?;
    let stored_plaintext = if encrypted.is_some() {
        serde_json::Value::Object(Default::default())
    } else {
        config.clone()
    };
    Ok((stored_plaintext, encrypted))
}

pub struct PreparedChannelStorage {
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
}

/// Store transport configuration and endpoint auth independently. Auth may
/// contain secret-equivalent hashes and provider credentials, so it follows
/// the same encryption policy as channel configuration.
pub fn prepare_channel_storage(
    encryption: Option<&Arc<EncryptionService>>,
    config: &serde_json::Value,
) -> anyhow::Result<PreparedChannelStorage> {
    let mut transport = config.clone();
    let auth = transport
        .as_object_mut()
        .and_then(|object| object.remove("auth"))
        .filter(|value| !value.is_null());
    let auth = auth.map(|value| {
        serde_json::from_value::<EndpointAuthConfig>(value.clone())
            .and_then(serde_json::to_value)
            .unwrap_or(value)
    });
    let (channel_config, mut channel_config_encrypted) =
        prepare_channel_config(encryption, &transport)?;
    if auth.is_some()
        && channel_config_encrypted.is_none()
        && let Some(encryption) = encryption
    {
        let json = serde_json::to_string(&transport)?;
        channel_config_encrypted = Some(encryption.encrypt_string(&json)?);
    }
    let (auth, auth_encrypted) = match auth {
        Some(auth) => {
            let encrypted = encrypt_channel_config(encryption, &auth)?;
            if encrypted.is_some() {
                (None, encrypted)
            } else {
                (Some(auth), None)
            }
        }
        None => (None, None),
    };
    Ok(PreparedChannelStorage {
        channel_config,
        channel_config_encrypted,
        auth,
        auth_encrypted,
    })
}

fn first_class_auth_value(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AgentEndpointRow,
) -> Option<serde_json::Value> {
    if let Some(encrypted) = row.auth_encrypted.as_deref() {
        let Some(encryption) = encryption else {
            tracing::error!("Endpoint auth is encrypted but encryption is unavailable");
            return Some(serde_json::json!({"mode": "http_basic"}));
        };
        let json = match encryption.decrypt_to_string(encrypted) {
            Ok(json) => json,
            Err(err) => {
                tracing::error!(error = %err, "Failed to decrypt endpoint auth");
                return Some(serde_json::json!({"mode": "http_basic"}));
            }
        };
        return serde_json::from_str(&json)
            .map_err(|err| {
                tracing::error!(error = %err, "Failed to parse decrypted endpoint auth JSON");
            })
            .ok()
            .or_else(|| Some(serde_json::json!({"mode": "http_basic"})));
    }
    row.auth.clone()
}

pub fn decrypt_endpoint_auth(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AgentEndpointRow,
) -> Option<EndpointAuthConfig> {
    first_class_auth_value(encryption, row).map(|value| {
        serde_json::from_value(value).unwrap_or_else(|err| {
            tracing::error!(error = %err, "Failed to parse endpoint auth");
            fail_closed_endpoint_auth()
        })
    })
}

fn fail_closed_endpoint_auth() -> EndpointAuthConfig {
    serde_json::from_value(serde_json::json!({"mode": "http_basic"}))
        .expect("fail-closed endpoint auth is valid")
}

pub(crate) fn parse_legacy_endpoint_auth(value: serde_json::Value) -> EndpointAuthConfig {
    serde_json::from_value(value).unwrap_or_else(|err| {
        tracing::error!(error = %err, "Failed to parse legacy endpoint auth");
        fail_closed_endpoint_auth()
    })
}
/// Return a write-ready config that includes either first-class auth or the
/// encrypted legacy nested value. First-class storage is authoritative even
/// when its payload cannot be decrypted or parsed.
pub fn channel_config_with_auth(
    encryption: Option<&Arc<EncryptionService>>,
    row: &AgentEndpointRow,
) -> serde_json::Value {
    let mut config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );
    if (row.auth.is_some() || row.auth_encrypted.is_some())
        && let Some(object) = config.as_object_mut()
    {
        object.remove("auth");
        if let Some(auth) = first_class_auth_value(encryption, row) {
            object.insert("auth".to_string(), auth);
        }
    }
    config
}

// ============================================================================
// Row mapping
// ============================================================================

pub fn channel_row_to_channel(
    encryption: Option<&Arc<EncryptionService>>,
    row: AgentEndpointRow,
) -> AgentEndpoint {
    let public_id: AgentEndpointId = row
        .public_id
        .parse()
        .unwrap_or_else(|_| AgentEndpointId::from_uuid(row.id));

    let mut channel_config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );
    let legacy_auth = channel_config
        .as_object_mut()
        .and_then(|object| object.remove("auth"));
    let auth = if row.auth.is_some() || row.auth_encrypted.is_some() {
        decrypt_endpoint_auth(encryption, &row).map(Box::new)
    } else {
        legacy_auth.map(|value| Box::new(parse_legacy_endpoint_auth(value)))
    };

    AgentEndpoint {
        public_id,
        internal_id: row.id,
        channel_type: EndpointTransport::from_str_opt(&row.channel_type)
            .unwrap_or(EndpointTransport::Slack),
        channel_config,
        auth,
        enabled: row.enabled,
        status: EndpointStatus::from(row.status.as_str()),
        agent_version_policy: AgentVersionPolicy::Default,
        agent_version_id: None,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Update channel config with proper encryption handling.
/// Used by webhook handlers (unauthenticated, no caller context).
pub async fn update_channel_config_unscoped(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    channel_internal_id: Uuid,
    config: &serde_json::Value,
) -> anyhow::Result<()> {
    let prepared = prepare_channel_storage(encryption, config)?;
    let input = UpdateEndpointByIdRow {
        channel_config: Some(prepared.channel_config),
        channel_config_encrypted: UpdateField::from_option(prepared.channel_config_encrypted),
        auth: UpdateField::from_option(prepared.auth),
        auth_encrypted: UpdateField::from_option(prepared.auth_encrypted),
        ..Default::default()
    };
    db.update_endpoint_by_id(channel_internal_id, input).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::generate_encryption_key;
    use chrono::Utc;

    fn row(
        channel_config: serde_json::Value,
        channel_config_encrypted: Option<Vec<u8>>,
        auth: Option<serde_json::Value>,
        auth_encrypted: Option<Vec<u8>>,
    ) -> AgentEndpointRow {
        AgentEndpointRow {
            id: Uuid::now_v7(),
            app_id: Uuid::now_v7(),
            public_id: AgentEndpointId::new().to_string(),
            channel_type: "ag_ui".to_string(),
            channel_config,
            channel_config_encrypted,
            auth,
            auth_encrypted,
            durable_schedule_id: None,
            enabled: true,
            status: "live".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new(&generate_encryption_key("endpoint-auth-test"), &[]).unwrap(),
        )
    }

    #[test]
    fn channel_storage_separates_and_encrypts_auth() {
        let encryption = encryption();
        let config = serde_json::json!({
            "anonymous": false,
            "token": "transport-secret",
            "auth": {
                "mode": "http_basic",
                "provider": {
                    "type": "http_basic",
                    "username": "operator",
                    "password_hash": "$argon2id$test"
                }
            }
        });

        let prepared = prepare_channel_storage(Some(&encryption), &config).unwrap();
        assert!(prepared.auth.is_none());
        assert!(prepared.auth_encrypted.is_some());
        let transport = decrypt_channel_config(
            Some(&encryption),
            prepared.channel_config_encrypted.as_deref(),
            &prepared.channel_config,
        );
        assert!(transport.get("auth").is_none());
        assert_eq!(transport["token"], "transport-secret");

        let auth: serde_json::Value = serde_json::from_str(
            &encryption
                .decrypt_to_string(prepared.auth_encrypted.as_deref().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(auth["provider"]["password_hash"], "$argon2id$test");
    }
    #[test]
    fn auth_only_storage_replaces_legacy_transport_with_encrypted_empty_object() {
        let encryption = encryption();
        let prepared = prepare_channel_storage(
            Some(&encryption),
            &serde_json::json!({"auth": {"mode": "anonymous"}}),
        )
        .unwrap();

        let transport = decrypt_channel_config(
            Some(&encryption),
            prepared.channel_config_encrypted.as_deref(),
            &prepared.channel_config,
        );
        assert_eq!(transport, serde_json::json!({}));
        assert!(prepared.auth_encrypted.is_some());
    }

    #[test]
    fn first_class_auth_overrides_and_removes_legacy_auth() {
        let row = row(
            serde_json::json!({
                "auth": {"mode": "anonymous"},
                "anonymous": true
            }),
            None,
            Some(serde_json::json!({"mode": "google_oidc", "provider": {
                "type": "google_oidc",
                "client_id": "client"
            }})),
            None,
        );

        let channel = channel_row_to_channel(None, row);
        assert_eq!(
            channel.auth.map(|auth| auth.mode),
            Some(everruns_platform::EndpointAuthMode::GoogleOidc)
        );
        assert!(channel.channel_config.get("auth").is_none());
    }

    #[test]
    fn corrupt_first_class_auth_fails_closed_without_legacy_fallback() {
        let row = row(
            serde_json::json!({"auth": {"mode": "anonymous"}}),
            None,
            None,
            Some(b"not-an-encrypted-payload".to_vec()),
        );

        let channel = channel_row_to_channel(Some(&encryption()), row);
        assert_eq!(
            channel.auth.map(|auth| auth.mode),
            Some(everruns_platform::EndpointAuthMode::HttpBasic)
        );
        assert!(channel.channel_config.get("auth").is_none());
    }

    #[test]
    fn encrypted_transport_without_encryption_does_not_use_plaintext_fallback() {
        let row = row(
            serde_json::json!({}),
            Some(b"encrypted-legacy-auth".to_vec()),
            None,
            None,
        );

        let channel = channel_row_to_channel(None, row);
        assert!(channel.channel_config.is_null());
        assert!(channel.auth.is_none());
        assert!(channel.ag_ui_config().is_none());
    }
    #[test]
    fn encrypted_legacy_auth_is_available_for_lazy_split() {
        let encryption = encryption();
        let legacy = serde_json::json!({
            "anonymous": false,
            "auth": {"mode": "o_auth2_introspection", "provider": {
                "type": "o_auth2_introspection",
                "introspection_url": "https://identity.example.com/introspect"
            }}
        });
        let encrypted = encrypt_channel_config(Some(&encryption), &legacy).unwrap();
        let row = row(serde_json::json!({}), encrypted, None, None);

        let config = channel_config_with_auth(Some(&encryption), &row);
        assert_eq!(config["auth"]["mode"], "o_auth2_introspection");
        let channel = channel_row_to_channel(Some(&encryption), row);
        let auth = serde_json::to_value(channel.auth.unwrap()).unwrap();
        assert_eq!(auth["mode"], "oauth2_introspection");
        assert_eq!(auth["provider"]["type"], "oauth2_introspection");

        let prepared = prepare_channel_storage(Some(&encryption), &config).unwrap();
        let rewritten: serde_json::Value = serde_json::from_str(
            &encryption
                .decrypt_to_string(prepared.auth_encrypted.as_deref().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(rewritten["mode"], "oauth2_introspection");
        assert_eq!(rewritten["provider"]["type"], "oauth2_introspection");
    }
}
