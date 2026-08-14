// Notification inbox API.
// Routes use ResolvedOrg for org scoping and authenticated user delivery.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::notifications::NotificationService;
use crate::domains::notifications::{ListNotifications, MarkNotificationViewed};
use crate::kernel_imports::{Caller, everruns_provider::typed_id::NotificationId};
use crate::notification_notifications::NotificationNotificationBroadcaster;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
};
use axum_extra::extract::Query;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio::time::Instant;
use utoipa::{IntoParams, ToSchema};

use super::common::{ApiResult, ErrorResponse, impl_auth_state};
use super::sse::{DisconnectReason, SseConnectionTracker, SseStreamConfig};
use futures::{
    StreamExt,
    stream::{self, Stream},
};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Notification {
    #[schema(value_type = String, example = "notification_01933b5a00007000800000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: NotificationId,
    /// Discriminator selecting the variant of this resource.
    pub kind: String,
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: String,
    pub body: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub href: Option<String>,
    pub payload: serde_json::Value,
    pub occurrence_count: i32,
    pub viewed_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListNotificationsResponse {
    /// Page of items returned by this query.
    pub data: Vec<Notification>,
    pub unviewed_count: u32,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ListNotificationsQuery {
    /// Maximum number of items returned in this page.
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct NotificationSseQuery {
    pub since: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub notification_service: Arc<NotificationService>,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub notification_broadcaster: Option<Arc<NotificationNotificationBroadcaster>>,
    pub auth: AuthState,
}

impl AppState {
    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);

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
        ErrorResponse::new("Notifications require an authenticated user".to_string())
            .into_response(StatusCode::UNAUTHORIZED)
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
) -> ApiResult<ListNotificationsResponse> {
    require_user_id(&org)?;
    Ok(Json(
        ListNotifications { limit: query.limit }
            .run(&state.ctx(&org))
            .await?,
    ))
}

pub async fn mark_notification_viewed(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> ApiResult<Notification> {
    require_user_id(&org)?;
    Ok(Json(
        MarkNotificationViewed { notification_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

pub async fn stream_notifications_sse(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<NotificationSseQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    if !org.feature_flags.notifications {
        return Err(ErrorResponse::feature_not_enabled("notifications"));
    }
    let user_id = require_user_id(&org)?;

    let sse_guard = state
        .sse_tracker
        .try_acquire(org.org_id, user_id)
        .map_err(|rejection| {
            tracing::warn!(org_id = org.org_id, user_id = %user_id, reason = %rejection, "Notification SSE rejected");
            ErrorResponse::new(rejection.to_string())
                .into_response(StatusCode::TOO_MANY_REQUESTS)
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
