// Notification inbox API.
// Routes use ResolvedOrg for org scoping and authenticated user delivery.

use crate::auth::{AuthState, ResolvedOrg};
use crate::notification_notifications::NotificationNotificationBroadcaster;
use crate::services::NotificationService;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
};
use axum_extra::extract::Query;
use chrono::{DateTime, Utc};
use everruns_core::NotificationId;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio::time::Instant;
use utoipa::{IntoParams, ToSchema};

use super::common::ErrorResponse;
use super::sse::{DisconnectReason, SseConnectionTracker, SseStreamConfig};
use futures::{
    StreamExt,
    stream::{self, Stream},
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Notification {
    #[schema(value_type = String, example = "notification_01933b5a00007000800000000000001")]
    pub id: NotificationId,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub occurrence_count: i32,
    pub viewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListNotificationsResponse {
    pub data: Vec<Notification>,
    pub unviewed_count: u32,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListNotificationsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct NotificationSseQuery {
    pub since: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AppState {
    pub notification_service: Arc<NotificationService>,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub notification_broadcaster: Option<Arc<NotificationNotificationBroadcaster>>,
    pub auth: AuthState,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/notifications", get(list_notifications))
        .route("/v1/notifications/sse", get(stream_notifications_sse))
        .route(
            "/v1/notifications/{notification_id}/view",
            post(mark_notification_viewed),
        )
        .with_state(state)
}

fn require_user_id(org: &ResolvedOrg) -> Result<uuid::Uuid, (StatusCode, Json<ErrorResponse>)> {
    org.user_id.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Notifications require an authenticated user".to_string(),
            }),
        )
    })
}

fn row_to_notification(row: crate::storage::NotificationRow) -> Notification {
    Notification {
        id: row.id,
        kind: row.kind,
        title: row.title,
        body: row.body,
        target_type: row.target_type,
        target_id: row.target_id,
        href: row.href,
        payload: row.payload,
        occurrence_count: row.occurrence_count,
        viewed_at: row.viewed_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub async fn list_notifications(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListNotificationsQuery>,
) -> Result<Json<ListNotificationsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let user_id = require_user_id(&org)?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);

    let notifications = state
        .notification_service
        .list(org.org_id, user_id, limit)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list notifications: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;
    let unviewed_count = state
        .notification_service
        .count_unviewed(org.org_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to count notifications: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(ListNotificationsResponse {
        data: notifications.into_iter().map(row_to_notification).collect(),
        unviewed_count,
    }))
}

pub async fn mark_notification_viewed(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> Result<Json<Notification>, (StatusCode, Json<ErrorResponse>)> {
    let user_id = require_user_id(&org)?;
    let notification_id: NotificationId = notification_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid notification ID: {}", e),
            }),
        )
    })?;

    let notification = state
        .notification_service
        .mark_viewed(org.org_id, user_id, notification_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to mark notification viewed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Notification not found".to_string(),
                }),
            )
        })?;

    Ok(Json(row_to_notification(notification)))
}

