//! Slack Web API calls and request-signature verification.

use axum::http::HeaderMap;
use hmac::{KeyInit, Mac};

use super::*;

pub(crate) const SLACK_API_BASE: &str = "https://slack.com/api";

/// Resolve a Slack user ID to a display name via `users.info` API.
///
/// Returns the display name (or real_name fallback) on success, or `None` if
/// resolution fails (missing `users:read` scope, network error, etc.).
/// Results are cached: successful lookups and permanent failures (missing scope)
/// are not retried. Transient errors are retried on next call.
pub(crate) async fn resolve_slack_user_name(
    cache: &SlackUserCache,
    bot_token: &str,
    user_id: &str,
) -> Option<String> {
    resolve_slack_user_name_base(SLACK_API_BASE, cache, bot_token, user_id).await
}

/// Resolve a Slack user ID to a display name (with configurable base URL for testing).
pub(crate) async fn resolve_slack_user_name_base(
    base_url: &str,
    cache: &SlackUserCache,
    bot_token: &str,
    user_id: &str,
) -> Option<String> {
    // Check cache first
    if let Some(cached) = cache.get(user_id) {
        return cached;
    }

    // Call Slack API (user IDs are alphanumeric, safe to interpolate)
    let client = reqwest::Client::new();
    let url = format!("{}/users.info?user={user_id}", base_url);
    let result = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", bot_token))
        .send()
        .await;

    let response: reqwest::Response = match result {
        Ok(r) => r,
        Err(e) => {
            // Transient network error — don't cache, allow retry
            tracing::warn!(user_id = user_id, error = %e, "Failed to fetch Slack user info (network)");
            return None;
        }
    };

    let body: serde_json::Value = match response.json().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(user_id = user_id, error = %e, "Failed to parse Slack users.info response");
            return None;
        }
    };

    if body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        // Success — extract display_name, fall back to real_name, then name
        let user_obj = body.get("user");
        let profile = user_obj.and_then(|u| u.get("profile"));
        let display_name = profile
            .and_then(|p| p.get("display_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                user_obj
                    .and_then(|u| u.get("real_name"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                user_obj
                    .and_then(|u| u.get("name"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);

        cache.insert(user_id.to_string(), display_name.clone());
        display_name
    } else {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            user_id = user_id,
            error = error,
            "Slack users.info API error"
        );

        // Permanent errors (missing scope, invalid token) — cache as None to avoid repeated calls
        if error == "missing_scope"
            || error == "not_authed"
            || error == "invalid_auth"
            || error == "token_revoked"
            || error == "account_inactive"
        {
            cache.insert(user_id.to_string(), None);
        }

        None
    }
}

/// Post a message to Slack using the Bot API.
/// Delegates to the shared implementation in `slack_delivery`.
pub(crate) async fn post_to_slack(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> anyhow::Result<()> {
    crate::slack_delivery::post_to_slack(bot_token, channel, thread_ts, text).await
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
    // An empty HMAC key is public, so it cannot authenticate a request.
    if signing_secret.trim().is_empty() {
        return Err("Slack channel has no signing secret configured".to_string());
    }

    let timestamp = headers
        .get("X-Slack-Request-Timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing X-Slack-Request-Timestamp header")?;

    let signature = headers
        .get("X-Slack-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing X-Slack-Signature header")?;

    // Reject requests older than 5 minutes to prevent replay attacks
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).unsigned_abs() > 300 {
            return Err("Request timestamp too old".to_string());
        }
    }

    // Compute expected signature
    let sig_basestring = format!("v0:{}:{}", timestamp, String::from_utf8_lossy(body));
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|e| format!("HMAC key error: {}", e))?;
    mac.update(sig_basestring.as_bytes());
    let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

    // Constant-time comparison
    if expected.len() != signature.len() {
        return Err("Signature mismatch".to_string());
    }
    let matches = expected
        .as_bytes()
        .iter()
        .zip(signature.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if matches != 0 {
        return Err("Signature mismatch".to_string());
    }

    Ok(())
}

// =========================================================================
// Slack App Manifest generation — per-channel "Create in Slack" helper
// =========================================================================
