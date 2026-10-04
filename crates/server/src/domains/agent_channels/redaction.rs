// Response redaction for agent channels.
//
// Every read surface that returns a channel (agent channel commands and the
// frozen App archival reads) passes it through `redact_channel_for_response`,
// so write-only secrets never leave the server.

use everruns_platform::{AgentChannel, ChannelType};
use serde_json::{Value, json};

fn redact_channel_config(channel_type: &ChannelType, config: &mut Value) {
    redact_inline_channel_auth(config);
    let Some(map) = config.as_object_mut() else {
        return;
    };
    match channel_type {
        ChannelType::Slack => {
            // The whole object, not selected keys inside it. It carries the
            // OAuth client secret *and* the single-use install nonce, and
            // leaking the nonce would hand a reader exactly what the callback's
            // state check exists to withhold — this redaction runs on keys at
            // this level only, so a nested secret is not covered by the loop
            // below (EVE-1069).
            if let Some(provisioned) = map.remove("provisioned_app") {
                map.insert("slack_app_provisioned".to_string(), Value::Bool(true));
                // The app id is public — it is in every URL of the app's Slack
                // page — and it is what lets the UI link straight into the
                // agent in Slack. Everything else in the object stays withheld.
                if let Some(app_id) = provisioned.get("app_id").and_then(Value::as_str) {
                    map.insert(
                        "slack_app_id".to_string(),
                        Value::String(app_id.to_string()),
                    );
                }
            }
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

fn redact_inline_channel_auth(config: &mut Value) {
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

pub(crate) fn redact_channel_for_response(mut channel: AgentChannel) -> AgentChannel {
    if let Some(auth) = channel.auth.take() {
        let mut wrapped = json!({ "auth": auth });
        redact_inline_channel_auth(&mut wrapped);
        channel.auth = wrapped
            .get_mut("auth")
            .map(Value::take)
            .and_then(|value| serde_json::from_value(value).ok());
    }
    redact_channel_config(&channel.channel_type, &mut channel.channel_config);
    channel
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one-click install fields are nested, and `redact_channel_config`
    /// only walks keys at the top level — so a nested secret is not covered by
    /// the flat loop beside it. This asserts the whole object is dropped.
    ///
    /// The nonce matters as much as the secret: it is what the OAuth callback
    /// compares against, so a reader who could see it could drive the callback
    /// and bind their own workspace to this channel (EVE-1069).
    #[test]
    fn slack_redaction_drops_the_provisioned_app_whole() {
        let mut config = json!({
            "signing_secret": "shhh",
            "bot_token": "xoxb-token",
            "provisioned_app": {
                "app_id": "A0123",
                "client_id": "4567.89",
                "client_secret": "client-secret-value",
                "install_state": "nonce-value",
                "install_state_issued_at": "2026-09-19T00:00:00Z",
            },
        });
        redact_channel_config(&ChannelType::Slack, &mut config);

        let rendered = config.to_string();
        assert!(!rendered.contains("client-secret-value"), "{rendered}");
        assert!(!rendered.contains("nonce-value"), "{rendered}");
        assert!(!rendered.contains("4567.89"), "{rendered}");
        assert!(config.get("provisioned_app").is_none(), "{rendered}");
        assert_eq!(
            config.get("slack_app_provisioned"),
            Some(&Value::Bool(true))
        );
        // The public app id survives, so the UI can link into Slack.
        assert_eq!(config.get("slack_app_id"), Some(&json!("A0123")));
        // The flat secrets keep their existing treatment.
        assert!(!rendered.contains("shhh"), "{rendered}");
        assert_eq!(
            config.get("signing_secret_configured"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn a_hand_configured_channel_gains_no_provisioned_flag() {
        let mut config = json!({ "signing_secret": "shhh" });
        redact_channel_config(&ChannelType::Slack, &mut config);
        assert!(config.get("slack_app_provisioned").is_none());
    }
}
