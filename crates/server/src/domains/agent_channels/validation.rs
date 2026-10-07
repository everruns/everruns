use super::invocation::{cron_min_interval_seconds, normalize_cron_expression};
use crate::domains::common::{CommandError, classify_anyhow};
use crate::records::PublicToolVisibility;
use crate::records::agent_channel::{ScheduleChannelConfig, WebhookChannelConfig};
use crate::records::{
    A2aChannelConfig, AgUiChannelConfig, ApiChannelConfig, ChannelAuthConfig, ChannelAuthMode,
    ChannelAuthProviderConfig, ChannelType, FcpChannelConfig, PublicChatChannelConfig,
    SlackChannelConfig,
};
use crate::storage::password::hash_password;
use serde_json::Value;
use std::str::FromStr;

fn schedule_channel_min_interval_seconds() -> i64 {
    const DEFAULT: i64 = 300;
    std::env::var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT)
}

pub(crate) fn normalize_and_validate_channel_config(
    channel_type: ChannelType,
    mut channel_config: Value,
) -> Result<Value, CommandError> {
    match channel_type {
        ChannelType::AgUi
        | ChannelType::A2a
        | ChannelType::ApiEndpoint
        | ChannelType::PublicChat => {
            normalize_inline_channel_auth(&channel_type, &mut channel_config)?;
        }
        ChannelType::Fcp | ChannelType::Slack | ChannelType::Schedule | ChannelType::Webhook => {
            // FCP deliberately runs its own minimal auth stack (anonymous +
            // shared bearer token) so it never shares verifier code with
            // AG-UI/A2A. See `knowledge/integrations/fcp-channel.md`.
            if channel_config.get("auth").is_some() {
                return Err(CommandError::bad_request(format!(
                    "Channel auth is not supported for {channel_type} channels"
                )));
            }
        }
    }
    match channel_type {
        ChannelType::Slack => {
            // The channel must exist before its manifest can be generated, while
            // Slack provides credentials only after the app is created.
            let config = serde_json::from_value::<SlackChannelConfig>(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid Slack channel config: {e}"))
                })?;
            validate_session_binding(&channel_type, config.session_strategy)?;
        }
        ChannelType::AgUi => {
            let config: AgUiChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid AG-UI channel config: {e}"))
                })?;
            // Reject obviously broken caps (e.g. > 1M req/min) so a typo can't
            // silently disable the per-channel limit by overflowing reasonable
            // expectations. `0` is allowed and means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "AG-UI rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if let Some(token) = config.token
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI token must be non-empty when configured",
                ));
            }
            if config.generic_tool_text.chars().count() > 120 {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text must be at most 120 characters",
                ));
            }
            // Narrated also emits the configured generic_tool_text on public streams (the
            // raw narration is intentionally not forwarded), so non-empty text is required
            // for both Generic and Narrated.
            if matches!(
                config.tool_visibility,
                PublicToolVisibility::Generic | PublicToolVisibility::Narrated
            ) && config.generic_tool_text.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text cannot be empty when tool_visibility is generic or narrated",
                ));
            }
        }
        ChannelType::Schedule => {
            let config: ScheduleChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid schedule channel config: {e}"))
                })?;
            validate_session_binding(&channel_type, config.session_mode)?;
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Schedule channel config requires a non-empty message",
                ));
            }
            let normalized = normalize_cron_expression(&config.cron_expression)?;
            // Enforce minimum cron interval.
            let schedule = cron::Schedule::from_str(&normalized).expect("already validated");
            let min_limit = schedule_channel_min_interval_seconds();
            if let Some(interval) = cron_min_interval_seconds(&schedule, min_limit)
                && interval < min_limit
            {
                return Err(CommandError::bad_request(format!(
                    "Schedule channel cron must fire no more than once every {min_limit} seconds (≥ {} min); expression fires every {interval} seconds",
                    min_limit / 60
                )));
            }
            if let Some(map) = channel_config.as_object_mut() {
                map.insert("cron_expression".to_string(), Value::String(normalized));
            }
        }
        ChannelType::Webhook => {
            let config: WebhookChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid webhook channel config: {e}"))
                })?;
            validate_session_binding(&channel_type, config.session_mode)?;
            if config.token.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty token",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty message",
                ));
            }
        }
        ChannelType::A2a => {
            let config: A2aChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid A2A channel config: {e}"))
                })?;
            validate_session_binding(&channel_type, config.session_mode)?;
            if config.api_key_hash.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_hash",
                ));
            }
            if config.api_key_prefix.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_prefix",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty message",
                ));
            }
            // Mirror the AG-UI cap so a typo in `rate_limit_per_minute` cannot
            // silently disable the per-channel limit by overflowing reasonable
            // expectations. `0` is allowed and means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "A2A rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            // Whitespace-only `signing_secret` is almost certainly an
            // accidental write — reject it so the channel does not silently
            // start enforcing a signature with a useless key. `None` keeps
            // the current API-key-only behavior; an explicit non-empty
            // value opts the channel into HMAC replay protection
            // (TM-A2A-010). Cap the length so a single channel write
            // cannot bloat the encrypted column.
            if let Some(secret) = config.signing_secret.as_deref() {
                if secret.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "A2A signing_secret must be non-empty when configured",
                    ));
                }
                if secret.len() > 4096 {
                    return Err(CommandError::bad_request(
                        "A2A signing_secret must be at most 4096 bytes",
                    ));
                }
            }
            if let Some(pact) = config.pact.as_ref() {
                validate_pact_profile(pact)?;
            }
        }
        ChannelType::ApiEndpoint => {
            let config: ApiChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid api_endpoint channel config: {e}"))
                })?;
            validate_session_binding(&channel_type, config.session_mode)?;
            if config.api_key_hash.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "api_endpoint channel config requires a non-empty api_key_hash",
                ));
            }
            if config.api_key_prefix.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "api_endpoint channel config requires a non-empty api_key_prefix",
                ));
            }
            // Mirror the A2A/AG-UI cap so a typo cannot silently disable the
            // per-channel limit by overflowing reasonable expectations. `0` is
            // allowed and means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "api_endpoint rate_limit_per_minute must be at most 1,000,000",
                ));
            }
        }
        ChannelType::Fcp => {
            let config: FcpChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid FCP channel config: {e}"))
                })?;
            if let Some(token) = config.token.as_deref()
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "FCP token must be non-empty when configured",
                ));
            }
            if let Some(handshake) = config.handshake.as_deref()
                && handshake.len() > 8 * 1024
            {
                return Err(CommandError::bad_request(
                    "FCP handshake must be at most 8 KiB",
                ));
            }
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "FCP rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if config.response_timeout_seconds == 0 || config.response_timeout_seconds > 600 {
                return Err(CommandError::bad_request(
                    "FCP response_timeout_seconds must be between 1 and 600",
                ));
            }
        }
        ChannelType::PublicChat => {
            let config: PublicChatChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid Public Chat channel config: {e}"))
                })?;
            // Mirror the AG-UI cap so a typo cannot silently disable the
            // per-channel limit by overflowing reasonable expectations. `0`
            // means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "Public Chat rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if let Some(token) = config.token.as_deref()
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "Public Chat token must be non-empty when configured",
                ));
            }
            if config.generic_tool_text.chars().count() > 120 {
                return Err(CommandError::bad_request(
                    "Public Chat generic_tool_text must be at most 120 characters",
                ));
            }
            if matches!(
                config.tool_visibility,
                PublicToolVisibility::Generic | PublicToolVisibility::Narrated
            ) && config.generic_tool_text.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "Public Chat generic_tool_text cannot be empty when tool_visibility is generic or narrated",
                ));
            }
            // An anonymous channel with no auth and no captcha is allowed (the
            // simplest "anyone with the link" case), but a captcha config must
            // be coherent: a non-empty site key, and a secret key on first
            // configuration (PATCH may omit it to preserve the stored value).
            if let Some(captcha) = config.captcha.as_ref() {
                if captcha.site_key.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha requires a non-empty site_key",
                    ));
                }
                if let Some(secret) = captcha.secret_key.as_deref()
                    && secret.trim().is_empty()
                {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha secret_key must be non-empty when configured",
                    ));
                }
                // An enabled captcha with no stored secret would fail closed at
                // runtime (every anonymous request → 503). Require the secret
                // when enabled. On PATCH the existing secret is merged in before
                // this check, so editing other fields keeps working.
                let has_secret = captcha
                    .secret_key
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty());
                if captcha.enabled && !has_secret {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha requires a secret_key when enabled",
                    ));
                }
            }
            // Branding sanity: cap the free-text fields so a single channel
            // write cannot bloat the encrypted config column.
            if let Some(name) = config.branding.display_name.as_deref()
                && name.chars().count() > 120
            {
                return Err(CommandError::bad_request(
                    "Public Chat branding display_name must be at most 120 characters",
                ));
            }
            if let Some(welcome) = config.branding.welcome_message.as_deref()
                && welcome.chars().count() > 2000
            {
                return Err(CommandError::bad_request(
                    "Public Chat branding welcome_message must be at most 2000 characters",
                ));
            }
        }
    }

    Ok(channel_config)
}

