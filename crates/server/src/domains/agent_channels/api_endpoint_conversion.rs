// Agent Execution API, phase 6: convert retired `api_endpoint` channels to
// `api` channels, once, at server startup.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - In Rust at startup, not in a SQL migration: the key hash and auth sit in
//   encrypted columns that only the encryption service can read. Startup runs
//   it before the server takes traffic and never fails because of it; a
//   channel that cannot be read (encrypted with no key, say) stays as it was
//   and is retried on the next start.
// - The row converts in place: same id, same `/v1/channels/{channel_id}` base
//   URL, and the sessions it started keep their `sessions.channel_id`.
// - The channel's key becomes an agent key named "Imported key" with the same
//   SHA-256 hash and prefix, so the `evr_app_` secret keeps authenticating.
//   Only when the key was the channel's credential (auth `api_key` or none):
//   under any other auth mode the key was never accepted, and importing it
//   would grant access that did not exist.
// - Sessions it started get the imported key's tags, so the key still finds
//   them. A shared session (`shared_session`) has no equivalent: the channel
//   becomes `per_user` and the one imported key owns it.
// - `oidc`, `google_oidc` and `oauth2_introspection` carry over as the api
//   channel's `auth_methods`. Each token subject becomes its own end user, so
//   their earlier shared sessions are not theirs and are not retagged.
// - Anonymous, shared-secret, Basic and mTLS endpoints have no Agent API
//   equivalent: the channel converts `disabled` (not live, so it answers
//   nothing) and a warning names it. The owner adds a key and republishes.
// - Visibility is `messages`: the old routes returned only final agent text.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use super::queries::{decrypt_channel_config, parse_legacy_channel_auth, prepare_channel_config};
use super::record::api::{
    API_AUTH_MODES, AgentApiChannelConfig, ApiVisibility, agent_key_public_id,
};
use super::record::{ChannelAuthConfig, ChannelAuthMode, ChannelType, SessionBinding};
use crate::storage::{
    ApiEndpointChannelRow, ConvertedApiChannelRow, EncryptionService, StorageBackend,
};

/// Name of the agent key a converted channel's key becomes.
pub const IMPORTED_KEY_NAME: &str = "Imported key";

/// The stored config of an `api_endpoint` channel.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    api_key_hash: String,
    api_key_prefix: String,
    #[serde(default = "shared")]
    session_mode: SessionBinding,
    #[serde(default)]
    rate_limit_per_minute: Option<u32>,
}

fn shared() -> SessionBinding {
    SessionBinding::Shared
}

/// What one channel becomes.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversionPlan {
    pub config: AgentApiChannelConfig,
    /// The key to import: `(hash, prefix)`.
    pub key: Option<(String, String)>,
    /// Why the channel converts disabled, if it does.
    pub disabled_because: Option<&'static str>,
}

/// The plan for a channel with this decrypted config and auth.
pub fn plan_conversion(config: &Value, auth: Option<&ChannelAuthConfig>) -> ConversionPlan {
    let legacy = serde_json::from_value::<LegacyConfig>(config.clone()).ok();
    let mut plan = ConversionPlan {
        config: AgentApiChannelConfig {
            session_binding: match legacy.as_ref().map(|l| l.session_mode) {
                Some(SessionBinding::Ephemeral) => SessionBinding::Ephemeral,
                _ => SessionBinding::Requester,
            },
            visibility: ApiVisibility::Messages,
            rate_limit_per_minute: legacy
                .as_ref()
                .and_then(|l| l.rate_limit_per_minute)
                .filter(|limit| (1..=1_000_000).contains(limit)),
            ..AgentApiChannelConfig::default()
        },
        key: None,
        disabled_because: None,
    };
    let Some(legacy) = legacy else {
        plan.disabled_because = Some("its config could not be read");
        return plan;
    };
    match auth.map(|auth| &auth.mode) {
        None | Some(ChannelAuthMode::ApiKey) => {
            if legacy.api_key_hash.trim().is_empty() {
                plan.disabled_because = Some("it has no key");
            } else {
                plan.key = Some((legacy.api_key_hash, legacy.api_key_prefix));
            }
        }
        Some(mode) if API_AUTH_MODES.contains(mode) => {
            let auth = auth.cloned().expect("matched Some");
            let mut candidate = plan.config.clone();
            candidate.auth_methods = vec![auth];
            let valid = serde_json::to_value(&candidate).is_ok_and(|value| {
                super::validation::normalize_and_validate_channel_config(ChannelType::Api, value)
                    .is_ok()
            });
            if valid {
                plan.config = candidate;
            } else {
                plan.disabled_because = Some("its identity provider settings are not valid");
            }
        }
        Some(_) => {
            plan.disabled_because = Some(
                "its sign-in method (anonymous, shared secret, Basic or mTLS) has no Agent API equivalent",
            );
        }
    }
    plan
}

