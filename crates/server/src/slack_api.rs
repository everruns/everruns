//! The Slack Web API calls that are not `chat.postMessage`'s own loop.
//!
//! Split out of `slack_delivery` (EVE-1026): the delivery adapter decides *what*
//! to say in a thread, and this decides *how* to say it to Slack. Two callers
//! now need the second half — delivery and the task summary — and
//! `slack_delivery` is on the file-size ratchet.
//!
//! Every method here shares one envelope: `ok: false` with an `error` code, and
//! a `Retry-After` header on a rate limit. Decision therefore lives in one
//! place (EVE-974) rather than being rewritten per endpoint.

use crate::slack_api_error::{SlackApiError, parse_retry_after};
use tracing::{debug, error};

/// Slack Web API base.
pub(crate) const SLACK_API_BASE: &str = "https://slack.com/api";

/// Post one message and return its `ts`.
///
/// The status summary needs the `ts` back so it can update that message rather
/// than posting a second one; the delivery path's own post loop discards it
/// because it may split a reply across several calls.
pub(crate) async fn post_slack_message_returning_ts(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
) -> Result<String, SlackApiError> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
        "blocks": [{ "type": "markdown", "text": text }],
    });
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::json!(thread_ts);
    }
    let body = slack_api_call(SLACK_API_BASE, bot_token, "chat.postMessage", payload).await?;
    body.get("ts")
        .and_then(|ts| ts.as_str())
        .map(str::to_string)
        .ok_or_else(|| SlackApiError::Transient("chat.postMessage returned no ts".to_string()))
}

/// Rewrite a message's text in place.
pub(crate) async fn update_slack_message_text(
    bot_token: &str,
    channel: &str,
    ts: &str,
    text: &str,
) -> Result<(), SlackApiError> {
    slack_api_call(
        SLACK_API_BASE,
        bot_token,
        "chat.update",
        serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
            "blocks": [{ "type": "markdown", "text": text }],
        }),
    )
    .await?;
    Ok(())
}

/// Call one Slack Web API method that is not `chat.postMessage`.
///
/// The streaming methods share the same envelope — `ok: false` plus an `error`
/// code, and a `Retry-After` header on a rate limit — so error handling lives
/// here rather than being rewritten per endpoint (EVE-974). Posting keeps its
/// own loop above, which splits one reply across several calls and logs each
/// part.
pub(crate) async fn slack_api_call(
    base_url: &str,
    bot_token: &str,
    method: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, SlackApiError> {
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/{}", base_url, method))
        .header("Authorization", format!("Bearer {}", bot_token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    let status = response.status();
    // Read the header before the body is consumed: a 429 carries its advice here,
    // not in the JSON.
    let retry_after = parse_retry_after(response.headers());

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| SlackApiError::Transient(e.to_string()))?;

    if !body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown");

        let failure = SlackApiError::from_code(error, retry_after);

        if matches!(failure, SlackApiError::RateLimited { .. }) {
            debug!(
                method = method,
                retry_after_secs = ?retry_after.map(|d| d.as_secs()),
                status = %status,
                "Slack rate limited the call"
            );
        } else {
            error!(
                method = method,
                error = error,
                status = %status,
                "Slack API call failed"
            );
        }
        return Err(failure);
    }

    Ok(body)
}

/// Post a Block Kit message into a thread.
///
/// `text` is the notification/fallback string: a blocks-only post reaches push
/// notifications and screen readers as an empty message.
pub(crate) async fn post_slack_blocks(
    bot_token: &str,
    channel: &str,
    thread_ts: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<(), SlackApiError> {
    let mut payload = serde_json::json!({
        "channel": channel,
        "text": text,
        "blocks": blocks,
    });
    if !thread_ts.is_empty() {
        payload["thread_ts"] = serde_json::json!(thread_ts);
    }
    slack_api_call(SLACK_API_BASE, bot_token, "chat.postMessage", payload).await?;
    Ok(())
}

/// Rewrite a message's blocks, e.g. to retire an answered approval card.
///
/// `text` is the notification/fallback string; `blocks` is what the thread
/// renders. Both are required — a `chat.update` that sends blocks without text
/// leaves push notifications and accessibility clients with nothing to read.
pub(crate) async fn update_slack_message_blocks(
    bot_token: &str,
    channel: &str,
    ts: &str,
    text: &str,
    blocks: &serde_json::Value,
) -> Result<(), SlackApiError> {
    slack_api_call(
        SLACK_API_BASE,
        bot_token,
        "chat.update",
        serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
            "blocks": blocks,
        }),
    )
    .await?;
    Ok(())
}