fn validate_session_binding(
    channel_type: &ChannelType,
    binding: everruns_core::channel::SessionBinding,
) -> Result<(), CommandError> {
    if channel_type.allows_binding(binding) {
        Ok(())
    } else {
        Err(CommandError::bad_request(format!(
            "Session binding {binding:?} is not supported for {channel_type} channels"
        )))
    }
}

fn hash_channel_basic_password(password: &str) -> Result<String, CommandError> {
    hash_password(password).map_err(classify_anyhow)
}

fn normalize_inline_channel_auth(
    channel_type: &ChannelType,
    channel_config: &mut Value,
) -> Result<(), CommandError> {
    let Some(auth_value) = channel_config.get("auth") else {
        return Ok(());
    };
    if auth_value.is_null() {
        return Ok(());
    }
    let auth: ChannelAuthConfig = serde_json::from_value(auth_value.clone())
        .map_err(|e| CommandError::bad_request(format!("Invalid channel auth config: {e}")))?;
    validate_channel_auth_config(channel_type, channel_config, &auth)?;

    if let Some(provider) = channel_config
        .get_mut("auth")
        .and_then(|auth| auth.get_mut("provider"))
        .and_then(Value::as_object_mut)
    {
        for configured_flag in [
            "client_secret_configured",
            "password_configured",
            "proxy_secret_configured",
        ] {
            provider.remove(configured_flag);
        }
        if provider.get("type").and_then(Value::as_str) == Some("http_basic") {
            let password = provider
                .remove("password")
                .and_then(|value| value.as_str().map(str::to_owned));
            if let Some(password) = password {
                if password.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "HTTP Basic password must be non-empty when configured",
                    ));
                }
                provider.insert(
                    "password_hash".to_string(),
                    Value::String(hash_channel_basic_password(&password)?),
                );
            }
        }
    }
    Ok(())
}

