// Inbound MCP Events callback: `POST /v1/e/{ingress_id}/mcp-events`.
//
// An `mcp_event` trigger subscribes to an event on one of its agent's MCP
// servers and hands the server this URL (`domains::agent_triggers::mcp_event`).
// The server first POSTs a signed verification challenge, then signed events.
//
// THREAT[TM-TRIGGER-005]: the route is public and unauthenticated by our auth.
// The Standard Webhooks signature, keyed by the per-trigger secret, is the only
// gate; it is verified before the body is parsed, together with the timestamp
// window. Replays within the window are recorded duplicates (`webhook-id` is
// the pipeline's event id). Bodies over 256 KiB are refused unread by the
// handler. Unknown, disabled or superseded subscriptions answer `410 Gone` so
// the server stops sending.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::json;

use super::common::ErrorResponse;
use super::endpoint_webhooks::EndpointWebhookState;
use crate::domains::agent_triggers::events::TriggerEventOutcome;
use crate::domains::agent_triggers::mcp_event::{
    CALLBACK_SEGMENT, InboundDelivery, InboundOutcome, InboundRejection,
};
use crate::middleware::RequestId;
use crate::services::standard_webhooks::{
    HEADER_ID, HEADER_SIGNATURE, HEADER_SUBSCRIPTION_ID, HEADER_TIMESTAMP,
};

pub fn routes(state: EndpointWebhookState) -> Router {
    Router::new()
        .route(
            &format!("/v1/e/{{ingress_id}}/{CALLBACK_SEGMENT}"),
            post(receive),
        )
        .with_state(state)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn error(status: StatusCode, message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(status)
        .into_response()
}

async fn receive(
    State(state): State<EndpointWebhookState>,
    Path(ingress_id): Path<String>,
    req_id: Option<axum::Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(service) = state.mcp_event_triggers.as_ref() else {
        return error(StatusCode::GONE, "Unknown subscription");
    };
    let delivery = InboundDelivery {
        ingress_id: &ingress_id,
        webhook_id: header(&headers, HEADER_ID),
        timestamp: header(&headers, HEADER_TIMESTAMP),
        signature: header(&headers, HEADER_SIGNATURE),
        subscription_id: header(&headers, HEADER_SUBSCRIPTION_ID),
        body: &body,
    };
    let outcome = service
        .receive(
            &state.session_service,
            &state.message_service,
            delivery,
            req_id.map(|axum::Extension(id)| id.0),
        )
        .await;
    match outcome {
        Ok(InboundOutcome::Challenge(challenge)) => {
            (StatusCode::OK, Json(json!({ "challenge": challenge }))).into_response()
        }
        Ok(InboundOutcome::Event(outcome)) => {
            let (delivery, session_id) = match outcome {
                TriggerEventOutcome::Dispatched(result) => {
                    ("dispatched", Some(result.session_id.to_string()))
                }
                TriggerEventOutcome::Filtered => ("filtered", None),
                TriggerEventOutcome::Duplicate => ("duplicate", None),
            };
            (
                StatusCode::OK,
                Json(json!({ "delivery": delivery, "session_id": session_id })),
            )
                .into_response()
        }
        Err(InboundRejection::Gone) => error(StatusCode::GONE, "Unknown subscription"),
        Err(InboundRejection::TooLarge) => {
            error(StatusCode::PAYLOAD_TOO_LARGE, "Body exceeds 256 KiB")
        }
        Err(InboundRejection::Unauthorized(reason)) => {
            tracing::debug!(%ingress_id, %reason, "rejected MCP event delivery");
            error(StatusCode::UNAUTHORIZED, "Invalid signature")
        }
        Err(InboundRejection::BadRequest(reason)) => error(StatusCode::BAD_REQUEST, reason),
        Err(InboundRejection::Failed(err)) => {
            tracing::warn!(%ingress_id, error = %err, "MCP event trigger dispatch failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}
