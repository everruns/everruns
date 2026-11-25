//! `events/list`, `events/subscribe`, `events/unsubscribe` (SEP-3415,
//! EVE-1121). The service owns validation, verification and delivery; this
//! module maps JSON-RPC to it.

use serde_json::{Value, json};

use super::{AppState, JsonRpcResponse, ResolvedOrg};
use crate::domains::mcp_servers::events::{
    EventsError, McpEventsService, Subscriber, SubscriptionTarget,
};

/// Whether the host wired the built-in events service.
pub(super) fn enabled(_org: &ResolvedOrg, state: &AppState) -> bool {
    state.mcp_events.is_some()
}

pub(super) async fn handle_method(
    method: &str,
    id: Option<Value>,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let Some(service) = state.mcp_events.as_ref().filter(|_| enabled(org, state)) else {
        return JsonRpcResponse::method_not_found(id);
    };
    let result = match method {
        // One page: the catalog is three events.
        "events/list" => Ok(json!({ "events": McpEventsService::event_definitions() })),
        "events/poll" | "events/stream" => match SubscriptionTarget::event_name(&params) {
            Err(error) => Err(error),
            Ok(_) => {
                let mode = if method == "events/poll" {
                    "poll"
                } else {
                    "push"
                };
                Err(EventsError {
                    code: -32026,
                    message: "Only webhook delivery is supported".into(),
                    data: Some(json!({"feature": "deliveryMode", "value": mode})),
                })
            }
        },
        "events/subscribe" => subscribe(service, &params, org, state).await,
        "events/unsubscribe" => unsubscribe(service, &params, org, state).await,
        _ => return JsonRpcResponse::method_not_found(id),
    };
    match result {
        Ok(result) => JsonRpcResponse::success(id, result),
        Err(error) => {
            let mut response = JsonRpcResponse::error(id, error.code as i32, error.message);
            if let (Some(data), Some(body)) = (error.data, response.error.as_mut()) {
                body.data = Some(data);
            }
            response
        }
    }
}

/// THREAT[TM-MCP-010]: a subscription is bound to the token's user, and only a
/// user who may view sessions may subscribe. Delivery re-checks both.
fn subscriber(org: &ResolvedOrg, state: &AppState) -> Result<Subscriber, EventsError> {
    let user_id = org.user_id.ok_or_else(|| EventsError {
        code: -32024,
        message: "MCP Events need a user token".to_string(),
        data: None,
    })?;
    crate::domains::sessions::SESSION_VIEW
        .evaluate_with(
            state.auth.permission_resolver.as_ref(),
            &everruns_core::Caller::from(org),
        )
        .map_err(|error| EventsError {
            code: -32024,
            message: error.to_string(),
            data: None,
        })?;
    Ok(Subscriber {
        org_id: org.org_id,
        user_id,
    })
}

async fn subscribe(
    service: &McpEventsService,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, EventsError> {
    let subscriber = subscriber(org, state)?;
    let target = SubscriptionTarget::from_params(params)?;
    if params.pointer("/delivery/mode").and_then(Value::as_str) != Some("webhook") {
        return Err(EventsError {
            code: -32602,
            message: "delivery.mode is required".into(),
            data: None,
        });
    }
    if params
        .get("cursor")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(EventsError {
            code: -32602,
            message: "cursor must be a string or null".into(),
            data: None,
        });
    }
    let secret = params
        .pointer("/delivery/secret")
        .and_then(Value::as_str)
        .ok_or_else(|| EventsError {
            code: -32602,
            message: "delivery.secret is required".to_string(),
            data: None,
        })?;
    let ttl_ms = match params.get("ttlMs") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .or_else(|| value.as_i64().map(|v| v.max(0) as u64))
                .ok_or_else(|| EventsError {
                    code: -32602,
                    message: "ttlMs must be an integer or null".into(),
                    data: None,
                })?,
        ),
    };
    let subscribed = service
        .subscribe(&subscriber, &target, secret, ttl_ms)
        .await?;
    // Cursorless event types have no replay position to truncate (SEP-3415).
    Ok(json!({
        "id": subscribed.id,
        "refreshBefore": subscribed.refresh_before.to_rfc3339(),
        "cursor": Value::Null,
        "truncated": false,
    }))
}

async fn unsubscribe(
    service: &McpEventsService,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, EventsError> {
    let subscriber = subscriber(org, state)?;
    let target = SubscriptionTarget::from_params(params)?;
    service.unsubscribe(&subscriber, &target).await?;
    Ok(json!({}))
}