fn validate_channel_auth_config(
    channel_type: &ChannelType,
    channel_config: &Value,
    auth: &ChannelAuthConfig,
) -> Result<(), CommandError> {
    match auth.mode {
        ChannelAuthMode::Anonymous => Ok(()),
        ChannelAuthMode::SharedSecret => {
            if *channel_type != ChannelType::AgUi && *channel_type != ChannelType::PublicChat {
                return Err(CommandError::bad_request(
                    "Shared token auth is only supported for AG-UI and Public Chat channels",
                ));
            }
            if channel_config
                .get("token")
                .and_then(Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
            {
                Ok(())
            } else {
                Err(CommandError::bad_request(
                    "Shared token auth requires a non-empty token",
                ))
            }
        }
        ChannelAuthMode::ApiKey => {
            if *channel_type != ChannelType::A2a && *channel_type != ChannelType::ApiEndpoint {
                return Err(CommandError::bad_request(
                    "API key auth is only supported for A2A and api_endpoint channels",
                ));
            }
            let has_hash = channel_config
                .get("api_key_hash")
                .and_then(Value::as_str)
                .is_some_and(|hash| !hash.trim().is_empty());
            let has_prefix = channel_config
                .get("api_key_prefix")
                .and_then(Value::as_str)
                .is_some_and(|prefix| !prefix.trim().is_empty());
            if has_hash && has_prefix {
                Ok(())
            } else {
                Err(CommandError::bad_request(
                    "API key auth requires a configured API key",
                ))
            }
        }
        ChannelAuthMode::GoogleOidc => match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::GoogleOidc { client_id, .. })
                if !client_id.trim().is_empty() =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "Google auth requires provider.type=google_oidc and non-empty client_id",
            )),
        },
        ChannelAuthMode::Oidc => match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::Oidc { issuer, jwks_url }) => {
                if issuer.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "OIDC auth requires a non-empty issuer",
                    ));
                }
                everruns_contracts::url_validation::validate_safe_url(issuer)
                    .map_err(|e| CommandError::bad_request(format!("Invalid OIDC issuer: {e}")))?;
                if let Some(jwks_url) = jwks_url {
                    everruns_contracts::url_validation::validate_safe_url(jwks_url).map_err(
                        |e| CommandError::bad_request(format!("Invalid OIDC JWKS URL: {e}")),
                    )?;
                }
                // Every OIDC token must name its audience, and an AgentID token
                // names the operator's registered client id. Reject at save time
                // what would otherwise fail every request as misconfigured.
                if auth.is_agentid()
                    && !auth
                        .requirements
                        .audiences
                        .iter()
                        .any(|audience| !audience.trim().is_empty())
                {
                    return Err(CommandError::bad_request(
                        "AgentID auth requires requirements.audiences to hold your AgentID client id",
                    ));
                }
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "OIDC auth requires provider.type=oidc",
            )),
        },
        ChannelAuthMode::OAuth2Introspection => match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::OAuth2Introspection {
                introspection_url, ..
            }) => {
                everruns_contracts::url_validation::validate_safe_url(introspection_url).map_err(
                    |e| CommandError::bad_request(format!("Invalid OAuth2 introspection URL: {e}")),
                )?;
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "OAuth2 introspection auth requires provider.type=oauth2_introspection",
            )),
        },
        ChannelAuthMode::HttpBasic => match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::HttpBasic {
                username,
                password,
                password_hash,
                ..
            }) if !username.trim().is_empty()
                && (password.as_deref().is_some_and(|p| !p.trim().is_empty())
                    || password_hash
                        .as_deref()
                        .is_some_and(|hash| !hash.trim().is_empty())) =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "HTTP Basic auth requires provider.type=http_basic, username, and password or password_hash",
            )),
        },
        ChannelAuthMode::Mtls => match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::Mtls {
                header_name,
                allowed_values,
                proxy_secret_header,
                proxy_secret,
                ..
            }) if !header_name.trim().is_empty()
                && !allowed_values.is_empty()
                && proxy_secret_header
                    .as_deref()
                    .is_some_and(|h| !h.trim().is_empty())
                && proxy_secret
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty()) =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "mTLS auth requires provider.type=mtls, header_name, allowed_values, proxy_secret_header, and proxy_secret",
            )),
        },
    }
}