pub async fn stream_notifications_sse(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<NotificationSseQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    let user_id = require_user_id(&org)?;

    let sse_guard = state
        .sse_tracker
        .try_acquire(org.org_id, user_id)
        .map_err(|rejection| {
            tracing::warn!(org_id = org.org_id, user_id = %user_id, reason = %rejection, "Notification SSE rejected");
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: rejection.to_string(),
                }),
            )
        })?;

    let service = state.notification_service.clone();
    let config = SseStreamConfig::realtime();
    let connection_start = Instant::now();
    let waker = Arc::new(tokio::sync::Notify::new());

    if let Some(ref broadcaster) = state.notification_broadcaster {
        let mut rx = broadcaster.subscribe();
        let target_org_id = org.org_id;
        let target_user_id = user_id;
        let target_waker = waker.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(payload)
                        if payload.org_id == target_org_id && payload.user_id == target_user_id =>
                    {
                        target_waker.notify_one();
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        target_waker.notify_one();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    #[derive(Clone)]
    enum StreamPhase {
        SendConnected,
        Polling,
        SendDisconnecting,
        Closed,
    }

    #[derive(Clone)]
    struct StreamState {
        phase: StreamPhase,
        last_updated_at: Option<DateTime<Utc>>,
        backoff_ms: u64,
        max_duration: Duration,
        config: SseStreamConfig,
        connection_start: Instant,
        event_waker: Arc<tokio::sync::Notify>,
    }

    let initial_state = StreamState {
        phase: StreamPhase::SendConnected,
        last_updated_at: query.since,
        backoff_ms: config.min_backoff_ms,
        max_duration: config.jittered_max_connection_duration(),
        config,
        connection_start,
        event_waker: waker,
    };

    let stream = stream::unfold(initial_state, move |state| {
        let service = service.clone();
        async move {
            match state.phase {
                StreamPhase::Closed => None,
                StreamPhase::SendConnected => {
                    let connected = Ok(SseEvent::default()
                        .event("connected")
                        .data(r#"{"status":"connected"}"#)
                        .retry(state.config.retry_hint(state.backoff_ms)));
                    Some((
                        stream::iter(vec![connected]),
                        StreamState {
                            phase: StreamPhase::Polling,
                            ..state
                        },
                    ))
                }
                StreamPhase::SendDisconnecting => {
                    let disconnecting = Ok(SseEvent::default()
                        .event("disconnecting")
                        .data(format!(
                            r#"{{"reason":"{}","retry_ms":{}}}"#,
                            DisconnectReason::ConnectionCycle.as_str(),
                            state.config.disconnect_retry_ms
                        ))
                        .retry(state.config.disconnect_retry()));
                    Some((
                        stream::iter(vec![disconnecting]),
                        StreamState {
                            phase: StreamPhase::Closed,
                            ..state
                        },
                    ))
                }
                StreamPhase::Polling => {
                    if state.connection_start.elapsed() > state.max_duration {
                        return Some((
                            stream::iter(vec![]),
                            StreamState {
                                phase: StreamPhase::SendDisconnecting,
                                ..state
                            },
                        ));
                    }

                    match service
                        .list_updated_since(org.org_id, user_id, state.last_updated_at, 100)
                        .await
                    {
                        Ok(notifications) if !notifications.is_empty() => {
                            let retry_duration =
                                state.config.retry_hint(state.config.min_backoff_ms);
                            let last_updated_at = notifications.last().map(|row| row.updated_at);
                            let events = notifications
                                .into_iter()
                                .map(|row| {
                                    let json = serde_json::to_string(&row_to_notification(row))
                                        .unwrap_or_else(|_| "{}".to_string());
                                    Ok(SseEvent::default()
                                        .event("notification.upsert")
                                        .data(json)
                                        .retry(retry_duration))
                                })
                                .collect::<Vec<_>>();

                            Some((
                                stream::iter(events),
                                StreamState {
                                    last_updated_at,
                                    backoff_ms: state.config.min_backoff_ms,
                                    ..state
                                },
                            ))
                        }
                        Ok(_) => {
                            let fallback = Duration::from_millis(state.backoff_ms);
                            tokio::select! {
                                _ = state.event_waker.notified() => {}
                                _ = tokio::time::sleep(fallback) => {}
                            }

                            Some((
                                stream::iter(vec![]),
                                StreamState {
                                    backoff_ms: state.config.next_backoff(state.backoff_ms),
                                    ..state
                                },
                            ))
                        }
                        Err(e) => {
                            tracing::error!("Failed to stream notifications: {}", e);
                            None
                        }
                    }
                }
            }
        }
    })
    .flatten();

    let guarded_stream = GuardedStream {
        inner: Box::pin(stream),
        _guard: sse_guard,
    };

    let keep_alive = KeepAlive::new()
        .interval(config.heartbeat_interval())
        .text("heartbeat");
    Ok(Sse::new(guarded_stream).keep_alive(keep_alive))
}

struct GuardedStream<S> {
    inner: std::pin::Pin<Box<S>>,
    _guard: super::sse::SseConnectionGuard,
}

impl<S: Stream> Stream for GuardedStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}
