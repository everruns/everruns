//! Inbound Slack events: resolve the channel, verify, acknowledge, and hand
//! the event to the channel host (`channels::slack::host`).

use crate::domains::agent_channels::record::ChannelType;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use everruns_contracts::runtime::channel_driver::ChannelRequest;
use everruns_core::channel::{InboundAttachment, InboundChannelEvent};
use std::collections::HashMap;

use crate::api::common::ErrorResponse;
use crate::channels::host::ServerChannelScope;
use crate::middleware::RequestId;

use super::*;

/// Parse a Slack webhook event into a platform-agnostic `InboundChannelEvent`.
///
/// Translates Slack-specific structures (event, files, attachments) into the
/// generic channel abstraction types. The resulting event can be routed via
/// `build_session_routing_tag()` and processed by the generic adapter lifecycle.
pub(crate) fn parse_slack_inbound_event(
    event: &SlackEvent,
    slack_config: &SlackChannelConfig,
    display_name: Option<String>,
) -> InboundChannelEvent {
    let slack_user_id = event.user.clone().unwrap_or_default();

    // Build ExternalActor
    let mut actor_metadata = std::collections::HashMap::new();
    if let Some(ref channel) = event.channel {
        actor_metadata.insert("channel".to_string(), channel.clone());
    }
    if let Some(ref team_id) = slack_config.team_id {
        actor_metadata.insert("team_id".to_string(), team_id.clone());
    }

    let actor = everruns_core::ExternalActor {
        actor_id: slack_user_id,
        actor_name: display_name,
        source: "slack".to_string(),
        metadata: if actor_metadata.is_empty() {
            None
        } else {
            Some(actor_metadata)
        },
    };

    // Build attachments from files + legacy attachments
    let mut attachments = Vec::new();
    for file in &event.files {
        let mime = file.mimetype.as_deref().unwrap_or("");
        let name = file.name.as_deref().unwrap_or("unnamed file");
        if SUPPORTED_IMAGE_TYPES.contains(&mime)
            && let Some(url) = &file.url_private
            && let Some(url) = validated_image_url(url)
        {
            attachments.push(InboundAttachment::Image {
                url,
                alt_text: Some(name.to_string()),
            });
            continue;
        }
        attachments.push(InboundAttachment::FileDescription {
            name: name.to_string(),
            mime_type: Some(mime.to_string()),
        });
    }
    for att in &event.attachments {
        if let Some(url) = &att.image_url
            && let Some(url) = validated_image_url(url)
        {
            attachments.push(InboundAttachment::Image {
                url,
                alt_text: att.title.clone(),
            });
        }
    }

    // Build routing metadata for build_session_routing_tag()
    let mut routing_metadata = HashMap::new();
    // thread_ref: use thread_ts if threaded, else message ts (for new thread)
    let thread_ref = event
        .thread_ts
        .as_deref()
        .or(event.ts.as_deref())
        .unwrap_or("unknown");
    routing_metadata.insert("thread_ref".to_string(), thread_ref.to_string());
    if let Some(ref channel) = event.channel {
        routing_metadata.insert("channel_id".to_string(), channel.clone());
    }
    if let Some(ref user) = event.user {
        routing_metadata.insert("user_id".to_string(), user.clone());
    }

    InboundChannelEvent {
        actor,
        text: event.text.clone().unwrap_or_default(),
        attachments,
        dedup_key: event.ts.clone().unwrap_or_default(),
        thread_ref: Some(thread_ref.to_string()),
        routing_metadata,
        message_metadata: Default::default(),
    }
}

pub(crate) async fn resolve_slack_channel(
    state: &SlackState,
    target: SlackTarget,
) -> Result<
    (
        crate::api::channel_ingress::IngressContext,
        crate::api::channel_ingress::IngressChannel,
    ),
    (StatusCode, Json<ErrorResponse>),
> {
    let (app, channel) = match target {
        SlackTarget::LegacyApp(app_id) => {
            match crate::api::channel_ingress::resolve_legacy_channel(
                &state.db,
                state.encryption.as_ref(),
                &app_id,
                ChannelType::Slack,
            )
            .await
            .map_err(|error| {
                tracing::error!(app_id, %error, "Failed to lookup Slack alias");
                ErrorResponse::new("Internal server error")
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })? {
                crate::api::channel_ingress::LegacyChannelMatch::One(endpoint) => *endpoint,
                crate::api::channel_ingress::LegacyChannelMatch::NotFound => {
                    return Err(
                        ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND)
                    );
                }
                crate::api::channel_ingress::LegacyChannelMatch::Ambiguous => {
                    return Err(ErrorResponse::new(
                        "Multiple enabled Slack channels; use an endpoint-scoped /v1/channels/{channel_id}/slack/... URL",
                    )
                    .into_response(StatusCode::CONFLICT));
                }
            }
        }
        SlackTarget::Channel(channel_id) => crate::api::channel_ingress::resolve_channel(
            &state.db,
            state.encryption.as_ref(),
            &channel_id,
        )
        .await
        .map_err(|error| {
            tracing::error!(channel_id, %error, "Failed to lookup Slack endpoint");
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND))?,
    };
    if channel.channel_type != ChannelType::Slack {
        return Err(ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND));
    }
    if let Err(reason) = crate::api::channel_ingress::channel_liveness(&app, &channel) {
        tracing::debug!(
            app_id = %app.public_id,
            channel_id = %channel.public_id,
            reason = reason.as_str(),
            "Slack ingress rejected: endpoint not live"
        );
        return Err(ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND));
    }

    Ok((app, channel))
}

