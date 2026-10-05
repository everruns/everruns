//! `events/list`, `events/subscribe`, `events/unsubscribe` (MCP Events draft,
//! EVE-1121). The service owns validation, verification and delivery; this
//! module maps JSON-RPC to it.

use serde_json::{Value, json};

use super::{AppState, JsonRpcResponse, ResolvedOrg};
use crate::services::mcp_events::{EventsError, McpEventsService, Subscriber, SubscriptionTarget};

/// Whether `events/*` exist for this org: the deployment wired the service and
/// the org opted into the experimental flag.
pub(super) fn enabled(org: &ResolvedOrg, state: &AppState) -> bool {
    state.mcp_events.is_some() && org.feature_flags.mcp_events
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
        code: -32602,
        message: "MCP Events need a user token".to_string(),
        data: None,
    })?;
    crate::domains::sessions::SESSION_VIEW
        .evaluate_with(
            state.auth.permission_resolver.as_ref(),
            &everruns_core::Caller::from(org),
        )
        .map_err(|error| EventsError {
            code: -32602,
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
    let secret = params
        .pointer("/delivery/secret")
        .and_then(Value::as_str)
        .ok_or_else(|| EventsError {
            code: -32602,
            message: "delivery.secret is required".to_string(),
            data: None,
        })?;
    let ttl_ms = params.get("ttlMs").and_then(Value::as_u64);
    let subscribed = service
        .subscribe(&subscriber, &target, secret, ttl_ms)
        .await?;
    // No replay: delivery starts now. A client resuming from a cursor is told
    // it missed events rather than silently getting none.
    let resumed = params.get("cursor").is_some_and(|cursor| !cursor.is_null());
    Ok(json!({
        "id": subscribed.id,
        "refreshBefore": subscribed.refresh_before.to_rfc3339(),
        "cursor": Value::Null,
        "truncated": resumed,
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
