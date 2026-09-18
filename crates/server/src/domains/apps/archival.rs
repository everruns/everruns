// Deprecated read-only access to frozen App records.

use super::queries as q;
use crate::domains::common::*;
use everruns_core::{Permission, Policy, Rule};
use everruns_platform::{App, AppChannel, ChannelType};
use everruns_provider::typed_id::AppId;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

const APP_VIEW: Policy = Policy {
    id: "app.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAppsView)],
};

fn redact_channel_config(channel_type: &ChannelType, config: &mut Value) {
    redact_inline_endpoint_auth(config);
    let Some(map) = config.as_object_mut() else {
        return;
    };
    match channel_type {
        ChannelType::Slack => {
            for (key, flag) in [
                ("signing_secret", "signing_secret_configured"),
                ("bot_token", "bot_token_configured"),
            ] {
                let removed = map.remove(key);
                let is_configured = removed
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if is_configured {
                    map.insert(flag.to_string(), Value::Bool(true));
                }
            }
        }
        ChannelType::AgUi => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::Webhook => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::A2a => {
            map.remove("api_key_hash");
            // signing_secret is write-only — the API redacts it on read
            // and only surfaces `signing_secret_configured: bool` so
            // operators can tell whether replay protection is on without
            // ever leaking the shared key. Mirrors the Slack /
            // webhook-token redaction pattern (TM-A2A-010). Only set the
            // flag when the stored value is a non-empty string; a
            // `null` / empty value means replay protection is **off**
            // and must not be advertised as configured.
            let removed = map.remove("signing_secret");
            let is_configured = removed
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if is_configured {
                map.insert("signing_secret_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::Fcp => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::ApiEndpoint => {
            // The api_key_hash is a secret-equivalent: anyone who can submit a
            // key whose SHA-256 matches it authenticates. Never surface it on
            // read; the non-secret api_key_prefix stays for display.
            map.remove("api_key_hash");
        }
        ChannelType::PublicChat => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
            // Turnstile secret key is write-only: surface only whether it is
            // configured so the site key (public) can still be returned for the
            // client widget without leaking the verification secret.
            if let Some(captcha) = map.get_mut("captcha").and_then(Value::as_object_mut) {
                let removed = captcha.remove("secret_key");
                let is_configured = removed
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if is_configured {
                    captcha.insert("secret_key_configured".to_string(), Value::Bool(true));
                }
            }
        }
        ChannelType::Schedule => {}
    }
}

fn redact_inline_endpoint_auth(config: &mut Value) {
    let Some(provider) = config
        .get_mut("auth")
        .and_then(|auth| auth.get_mut("provider"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if provider.remove("password").is_some() || provider.remove("password_hash").is_some() {
        provider.insert("password_configured".to_string(), Value::Bool(true));
    }
    if provider.remove("client_secret").is_some() {
        provider.insert("client_secret_configured".to_string(), Value::Bool(true));
    }
    let has_proxy_secret = provider
        .get("proxy_secret")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    provider.remove("proxy_secret");
    if has_proxy_secret {
        provider.insert("proxy_secret_configured".to_string(), Value::Bool(true));
    }
}

fn redact_channel_for_response(mut channel: AppChannel) -> AppChannel {
    if let Some(auth) = channel.auth.take() {
        let mut wrapped = json!({ "auth": auth });
        redact_inline_endpoint_auth(&mut wrapped);
        channel.auth = wrapped
            .get_mut("auth")
            .map(Value::take)
            .and_then(|value| serde_json::from_value(value).ok());
    }
    redact_channel_config(&channel.channel_type, &mut channel.channel_config);
    channel
}

fn redact_app_for_response(mut app: App) -> App {
    app.channels = app
        .channels
        .into_iter()
        .map(redact_channel_for_response)
        .collect();
    app
}

/// List frozen App records for archival access.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListApps {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListApps {
    type Output = Vec<App>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_apps",
            category: "apps",
            description: "List frozen App records for archival access.",
            method: "GET",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<App>, CommandError> {
        let rows = ctx
            .db
            .list_apps(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(
            q::load_apps_list(&ctx.db, ctx.encryption.as_ref(), rows, ctx.org_id())
                .await
                .map_err(classify_anyhow)?
                .into_iter()
                .map(redact_app_for_response)
                .collect(),
        )
    }
}

/// Get one frozen App record for archival access.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetApp {
    pub id: String,
}

impl Command for GetApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_app",
            category: "apps",
            description: "Get one frozen App record for archival access.",
            method: "GET",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        q::get_by_public_id(
            &ctx.db,
            ctx.encryption.as_ref(),
            ctx.org_id(),
            &app_id.to_string(),
        )
        .await
        .map_err(classify_anyhow)?
        .map(redact_app_for_response)
        .ok_or_else(|| CommandError::not_found("App"))
    }
}