/// Counts of one conversion pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConversionReport {
    /// Converted and serving as before.
    pub converted: usize,
    /// Converted, but disabled until the owner acts.
    pub disabled: usize,
    /// Left as they were: unreadable, or held by another replica.
    pub skipped: usize,
}

/// Convert every `api_endpoint` channel. Each one commits on its own; a
/// failure leaves that channel as it was.
pub async fn convert_api_endpoint_channels(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
) -> anyhow::Result<ConversionReport> {
    let mut report = ConversionReport::default();
    for id in db.list_api_endpoint_channel_ids().await? {
        match convert_one(db, encryption, id).await {
            Ok(Some(true)) => report.converted += 1,
            Ok(Some(false)) => report.disabled += 1,
            Ok(None) => report.skipped += 1,
            Err(error) => {
                tracing::error!(channel = %id, %error, "api_endpoint channel conversion failed");
                report.skipped += 1;
            }
        }
    }
    Ok(report)
}

/// Startup entry point: converts and logs, never fails the server.
pub async fn convert_at_startup(db: &StorageBackend, encryption: Option<&Arc<EncryptionService>>) {
    match convert_api_endpoint_channels(db, encryption).await {
        Ok(report) if report == ConversionReport::default() => {}
        Ok(report) => tracing::info!(
            converted = report.converted,
            disabled = report.disabled,
            skipped = report.skipped,
            "Converted api_endpoint channels to api channels"
        ),
        Err(error) => tracing::error!(%error, "api_endpoint channel conversion did not run"),
    }
}

/// `Some(true)` converted live as before, `Some(false)` converted disabled,
/// `None` left alone.
async fn convert_one(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    id: uuid::Uuid,
) -> anyhow::Result<Option<bool>> {
    let Some(mut locked) = db.lock_api_endpoint_channel(id).await? else {
        return Ok(None);
    };
    let Some((config, auth)) = readable(&locked.row, encryption) else {
        tracing::error!(
            channel = %locked.row.public_id,
            "api_endpoint channel config cannot be decrypted; left unconverted until it can"
        );
        return Ok(None);
    };
    let mut plan = plan_conversion(&config, auth.as_ref());
    if let Some((hash, prefix)) = plan.key.take() {
        match locked.import_key(IMPORTED_KEY_NAME, &hash, &prefix).await? {
            Some(key_id) => {
                let channel = &locked.row.public_id;
                let tags = vec![
                    format!("api_channel:{channel}"),
                    format!("api_key:{}", agent_key_public_id(key_id)),
                ];
                locked
                    .retag_sessions(&format!("app_channel:{channel}"), &tags)
                    .await?;
            }
            None => plan.disabled_because = Some("its key is already another agent key"),
        }
    }
    let live = plan.disabled_because.is_none();
    if let Some(reason) = plan.disabled_because {
        tracing::warn!(
            channel = %locked.row.public_id,
            reason,
            "api_endpoint channel converted to a disabled api channel: add an agent key and publish it again"
        );
    }
    let (channel_config, channel_config_encrypted) =
        prepare_channel_config(encryption, &serde_json::to_value(&plan.config)?)?;
    let (enabled, status) = if live {
        (locked.row.enabled, locked.row.status.clone())
    } else {
        (false, "disabled".to_string())
    };
    locked
        .finish(ConvertedApiChannelRow {
            channel_config,
            channel_config_encrypted,
            enabled,
            status,
        })
        .await?;
    Ok(Some(live))
}