/// POST /v1/apps/{app_id}/slack/events — Slack Events API webhook
///
/// This endpoint handles:
/// 1. URL verification challenges (Slack sends these when setting up the webhook)
/// 2. Event callbacks (messages, mentions, etc.)
///
/// Security: Verified via Slack signing secret (HMAC-SHA256), not API key auth.
pub(crate) async fn handle_slack_event_legacy(
    State(state): State<SlackState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    handle_slack_event(state, SlackTarget::LegacyApp(app_id), req_id, headers, body).await
}

pub(crate) async fn handle_slack_event_channel(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    handle_slack_event(
        state,
        SlackTarget::Channel(channel_id),
        req_id,
        headers,
        body,
    )
    .await
}

pub(crate) async fn handle_slack_event(
    state: SlackState,
    target: SlackTarget,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let (app, slack_channel) = resolve_slack_channel(&state, target).await?;
    let app_id = app.public_id.to_string();
    let internal_error = || {
        ErrorResponse::new("Invalid Slack channel configuration")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    };
    let slack_config: SlackChannelConfig =
        serde_json::from_value(slack_channel.channel_config.clone()).map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Invalid Slack channel config");
            internal_error()
        })?;
    // THREAT[TM-SLACK-001]: verified here too, so a forged request is refused
    // with a 401 rather than acknowledged; the driver verifies again.
    verify_slack_signature(&headers, &body, &slack_config.signing_secret).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Slack signature verification failed");
        ErrorResponse::new("Invalid signature").into_response(StatusCode::UNAUTHORIZED)
    })?;
    let envelope_type = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|body| body["type"].as_str().map(str::to_string))
        .ok_or_else(|| {
            ErrorResponse::new("Invalid request body").into_response(StatusCode::BAD_REQUEST)
        })?;

    let (Some(channels), Some(resolved)) = (
        state.channels.clone(),
        state.channels.as_ref().and_then(|channels| {
            channels.resolver.channel_for(ServerChannelScope {
                request_id,
                ..ServerChannelScope::new(app.clone(), slack_channel.clone())
            })
        }),
    ) else {
        tracing::error!(app_id = %app_id, "Slack channel has no host to run on");
        return Err(internal_error());
    };
    let request = ChannelRequest {
        path: String::new(),
        headers: headers
            .iter()
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect(),
        body: body.to_vec(),
    };

    // The URL check wants its challenge back in this response.
    if envelope_type != "event_callback" {
        let response = channels
            .host
            .try_handle(&resolved, &request)
            .await
            .map_err(|error| {
                tracing::warn!(app_id = %app_id, %error, "Slack request failed");
                <(StatusCode, Json<ErrorResponse>)>::from(
                    crate::channels::host::channel_command_error(error),
                )
            })?;
        let body = match response.body {
            serde_json::Value::Null => ack_json(),
            body if body.get("challenge").is_some() => body,
            _ => ack_json(),
        };
        return Ok((StatusCode::OK, Json(body)));
    }

    // Slack wants an answer within three seconds and redelivers otherwise, so
    // the host runs the event after the acknowledgement. Its failures only log:
    // a non-200 would bring a redelivery storm for nothing a retry fixes.
    tokio::spawn(async move {
        if let Err(error) = channels.host.try_handle(&resolved, &request).await {
            tracing::warn!(app_id = %app_id, %error, "Failed to handle Slack event");
        }
    });
    Ok((StatusCode::OK, Json(ack_json())))
}

pub(crate) fn ack_json() -> serde_json::Value {
    serde_json::to_value(AckResponse { ok: true }).unwrap()
}

pub(crate) fn event_matches_slack_scope(
    slack_config: &SlackChannelConfig,
    envelope_team_id: Option<&str>,
    event: &SlackEvent,
) -> bool {
    if let Some(expected_team_id) = slack_config.team_id.as_deref()
        && envelope_team_id != Some(expected_team_id)
    {
        return false;
    }

    if let Some(expected_channel_id) = slack_config.channel_id.as_deref()
        && event.channel.as_deref() != Some(expected_channel_id)
    {
        return false;
    }

    true
}

pub(crate) fn is_supported_slack_message_subtype(subtype: Option<&str>) -> bool {
    matches!(subtype, None | Some("file_share" | "thread_broadcast"))
}

pub(crate) fn slack_message_metadata(
    app: &crate::api::channel_ingress::IngressContext,
    slack_channel: &crate::api::channel_ingress::IngressChannel,
    event: &SlackEvent,
) -> HashMap<String, serde_json::Value> {
    [
        (
            "_app_id".to_string(),
            serde_json::Value::String(app.public_id.to_string()),
        ),
        (
            "_app_channel_id".to_string(),
            serde_json::Value::String(slack_channel.public_id.to_string()),
        ),
        (
            "slack_channel".to_string(),
            serde_json::Value::String(event.channel.clone().unwrap_or_default()),
        ),
        (
            "slack_thread_ts".to_string(),
            serde_json::Value::String(
                event
                    .thread_ts
                    .as_ref()
                    .or(event.ts.as_ref())
                    .cloned()
                    .unwrap_or_default(),
            ),
        ),
        (
            "slack_ts".to_string(),
            serde_json::Value::String(event.ts.clone().unwrap_or_default()),
        ),
        // Needed by `chat.startStream` after a restart: recovery has no event to
        // read the sender from (EVE-974).
        (
            "slack_user".to_string(),
            serde_json::Value::String(event.user.clone().unwrap_or_default()),
        ),
    ]
    .into_iter()
    .collect()
}
