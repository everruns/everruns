//! Inbound event routing: parse, scope, dispatch, and message processing.

use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use chrono::Utc;
use everruns_core::Caller;
use everruns_core::channel::{
    InboundAttachment, InboundChannelEvent, SessionBinding, ThreadContext,
};
use everruns_core::progress_reporting::sync_slack_reply_mode_tags;
use everruns_platform::{App, AppChannel, ChannelType, SlackChannelConfig, SlackReplyMode};
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};
use std::collections::HashMap;

use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::domains::messages::CreateMessageContext;
use crate::domains::sessions::SessionService;
use crate::execution_metadata;
use crate::middleware::RequestId;
use crate::services::PrincipalService;
use crate::slack_delivery::{SlackSurface, classify_surface};
use crate::storage::models::{CreateSessionParticipantRow, SessionParticipantRow, UpdateSession};

use super::super::common::ErrorResponse;

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
    }
}

pub(crate) async fn resolve_slack_channel(
    state: &SlackState,
    target: SlackTarget,
) -> Result<(App, AppChannel), (StatusCode, Json<ErrorResponse>)> {
    let (app, endpoint_channel) = match target {
        SlackTarget::LegacyApp(app_id) => {
            let app = crate::domains::apps::queries::get_by_public_id_unscoped(
                &state.db,
                state.encryption.as_ref(),
                &app_id,
            )
            .await
            .map_err(|error| {
                tracing::error!(app_id, %error, "Failed to lookup app for Slack ingress");
                ErrorResponse::new("Internal server error")
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?
            .ok_or_else(|| {
                ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND)
            })?;
            (app, None)
        }
        SlackTarget::Endpoint(channel_id) => {
            let (app, channel) = crate::api::app_ingress::resolve_endpoint(
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
            .ok_or_else(|| {
                ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND)
            })?;
            (app, Some(channel))
        }
    };

    let channel = match endpoint_channel {
        Some(channel) if channel.channel_type == ChannelType::Slack => channel,
        Some(_) => {
            return Err(ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND));
        }
        None => match crate::api::app_ingress::resolve_legacy_channel(&app, ChannelType::Slack) {
            crate::api::app_ingress::LegacyChannelMatch::One(channel) => channel,
            crate::api::app_ingress::LegacyChannelMatch::NotFound => {
                return Err(
                    ErrorResponse::new("App not found").into_response(StatusCode::NOT_FOUND)
                );
            }
            crate::api::app_ingress::LegacyChannelMatch::Ambiguous => {
                return Err(ErrorResponse::new(
                    "Multiple enabled Slack channels; use an endpoint-scoped /v1/e/{channel_id}/slack/... URL",
                )
                .into_response(StatusCode::CONFLICT));
            }
        },
    };

    if let Err(reason) = crate::api::app_ingress::endpoint_liveness(&state.db, &app, &channel)
        .await
        .map_err(|_| {
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
    {
        tracing::debug!(
            app_id = %app.public_id,
            endpoint_id = %channel.public_id,
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

pub(crate) async fn handle_slack_event_endpoint(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<ErrorResponse>)> {
    handle_slack_event(
        state,
        SlackTarget::Endpoint(channel_id),
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

    // 3. Parse Slack channel config
    let slack_config: SlackChannelConfig =
        serde_json::from_value(slack_channel.channel_config.clone()).map_err(|e| {
            tracing::error!(app_id = %app_id, error = %e, "Invalid Slack channel config");
            ErrorResponse::new("Invalid Slack channel configuration")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    let slack_channel_internal_id = slack_channel.internal_id;

    // 4. Verify Slack signing secret // THREAT[TM-SLACK-001]
    verify_slack_signature(&headers, &body, &slack_config.signing_secret).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Slack signature verification failed");
        ErrorResponse::new("Invalid signature").into_response(StatusCode::UNAUTHORIZED)
    })?;

    // 5. Parse the event envelope
    let envelope: SlackEventEnvelope = serde_json::from_slice(&body).map_err(|e| {
        tracing::warn!(app_id = %app_id, error = %e, "Failed to parse Slack event");
        ErrorResponse::new("Invalid request body").into_response(StatusCode::BAD_REQUEST)
    })?;

    // 6. Handle based on event type
    match envelope.event_type.as_str() {
        "url_verification" => {
            let challenge = envelope.challenge.unwrap_or_default();
            tracing::info!(app_id = %app_id, "Slack URL verification challenge received");

            // Record webhook verification timestamp (idempotent — only sets if not already set)
            if slack_config.webhook_verified_at.is_none() {
                let mut updated_config = slack_config.clone();
                updated_config.webhook_verified_at = Some(Utc::now());
                if let Ok(config_json) = serde_json::to_value(&updated_config)
                    && let Err(e) = crate::domains::apps::queries::update_channel_config_unscoped(
                        &state.db,
                        state.encryption.as_ref(),
                        slack_channel_internal_id,
                        &config_json,
                    )
                    .await
                {
                    tracing::warn!(app_id = %app_id, error = %e, "Failed to record webhook verification");
                }
            }

            Ok((
                StatusCode::OK,
                Json(serde_json::to_value(ChallengeResponse { challenge }).unwrap()),
            ))
        }
        "event_callback" => {
            let envelope_team_id = envelope.team_id.clone();
            if let Some(event) = envelope.event {
                if !event_matches_slack_scope(&slack_config, envelope_team_id.as_deref(), &event) {
                    tracing::warn!(
                        app_id = %app_id,
                        expected_team_id = ?slack_config.team_id,
                        incoming_team_id = ?envelope_team_id,
                        expected_channel_id = ?slack_config.channel_id,
                        incoming_channel_id = ?event.channel,
                        "Ignoring Slack event outside configured scope"
                    );
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // Agent-surface lifecycle events. Acknowledged and logged so the
                // toggle is safe to enable before the behaviour that consumes them
                // lands (EVE-974 streaming, EVE-975 status, EVE-976 stop,
                // EVE-977 context). Handled explicitly rather than falling into the
                // generic "not a message" branch so an unknown event stays
                // distinguishable from one we deliberately ignore.
                // Where the user is looking updates the session's persisted
                // ThreadContext, read at prompt-assembly time. Deliberately not an
                // `input.message`: the pane reports a new context on every
                // navigation, and one event per change would flood both the event
                // log and the model's history for a field only the latest value of
                // which matters (EVE-977).
                if event.event_type == "app_context_changed" {
                    if let Err(error) = handle_app_context_changed(
                        &state,
                        &app,
                        &slack_channel,
                        &slack_config,
                        &event,
                    )
                    .await
                    {
                        // Non-fatal: Slack retries a non-200, and a lost context
                        // hint is not worth a redelivery storm.
                        tracing::warn!(
                            app_id = %app_id,
                            %error,
                            "Failed to record Slack context change"
                        );
                    }
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // The stop button. This is the first inbound *control* signal
                // from Slack rather than a message, so it is deliberately
                // narrow: it can cancel the resolved session's turn and nothing
                // else — no resume, retry, or mutation — and the session it
                // resolves must belong to the app that received the event
                // (EVE-976).
                if event.event_type == "agent_session_stopped" {
                    if let Err(error) = handle_agent_session_stopped(
                        &state,
                        &app,
                        &slack_channel,
                        &slack_config,
                        &event,
                    )
                    .await
                    {
                        // Non-fatal: Slack retries a non-200, and a failed stop
                        // is not worth a redelivery storm. The user can press
                        // stop again.
                        tracing::warn!(
                            app_id = %app_id,
                            %error,
                            "Failed to handle Slack stop request"
                        );
                    }
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // The user renamed the thread in the pane. We push titles the
                // other way on `session.title.updated`, so ignoring this would
                // silently revert their rename the next time the agent retitled
                // the session — two sources of truth for one name. Write it back
                // instead, through the same no-op-suppressing helper the agent
                // path uses, so a rename to the current title emits nothing and
                // the two directions cannot echo each other.
                if event.event_type == "agent_session_title_changed" {
                    if let Err(error) = handle_agent_session_title_changed(
                        &state,
                        &app,
                        &slack_channel,
                        &slack_config,
                        &event,
                    )
                    .await
                    {
                        // Non-fatal, like the stop button: a lost rename is not
                        // worth a Slack redelivery storm.
                        tracing::warn!(
                            app_id = %app_id,
                            %error,
                            "Failed to apply Slack thread rename"
                        );
                    }
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                if event.event_type == "app_home_opened" {
                    tracing::debug!(
                        app_id = %app_id,
                        event_type = %event.event_type,
                        "Slack agent-surface event acknowledged (no-op)"
                    );
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // Skip bot messages to avoid loops // THREAT[TM-SLACK-002]
                if event.bot_id.is_some() {
                    tracing::debug!(app_id = %app_id, "Skipping bot message");
                    return Ok((StatusCode::OK, Json(ack_json())));
                }
                if !is_supported_slack_message_subtype(event.subtype.as_deref()) {
                    tracing::debug!(
                        app_id = %app_id,
                        subtype = ?event.subtype,
                        "Ignoring unsupported Slack message subtype"
                    );
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                // Only handle "message" and "app_mention" events
                if event.event_type != "message" && event.event_type != "app_mention" {
                    tracing::debug!(app_id = %app_id, event_type = %event.event_type, "Ignoring non-message event");
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                let text = event.text.clone().unwrap_or_default();
                if text.is_empty() && event.files.is_empty() && event.attachments.is_empty() {
                    return Ok((StatusCode::OK, Json(ack_json())));
                }

                tracing::info!(
                    app_id = %app_id,
                    event_type = %event.event_type,
                    channel = ?event.channel,
                    user = ?event.user,
                    thread_ts = ?event.thread_ts,
                    "Slack message received"
                );

                // Record first message timestamp (idempotent — only sets once)
                if slack_config.first_message_received_at.is_none() {
                    let mut updated_config = slack_config.clone();
                    updated_config.first_message_received_at = Some(Utc::now());
                    if let Ok(config_json) = serde_json::to_value(&updated_config)
                        && let Err(e) =
                            crate::domains::apps::queries::update_channel_config_unscoped(
                                &state.db,
                                state.encryption.as_ref(),
                                slack_channel_internal_id,
                                &config_json,
                            )
                            .await
                    {
                        tracing::warn!(app_id = %app_id, error = %e, "Failed to record first message timestamp");
                    }
                }

                // Process message in background (Slack requires 200 within 3 seconds)
                let state = state.clone();
                let app = app.clone();
                let slack_channel = slack_channel.clone();
                let slack_config = slack_config.clone();
                let spawned_request_id = request_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_slack_message(
                        &state,
                        &app,
                        &slack_channel,
                        &slack_config,
                        &event,
                        spawned_request_id,
                    )
                    .await
                    {
                        tracing::error!(
                            app_id = %app_id,
                            error = %e,
                            "Failed to process Slack message"
                        );
                    }
                });
            }

            Ok((StatusCode::OK, Json(ack_json())))
        }
        other => {
            tracing::debug!(app_id = %app_id, event_type = %other, "Unhandled Slack event type");
            Ok((StatusCode::OK, Json(ack_json())))
        }
    }
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

/// Process an incoming Slack message: find/create session, create message, wait for response.
///
/// Uses the channel abstraction types:
/// - `InboundChannelEvent` for platform-agnostic message parsing
/// - `build_session_routing_tag()` for session lookup (via `build_session_tags()`)
/// - `ThreadContext` for participant tracking
pub(crate) async fn process_slack_message(
    state: &SlackState,
    app: &App,
    slack_channel: &AppChannel,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
    request_id: Option<String>,
) -> anyhow::Result<()> {
    let org_id = app.org_id;
    let slack_user_id = event.user.clone().unwrap_or_default();

    let surface = classify_surface(
        slack_config.agent_surface_enabled,
        event.channel_type.as_deref(),
        event.channel.as_deref().unwrap_or_default(),
    );

    // Resolve Slack user display name (gracefully falls back to user ID)
    let display_name = if !slack_user_id.is_empty() {
        resolve_slack_user_name(
            &state.user_name_cache,
            &slack_config.bot_token,
            &slack_user_id,
        )
        .await
    } else {
        None
    };

    // Parse Slack event into platform-agnostic InboundChannelEvent
    let inbound = parse_slack_inbound_event(event, slack_config, display_name);
    let text = inbound.text.clone();

    // ExternalActor for the message (None if no user ID)
    let external_actor = if !slack_user_id.is_empty() {
        Some(inbound.actor.clone())
    } else {
        None
    };

    // Resolve org_public_id
    let org_row = state
        .db
        .get_organization(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Organization not found for app"))?;
    let org_public_id = org_row.public_id;

    // Build session tags based on strategy
    let routing_tags = build_session_tags(app, slack_channel, slack_config, event, surface);
    let desired_tags = desired_session_tags(&routing_tags, slack_config.reply_mode);

    // Find or create session
    let (session, is_new_session) = match state
        .db
        .find_app_session_by_tags(org_id, app.internal_id, &routing_tags)
        .await?
    {
        Some(row) => {
            tracing::debug!(
                session_id = %row.id,
                tags = ?routing_tags,
                "Found existing Slack session"
            );
            let fallback = if row.harness_id.is_none() {
                Some(crate::org_init::base_harness_id(&state.db, org_id).await?)
            } else {
                None
            };
            let session = SessionService::row_to_session(row, &org_public_id, fallback);
            let mut synced_tags = session.tags.clone();
            sync_slack_reply_mode_tags(&mut synced_tags, slack_config.reply_mode.into());
            if synced_tags != session.tags
                && let Err(error) = state
                    .db
                    .update_session(
                        org_id,
                        session.id,
                        UpdateSession {
                            tags: Some(synced_tags),
                            ..Default::default()
                        },
                    )
                    .await
            {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "Failed to sync Slack reply-mode session tags"
                );
            }
            (session, false)
        }
        None => {
            tracing::info!(
                tags = ?desired_tags,
                "Creating new Slack session"
            );
            let title = build_session_title(slack_config, event);
            let req = CreateSessionRequest {
                source: None,
                workspace_id: None,
                harness_id: Some(app.harness_id),
                harness_name: None,
                agent_id: app.agent_id,
                agent_name: None,
                title: Some(title),
                goal: None,
                locale: None,
                tags: desired_tags.clone(),
                agent_identity_id: app.agent_identity_id,
                model_id: None,
                capabilities: vec![],
                tools: vec![],
                mcp_servers: Default::default(),
                system_prompt: None,
                initial_files: vec![],
                // EVE-1025: this thread can draw an approval card, so the
                // session says so. A surface that does not declare the hint
                // gets today's behaviour — no card, the ask stays prose — which
                // is the degradation Client Hints asks for rather than a gap.
                hints: Some(std::collections::HashMap::from([(
                    crate::slack_approvals::SLACK_APPROVAL_HINT.to_string(),
                    serde_json::Value::Bool(true),
                )])),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                parent_session_id: None,
                forked_from_session_id: None,
                budget_root_session_id: None,
                seed: everruns_core::SessionSeedMode::Fresh,
            };
            let internal_caller = Caller::internal(org_id);
            let s = state
                .session_service
                .create_from_app(
                    &internal_caller,
                    app.harness_id.uuid(),
                    app.agent_id.map(|agent_id| agent_id.uuid()),
                    app.agent_id,
                    app.internal_id,
                    Some(slack_channel.internal_id),
                    app.owner_principal_id,
                    app.resolved_owner_user_id,
                    everruns_platform::SessionSource::Slack,
                    req,
                )
                .await?;
            (s, true)
        }
    };

    // Accumulate the thread's participants durably. This used to build a
    // ThreadContext per message, track into it, log, and drop it — so
    // `participants_summary()` never saw more than one person and nothing
    // survived a restart (EVE-977).
    if let Some(ref thread_ref) = inbound.thread_ref {
        let mut thread_ctx = load_thread_context(state, session.id)
            .await
            .unwrap_or_else(|| ThreadContext::new(thread_ref.clone(), "slack"));
        if let Some(ref channel) = event.channel {
            thread_ctx
                .platform_metadata
                .insert("channel_id".to_string(), channel.clone());
        }
        if thread_ctx.track_participant(&inbound.actor) {
            tracing::debug!(
                session_id = %session.id,
                actor_id = %inbound.actor.actor_id,
                participants = thread_ctx.participant_count(),
                "Tracked new participant in thread context"
            );
        }
        // Non-fatal: a lost participant line must not cost the user their reply.
        if let Err(error) = save_thread_context(state, session.id, &thread_ctx).await {
            tracing::warn!(
                session_id = %session.id,
                %error,
                "Failed to persist thread context (participants will not accumulate)"
            );
        }
    }

    let speaker_participant = match external_actor.as_ref() {
        Some(actor) => Some(ensure_slack_user_participant(state, org_id, session.id, actor).await?),
        None => None,
    };

    // Inject thread context: when joining an existing thread mid-conversation,
    // fetch prior messages from Slack and inject them as context so the agent
    // sees the full conversation history.
    //
    // Deliberately per_thread only. A per_channel or per_user session is not
    // scoped to one thread: it outlives any single thread and accumulates its
    // own history across turns, so backfilling would re-inject a full thread
    // every time the session touched a new one, duplicating context it already
    // holds. per_thread is the only strategy where "new session" and "thread
    // the agent has not seen" are the same statement (EVE-969).
    if is_new_session
        && slack_config.session_strategy == SessionBinding::Thread
        && event.thread_ts.is_some()
    {
        let thread_ts = event.thread_ts.as_deref().unwrap();
        let channel = event.channel.as_deref().unwrap_or("unknown");
        if let Err(e) = inject_thread_context(
            state,
            &slack_config.bot_token,
            channel,
            thread_ts,
            session.id,
            event.ts.as_deref(),
        )
        .await
        {
            // Non-fatal: agent proceeds without historical context
            tracing::warn!(
                session_id = %session.id,
                thread_ts = thread_ts,
                error = %e,
                "Failed to inject thread context (agent will proceed without history)"
            );
        }
    }

    // Dedup: Slack sends both app_mention and message events for @mentions.
    // Check if we already have an input.message with this dedup_key (slack_ts) in the session.
    // This works across server instances because the check is DB-level.
    if !inbound.dedup_key.is_empty() {
        let already_exists = state
            .db
            .has_event_with_slack_ts(session.id, &inbound.dedup_key)
            .await?;
        if already_exists {
            tracing::debug!(
                session_id = %session.id,
                dedup_key = %inbound.dedup_key,
                "Skipping duplicate Slack event (already processed this ts)"
            );
            return Ok(());
        }
    }

    // Build content parts: text + file attachments + legacy attachments
    let mut content: Vec<InputContentPart> = Vec::new();
    if !text.is_empty() {
        content.push(InputContentPart::text(text));
    }
    content.extend(build_file_content_parts(&event.files));
    content.extend(build_attachment_content_parts(&event.attachments));

    // Create user message (triggers agent workflow)
    // Slack-specific metadata (ts for threading) stays in metadata;
    // user identity is carried by external_actor.
    let create_msg = CreateMessageRequest {
        message: InputMessage {
            role: MessageRole::User,
            content,
        },
        addressed_participant_id: None,
        controls: None,
        metadata: Some(slack_message_metadata(
            app,
            slack_channel,
            event,
            speaker_participant.as_ref(),
        )),
        tags: None,
        external_actor,
    };
    let mut event_metadata = execution_metadata::app_message_metadata(
        app.public_id,
        app.owner_principal_id,
        app.agent_identity_id,
    );
    if let Some(participant) = &speaker_participant
        && let Some(map) = event_metadata.as_object_mut()
    {
        map.insert(
            "participant_id".to_string(),
            serde_json::Value::String(participant.id.to_string()),
        );
    }

    let message = state
        .message_service
        .create(
            CreateMessageContext {
                org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: app.agent_id.map(|agent_id| agent_id.uuid()),
                session_id: session.id.uuid(),
                event_metadata: Some(event_metadata),
                request_id,
            },
            create_msg,
        )
        .await?;

    tracing::info!(
        session_id = %session.id,
        message_id = %message.id,
        "Slack message routed to session"
    );

    // Register delivery for agent response posting
    let bot_token = slack_config.bot_token.clone();
    let channel = event.channel.clone().unwrap_or_default();
    // Reply in thread: use thread_ts if available, otherwise the message ts
    let thread_ts = event
        .thread_ts
        .clone()
        .or_else(|| event.ts.clone())
        .unwrap_or_default();
    let session_id = session.id.uuid();
    let message_id = message.id;

    // Through the adapter rather than the Slack client directly (EVE-972), so the
    // trait's ack path is exercised by its only implementation instead of being
    // dead code a second platform would have to discover the gaps in.
    if slack_config.reply_mode == SlackReplyMode::ReportProgressOnly
        && !channel.is_empty()
        && !thread_ts.is_empty()
    {
        use everruns_core::channel::{
            ChannelDeliveryAdapter, DeliveryContext as ChannelDeliveryContext,
            DeliveryResult as ChannelDeliveryResult,
        };

        let adapter = crate::slack_delivery::SlackDeliveryAdapter::new();
        let delivery_ctx = ChannelDeliveryContext {
            auth_token: bot_token.clone(),
            channel_id: channel.clone(),
            thread_ref: thread_ts.clone(),
            reply_mode: slack_config.reply_mode.into(),
            extra: std::collections::HashMap::new(),
        };

        if let ChannelDeliveryResult::TransientError(error)
        | ChannelDeliveryResult::PermanentError(error) =
            adapter.send_ack(&thread_ts, "On it.", &delivery_ctx).await
        {
            tracing::warn!(
                session_id = %session.id,
                error = %error,
                "Failed to post initial Slack handoff acknowledgement"
            );
        }
    }

    if let Some(ref dispatcher) = state.delivery_dispatcher {
        // Event-driven delivery: no deadline, handles arbitrarily long turns
        dispatcher
            .register(crate::slack_delivery::DeliveryRegistration {
                session_id,
                input_message_id: message_id.to_string(),
                bot_token,
                channel,
                thread_ts,
                reply_mode: slack_config.reply_mode,
                surface,
                recipient_user_id: (!slack_user_id.is_empty()).then(|| slack_user_id.clone()),
                recipient_team_id: slack_config.team_id.clone(),
                tool_visibility: slack_config.tool_visibility,
                generic_tool_text: slack_config.generic_tool_text.clone(),
                approvals_enabled: crate::slack_approvals::approvals_enabled_in(
                    session.hints.as_ref(),
                ),
            })
            .await;
    } else {
        // Fallback for DEV_MODE without EventNotificationBroadcaster:
        // use the legacy polling approach
        let db = state.db.clone();
        let reply_mode = slack_config.reply_mode;
        tokio::spawn(async move {
            if let Err(e) = wait_and_post_response(
                &db, session_id, message_id, &bot_token, &channel, &thread_ts, reply_mode,
            )
            .await
            {
                tracing::error!(
                    session_id = %session_id,
                    error = %e,
                    "Failed to post Slack response"
                );
            }
        });
    }

    Ok(())
}

pub(crate) async fn ensure_slack_user_participant(
    state: &SlackState,
    org_id: i64,
    session_id: everruns_provider::typed_id::SessionId,
    actor: &everruns_core::ExternalActor,
) -> anyhow::Result<SessionParticipantRow> {
    let principal = PrincipalService::new(state.db.clone())
        .ensure_external_actor_principal(org_id, actor)
        .await?;
    state
        .db
        .ensure_active_user_session_participant(CreateSessionParticipantRow {
            org_id,
            session_id,
            kind: SessionParticipantKind::User,
            agent_id: None,
            agent_version_id: None,
            principal_id: principal.id,
            display_name: Some(actor.display_label().to_string()),
            role: SessionParticipantRole::Member,
            joined_at: None,
        })
        .await
}

pub(crate) fn slack_message_metadata(
    app: &App,
    slack_channel: &AppChannel,
    event: &SlackEvent,
    participant: Option<&SessionParticipantRow>,
) -> HashMap<String, serde_json::Value> {
    let mut metadata: HashMap<String, serde_json::Value> = [
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
    .collect();
    if let Some(participant) = participant {
        metadata.insert(
            "participant_id".to_string(),
            serde_json::Value::String(participant.id.to_string()),
        );
    }
    metadata
}

/// Build session tags for finding/creating sessions based on the binding.
///
/// Uses the generic `build_session_routing_tag()` from channel abstractions.
/// Since EVE-1005 there is no Slack-specific strategy type to convert from —
/// the channel config stores `SessionBinding` directly.
/// Cancel the turn running in the pane thread the stop button was pressed in.
///
/// Authorization comes from the lookup, not a separate check:
/// `find_app_session_by_tags` is scoped to this app's org and internal id, so a
/// session belonging to any other app simply does not resolve and the stop is
/// logged and dropped. That keeps the webhook's blast radius exactly as wide as
/// it already was for inbound messages.
///
/// A stop for a turn that already finished is a no-op rather than an error:
/// `cancel_session_turn_for` reads the session's terminal state first and skips
/// the `turn.cancelled` emission, so a completed turn is never race-flipped to
/// cancelled and the thread gets no spurious notice.
///
/// The in-thread confirmation is not posted here. `turn.cancelled` is a
/// terminal state the delivery dispatcher already renders as one line with a
/// session link (EVE-966); posting our own would double it.
pub(crate) async fn handle_agent_session_stopped(
    state: &SlackState,
    app: &App,
    slack_channel: &AppChannel,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
) -> anyhow::Result<()> {
    let Some(thread) = event.assistant_thread.as_ref() else {
        tracing::debug!(
            app_id = %app.public_id,
            "Slack stop event carried no assistant_thread; ignoring"
        );
        return Ok(());
    };

    // The pane reports its thread under `assistant_thread`, not at the event
    // root, so normalize before tag-building — same as the context path.
    let mut routing_event = event.clone();
    routing_event.channel = thread.channel_id.clone().or(routing_event.channel);
    routing_event.thread_ts = thread.thread_ts.clone().or(routing_event.thread_ts);

    let routing_tags = build_session_tags(
        app,
        slack_channel,
        slack_config,
        &routing_event,
        SlackSurface::Pane,
    );
    let Some(row) = state
        .db
        .find_app_session_by_tags(app.org_id, app.internal_id, &routing_tags)
        .await?
    else {
        tracing::info!(
            app_id = %app.public_id,
            tags = ?routing_tags,
            "Slack stop request resolved no session for this app; ignoring"
        );
        return Ok(());
    };

    crate::api::app_api::cancel_session_turn_for(
        &state.db,
        &state.message_service,
        row.id,
        "slack stop button",
    )
    .await?;

    tracing::info!(session_id = %row.id, "Cancelled Slack session turn on stop request");
    Ok(())
}

/// Apply a thread rename made in the Slack pane to the session it belongs to.
///
/// Deliberately as narrow as the stop button (EVE-976): it resolves a session
/// the *receiving app* owns, and can change that session's title and nothing
/// else. A rename for a pane with no session yet is a no-op — the first message
/// creates the session, and Slack's own title is what the user already sees.
///
/// Write-back rather than ignore, because the pane is not the only writer: the
/// agent retitles the session and we push that title to Slack, so an ignored
/// rename would be silently reverted the next time it did. `session_title_updated_event`
/// suppresses a no-op change, so the two directions settle instead of echoing.
pub(crate) async fn handle_agent_session_title_changed(
    state: &SlackState,
    app: &App,
    slack_channel: &AppChannel,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
) -> anyhow::Result<()> {
    let Some(thread) = event.assistant_thread.as_ref() else {
        tracing::debug!(
            app_id = %app.public_id,
            "Slack rename carried no assistant_thread; ignoring"
        );
        return Ok(());
    };

    let title = thread
        .title
        .as_deref()
        .or(event.title.as_deref())
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let Some(title) = title else {
        tracing::debug!(
            app_id = %app.public_id,
            "Slack rename carried no title; ignoring"
        );
        return Ok(());
    };

    // The pane reports its thread under `assistant_thread`, not at the event
    // root, so normalize before tag-building — same as the context path.
    let mut routing_event = event.clone();
    routing_event.channel = thread.channel_id.clone().or(routing_event.channel);
    routing_event.thread_ts = thread.thread_ts.clone().or(routing_event.thread_ts);

    let routing_tags = build_session_tags(
        app,
        slack_channel,
        slack_config,
        &routing_event,
        SlackSurface::Pane,
    );
    let Some(row) = state
        .db
        .find_app_session_by_tags(app.org_id, app.internal_id, &routing_tags)
        .await?
    else {
        tracing::debug!(
            app_id = %app.public_id,
            tags = ?routing_tags,
            "Slack rename for a pane with no session yet; ignoring"
        );
        return Ok(());
    };

    let Some(event_request) =
        everruns_host::session_services::capabilities::session::session_title_updated_event(
            row.id,
            everruns_core::events::EventContext::empty(),
            row.title.clone(),
            title.to_string(),
        )
    else {
        // Already the stored title — the rename came from our own push.
        return Ok(());
    };

    state
        .db
        .update_session(
            app.org_id,
            row.id,
            crate::storage::models::UpdateSession {
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .await?;
    state.event_service.emit(event_request).await?;

    tracing::info!(session_id = %row.id, "Applied Slack thread rename to session title");
    Ok(())
}

/// Record the user's current position on the session's persisted ThreadContext.
///
/// A context change for a pane that has no session yet is a no-op: there is
/// nothing to attach it to, and the first message will create the session with
/// no position recorded — which is correct, since by then the user may have
/// moved on. Slack re-reports on the next navigation either way.
pub(crate) async fn handle_app_context_changed(
    state: &SlackState,
    app: &App,
    slack_channel: &AppChannel,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
) -> anyhow::Result<()> {
    let Some(thread) = event.assistant_thread.as_ref() else {
        return Ok(());
    };

    // The pane reports its thread under `assistant_thread`, not at the event
    // root, so normalize before tag-building rather than teaching the tag
    // builder a second shape.
    let mut routing_event = event.clone();
    routing_event.channel = thread.channel_id.clone().or(routing_event.channel);
    routing_event.thread_ts = thread.thread_ts.clone().or(routing_event.thread_ts);

    let routing_tags = build_session_tags(
        app,
        slack_channel,
        slack_config,
        &routing_event,
        SlackSurface::Pane,
    );
    let Some(row) = state
        .db
        .find_app_session_by_tags(app.org_id, app.internal_id, &routing_tags)
        .await?
    else {
        tracing::debug!(
            app_id = %app.public_id,
            tags = ?routing_tags,
            "Slack context change for a pane with no session yet; ignoring"
        );
        return Ok(());
    };

    let view = thread
        .context
        .as_ref()
        .map(|ctx| everruns_core::ChannelViewContext {
            channel_id: ctx.channel_id.clone(),
            team_id: ctx.team_id.clone(),
            observed_at: Some(Utc::now()),
        })
        .unwrap_or_default();

    let thread_ref = routing_event
        .thread_ts
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let mut thread_ctx = load_thread_context(state, row.id)
        .await
        .unwrap_or_else(|| ThreadContext::new(thread_ref, "slack"));

    // Skip the write when the platform re-reports the same place — the pane
    // does that on every focus change, not only on a real move.
    if !thread_ctx.set_current_view(view) {
        return Ok(());
    }

    save_thread_context(state, row.id, &thread_ctx).await?;
    tracing::debug!(session_id = %row.id, "Recorded Slack context change");
    Ok(())
}