/// The row's config and auth, or `None` when either cannot be decrypted.
fn readable(
    row: &ApiEndpointChannelRow,
    encryption: Option<&Arc<EncryptionService>>,
) -> Option<(Value, Option<ChannelAuthConfig>)> {
    let mut config = decrypt_channel_config(
        encryption,
        row.channel_config_encrypted.as_deref(),
        &row.channel_config,
    );
    if config.is_null() {
        return None;
    }
    let nested = config
        .as_object_mut()
        .and_then(|object| object.remove("auth"));
    let auth = match (&row.auth_encrypted, &row.auth) {
        (Some(encrypted), _) => {
            let json = encryption?.decrypt_to_string(encrypted).ok()?;
            Some(serde_json::from_str::<Value>(&json).ok()?)
        }
        (None, Some(plain)) => Some(plain.clone()),
        (None, None) => nested,
    };
    let auth = auth
        .filter(|value| !value.is_null())
        .map(parse_legacy_channel_auth);
    Some((config, auth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legacy(mode: &str) -> Value {
        json!({
            "api_key_hash": "ab".repeat(32),
            "api_key_prefix": "evr_app_1234...",
            "session_mode": mode,
            "rate_limit_per_minute": 30,
        })
    }

    fn auth(value: Value) -> ChannelAuthConfig {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn key_endpoints_import_their_key_and_keep_their_limits() {
        for auth in [None, Some(auth(json!({"mode": "api_key"})))] {
            let plan = plan_conversion(&legacy("shared_session"), auth.as_ref());
            assert_eq!(plan.disabled_because, None);
            assert_eq!(
                plan.key,
                Some(("ab".repeat(32), "evr_app_1234...".to_string()))
            );
            assert_eq!(plan.config.session_binding, SessionBinding::Requester);
            assert_eq!(plan.config.visibility, ApiVisibility::Messages);
            assert_eq!(plan.config.rate_limit_per_minute, Some(30));
            assert!(plan.config.auth_methods.is_empty());
        }
        let plan = plan_conversion(&legacy("session_per_invocation"), None);
        assert_eq!(plan.config.session_binding, SessionBinding::Ephemeral);
    }

    #[test]
    fn identity_providers_carry_over_without_the_key() {
        let oidc = auth(json!({
            "mode": "oidc",
            "provider": {"type": "oidc", "issuer": "https://login.example.com"},
            "requirements": {"audiences": ["api://agent"]}
        }));
        let plan = plan_conversion(&legacy("shared_session"), Some(&oidc));
        assert_eq!(plan.disabled_because, None);
        assert_eq!(plan.key, None, "the key was never accepted under OIDC");
        assert_eq!(plan.config.auth_methods, vec![oidc]);
    }

    #[test]
    fn methods_without_an_equivalent_convert_disabled() {
        for mode in [
            json!({"mode": "anonymous"}),
            json!({"mode": "shared_secret"}),
            json!({"mode": "mtls"}),
            json!({"mode": "http_basic", "provider": {"type": "http_basic", "username": "u", "password_hash": "h"}}),
        ] {
            let plan = plan_conversion(&legacy("shared_session"), Some(&auth(mode.clone())));
            assert!(plan.disabled_because.is_some(), "{mode}");
            assert_eq!(plan.key, None, "{mode}");
            assert!(plan.config.auth_methods.is_empty(), "{mode}");
        }
        let plan = plan_conversion(&json!({"unexpected": true}), None);
        assert!(plan.disabled_because.is_some());
    }

    #[test]
    fn converted_config_is_a_valid_api_channel_config() {
        let plan = plan_conversion(&legacy("shared_session"), None);
        let value = serde_json::to_value(&plan.config).unwrap();
        super::super::validation::normalize_and_validate_channel_config(ChannelType::Api, value)
            .expect("valid api channel config");
    }
}