pub(crate) fn merge_preserved_secret_fields(
    channel_type: ChannelType,
    final_channel_config: &mut Value,
    existing_decrypted: &Value,
) {
    let (Some(out), Some(existing)) = (
        final_channel_config.as_object_mut(),
        existing_decrypted.as_object(),
    ) else {
        return;
    };

    match channel_type {
        ChannelType::Slack => {
            // Settings edits must not discard the app's install credentials or delivery evidence.
            // THREAT[TM-SLACK-010]: a PATCH cannot invent or replace server-owned install metadata.
            for key in [
                "provisioned_app",
                "webhook_verified_at",
                "first_message_received_at",
            ] {
                out.remove(key);
                if let Some(value) = existing.get(key) {
                    out.insert(key.to_string(), value.clone());
                }
            }
            for key in [
                "agent_surface_enabled",
                "tool_visibility",
                "generic_tool_text",
            ] {
                if !out.contains_key(key)
                    && let Some(value) = existing.get(key)
                {
                    out.insert(key.to_string(), value.clone());
                }
            }
            for key in ["signing_secret", "bot_token"] {
                let should_preserve = out
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(str::is_empty);
                if should_preserve && let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
        }
        ChannelType::AgUi => {
            if !out.contains_key("token")
                && let Some(existing_value) = existing.get("token")
            {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::Webhook => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::A2a => {
            for key in ["api_key_hash", "api_key_prefix"] {
                if let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
            // signing_secret is write-only on the wire — preserve the
            // existing value across PATCH so an operator editing the
            // session-mode / message / rate-limit field does not also
            // disable replay protection by omission (TM-A2A-010).
            if !out.contains_key("signing_secret")
                && let Some(existing_value) = existing.get("signing_secret")
            {
                out.insert("signing_secret".to_string(), existing_value.clone());
            }
        }
        ChannelType::Fcp => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::ApiEndpoint => {
            // api_key_hash is write-only on the wire (redacted on read), so a
            // PATCH that edits session_mode / rate_limit must preserve the
            // existing frozen key rather than wipe it.
            for key in ["api_key_hash", "api_key_prefix"] {
                if let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
        }
        ChannelType::PublicChat => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
            // Preserve the write-only Turnstile secret across a PATCH that
            // edits other captcha fields (e.g. toggling `enabled` or rotating
            // the site key) so the operator does not silently disable
            // verification by omitting the secret.
            if let (Some(out_captcha), Some(existing_captcha)) = (
                out.get_mut("captcha").and_then(Value::as_object_mut),
                existing.get("captcha").and_then(Value::as_object),
            ) {
                let should_preserve = out_captcha
                    .get("secret_key")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(str::is_empty);
                if should_preserve && let Some(existing_secret) = existing_captcha.get("secret_key")
                {
                    out_captcha.insert("secret_key".to_string(), existing_secret.clone());
                }
            }
        }
        ChannelType::Schedule => {}
    }
    merge_preserved_channel_auth_secrets(final_channel_config, existing_decrypted);
}

fn merge_preserved_channel_auth_secrets(
    final_channel_config: &mut Value,
    existing_decrypted: &Value,
) {
    let (Some(out_provider), Some(existing_provider)) = (
        final_channel_config
            .get_mut("auth")
            .and_then(|auth| auth.get_mut("provider"))
            .and_then(Value::as_object_mut),
        existing_decrypted
            .get("auth")
            .and_then(|auth| auth.get("provider"))
            .and_then(Value::as_object),
    ) else {
        return;
    };

    let same_type = out_provider.get("type").and_then(Value::as_str)
        == existing_provider.get("type").and_then(Value::as_str);
    if !same_type {
        return;
    }
    for key in ["password_hash", "client_secret", "proxy_secret"] {
        let should_preserve = out_provider
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(str::is_empty);
        if should_preserve && let Some(existing_value) = existing_provider.get(key) {
            out_provider.insert(key.to_string(), existing_value.clone());
        }
    }
}

/// Most personal agents one PACT endpoint trusts, and the largest inline JWKS
/// it stores per agent. Both bound what one channel write can store.
const MAX_PACT_PERSONAL_AGENTS: usize = 50;
const MAX_PACT_INLINE_JWKS_BYTES: usize = 16 * 1024;

fn validate_pact_profile(
    pact: &crate::records::agent_channel::PactProfileConfig,
) -> Result<(), CommandError> {
    let invalid = |message: &str| Err(CommandError::bad_request(format!("A2A pact: {message}")));
    if pact.audience.trim().is_empty() {
        return invalid("audience must be non-empty");
    }
    if pact.personal_agents.is_empty() {
        return invalid("personal_agents must list at least one personal agent");
    }
    if pact.personal_agents.len() > MAX_PACT_PERSONAL_AGENTS {
        return invalid("personal_agents lists more than 50 personal agents");
    }
    for agent in &pact.personal_agents {
        if agent.issuer.trim().is_empty() {
            return invalid("every personal agent needs a non-empty issuer");
        }
        match (agent.jwks_uri.as_deref(), agent.jwks.as_ref()) {
            (Some(uri), None) => {
                // The server fetches this URL, so only HTTPS is stored; the
                // fetch itself still refuses private addresses
                // (`channel_auth::build_pinned_client`).
                if !uri.starts_with("https://") || url::Url::parse(uri).is_err() {
                    return invalid("jwks_uri must be an https URL");
                }
            }
            (None, Some(jwks)) => {
                if serde_json::to_vec(jwks).map_or(0, |bytes| bytes.len())
                    > MAX_PACT_INLINE_JWKS_BYTES
                {
                    return invalid("inline jwks must be at most 16 KiB");
                }
                if serde_json::from_value::<jsonwebtoken::jwk::JwkSet>(jwks.clone()).is_err() {
                    return invalid("jwks must be a JWKS document ({\"keys\": [...]})");
                }
            }
            _ => return invalid("set exactly one of jwks_uri or jwks per personal agent"),
        }
    }
    if let Some(delegation) = &pact.delegation {
        super::validation_pact::validate_delegation(delegation).map_err(|message| {
            CommandError::bad_request(format!("A2A pact.delegation: {message}"))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agentid_preset_requires_a_client_id_audience() {
        let config = json!({});
        let preset = ChannelAuthConfig::agentid_preset("agentid-client");
        assert!(validate_channel_auth_config(&ChannelType::A2a, &config, &preset).is_ok());

        let mut without_audience = preset.clone();
        without_audience.requirements.audiences.clear();
        assert!(
            validate_channel_auth_config(&ChannelType::A2a, &config, &without_audience).is_err()
        );
    }

    #[test]
    fn slack_settings_edits_preserve_connection_evidence_and_provisioning() {
        let existing = json!({
            "signing_secret": "saved-secret",
            "bot_token": "saved-token",
            "provisioned_app": { "app_id": "A123" },
            "webhook_verified_at": "2026-10-03T10:00:00Z",
            "first_message_received_at": "2026-10-03T10:01:00Z",
            "agent_surface_enabled": true,
            "tool_visibility": "none",
            "generic_tool_text": "Working"
        });
        let mut edited = json!({ "session_strategy": "per_channel", "reply_mode": "all_messages" });
        merge_preserved_secret_fields(ChannelType::Slack, &mut edited, &existing);
        for key in [
            "signing_secret",
            "bot_token",
            "provisioned_app",
            "webhook_verified_at",
            "first_message_received_at",
            "agent_surface_enabled",
            "tool_visibility",
            "generic_tool_text",
        ] {
            assert_eq!(edited.get(key), existing.get(key), "lost {key}");
        }
        assert_eq!(edited["session_strategy"], "per_channel");
    }

    #[test]
    fn slack_settings_cannot_replace_server_owned_connection_metadata() {
        let existing = json!({
            "provisioned_app": { "app_id": "A123" },
            "first_message_received_at": "2026-10-03T10:01:00Z",
            "agent_surface_enabled": true
        });
        let mut edited = json!({
            "provisioned_app": { "app_id": "AOTHER" },
            "first_message_received_at": "2026-10-03T11:00:00Z",
            "webhook_verified_at": "2026-10-03T11:00:00Z",
            "agent_surface_enabled": false
        });
        merge_preserved_secret_fields(ChannelType::Slack, &mut edited, &existing);
        assert_eq!(edited["provisioned_app"], existing["provisioned_app"]);
        assert_eq!(
            edited["first_message_received_at"],
            existing["first_message_received_at"]
        );
        assert!(edited.get("webhook_verified_at").is_none());
        assert_eq!(edited["agent_surface_enabled"], false);
    }

    #[test]
    fn rejects_bindings_unsupported_by_the_channel_transport() {
        for binding in [
            "shared_session",
            "session_per_invocation",
            "channel",
            "ephemeral",
        ] {
            let config = json!({ "session_strategy": binding });
            assert!(
                normalize_and_validate_channel_config(ChannelType::Slack, config).is_err(),
                "Slack accepted unsupported binding {binding}"
            );
        }

        let invocation_configs = [
            (
                ChannelType::Schedule,
                json!({ "cron_expression": "0 0 * * * *", "message": "run" }),
            ),
            (
                ChannelType::Webhook,
                json!({ "token": "secret", "message": "run" }),
            ),
            (
                ChannelType::A2a,
                json!({ "api_key_hash": "hash", "api_key_prefix": "prefix", "message": "run" }),
            ),
            (
                ChannelType::ApiEndpoint,
                json!({ "api_key_hash": "hash", "api_key_prefix": "prefix" }),
            ),
        ];
        for (channel_type, base_config) in invocation_configs {
            for binding in ["per_thread", "per_channel", "per_user"] {
                let mut config = base_config.clone();
                config["session_mode"] = json!(binding);
                assert!(
                    normalize_and_validate_channel_config(channel_type.clone(), config).is_err(),
                    "{channel_type} accepted unsupported binding {binding}"
                );
            }
        }
    }
}
