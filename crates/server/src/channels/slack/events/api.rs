//! Slack Web API calls and request-signature verification.

use axum::http::HeaderMap;

#[cfg(test)]
use super::SlackUserCache;

pub(crate) use everruns_integrations::slack_channel::web_api::SLACK_API_BASE;

/// Resolve a Slack user ID to a display name (with configurable base URL for testing).
#[cfg(test)]
pub(crate) async fn resolve_slack_user_name_base(
    base_url: &str,
    cache: &SlackUserCache,
    bot_token: &str,
    user_id: &str,
) -> Option<String> {
    let cache_key = crate::channels::slack::provider::cache_key(base_url, bot_token, user_id);
    if let Some(cached) = cache.get(&cache_key) {
        return cached;
    }
    let body = match everruns_integrations::slack_channel::web_api::slack_api_call(
        base_url,
        bot_token,
        "users.info",
        serde_json::json!({"user": user_id}),
    )
    .await
    {
        Ok(body) => body,
        Err(error) => {
            if matches!(
                error.code(),
                Some(
                    "missing_scope"
                        | "not_authed"
                        | "invalid_auth"
                        | "token_revoked"
                        | "account_inactive"
                )
            ) {
                cache.insert(cache_key, None);
            }
            return None;
        }
    };
    {
        let display_name = super::super::provider::display_name(&body["user"]);
        cache.insert(cache_key, display_name.clone());
        display_name
    }
}

/// Verify Slack request signature using HMAC-SHA256.
///
/// Slack signs requests with:
///   sig_basestring = "v0:{timestamp}:{body}"
///   signature = "v0=" + HMAC-SHA256(signing_secret, sig_basestring)
///
/// Headers used:
///   - X-Slack-Request-Timestamp
///   - X-Slack-Signature
pub(crate) fn verify_slack_signature(
    headers: &HeaderMap,
    body: &[u8],
    signing_secret: &str,
) -> Result<(), String> {
    if signing_secret.trim().is_empty() {
        return Err("Slack signing secret unavailable".into());
    }
    let request = channelkit::WebhookRequest {
        method: "POST".into(),
        query: Default::default(),
        headers: headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.into()))
            })
            .collect(),
        body: body.to_vec(),
    };
    channelkit_slack::verify_signature(
        signing_secret,
        &request,
        chrono::Utc::now().timestamp().max(0) as u64,
    )
    .map_err(|_| "Invalid Slack signature or replay timestamp".into())
}

// =========================================================================
// Slack App Manifest generation — per-channel "Create in Slack" helper
// =========================================================================
