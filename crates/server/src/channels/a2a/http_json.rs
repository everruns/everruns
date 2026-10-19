// The A2A 1.0 HTTP+JSON binding (spec §11) on the channel's A2A URL.
//
// Design Decision: this binding is a translator in front of the audited
// JSON-RPC dispatcher, not a second set of handlers. Each route becomes the
// 1.0 operation it names (spec §11.3) with the same params a JSON-RPC caller
// would send, and runs through the same auth, method gate (TM-A2A-005),
// channel binding (TM-A2A-012) and handlers. On the way out the JSON-RPC
// envelope is unwrapped: `result` becomes the body, and `error` becomes the
// `google.rpc.Status` envelope with an `ErrorInfo` reason (spec §11.6, §5.4).
// Streams are rendered directly in this binding's frame shape by `stream.rs`.
//
// Both bindings share one interface URL: JSON-RPC is a POST to the URL itself,
// HTTP+JSON operations are paths below it (`{url}/message:send`). The binding
// is A2A 1.0 only, so a request without `A2A-Version` is read as 1.0, and any
// version other than 1.x is `VersionNotSupportedError`.
// See `knowledge/integrations/a2a-channel.md`.

use std::collections::HashMap;

use axum::{
    Extension, Router,
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Map, Value, json};

use super::wire::{self, Binding, WireVersion};
use super::{
    A2aHttpRequest, ChannelA2aState, JsonRpcRequest, MessageSendContext, authenticate_request,
    channel_app_id, dispatch,
};

/// Interface URLs that carry the binding: the canonical channel route and the
/// serve-compatible alias. The legacy App route stays JSON-RPC only.
const BASES: [&str; 2] = ["/v1/channels/{channel_id}/a2a", "/v1/e/{channel_id}/a2a"];

/// Media type of every HTTP+JSON body this binding returns (spec §11.1).
pub(super) const A2A_JSON: &str = "application/a2a+json";

/// Upper bound on a JSON-RPC result this binding unwraps. Results are task
/// snapshots and config lists, far below this.
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn routes(router: Router<ChannelA2aState>) -> Router<ChannelA2aState> {
    BASES.iter().fold(router, |router, base| {
        router
            .route(&format!("{base}/message:send"), post(send_message))
            .route(&format!("{base}/message:stream"), post(stream_message))
            .route(&format!("{base}/tasks"), get(list_tasks))
            .route(
                &format!("{base}/tasks/{{task}}"),
                get(get_task).post(task_action),
            )
            .route(
                &format!("{base}/tasks/{{task}}/pushNotificationConfigs"),
                get(list_push_configs).post(create_push_config),
            )
            .route(
                &format!("{base}/tasks/{{task}}/pushNotificationConfigs/{{config}}"),
                get(get_push_config).delete(delete_push_config),
            )
            .route(
                &format!("{base}/extendedAgentCard"),
                get(extended_agent_card),
            )
    })
}

/// One HTTP+JSON operation, as the 1.0 JSON-RPC method and params it maps to.
struct Operation {
    method: &'static str,
    /// `Err` when the request body is not a JSON object. Reported only after
    /// authentication, so an anonymous caller learns nothing from it.
    params: Result<Value, &'static str>,
}

impl Operation {
    fn new(method: &'static str, params: Value) -> Self {
        Self {
            method,
            params: Ok(params),
        }
    }

    /// An operation whose params are the request body, plus `extra` fields
    /// taken from the path.
    fn with_body(method: &'static str, body: &Bytes, extra: &[(&str, &str)]) -> Self {
        let params = match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(mut fields)) => {
                for (name, value) in extra {
                    fields.insert((*name).to_string(), Value::String((*value).to_string()));
                }
                Ok(Value::Object(fields))
            }
            _ => Err("Invalid params: the request body must be a JSON object"),
        };
        Self { method, params }
    }
}

/// The request parts every route hands to [`run`].
struct Incoming {
    channel_id: String,
    request: A2aHttpRequest,
}

macro_rules! incoming {
    ($channel_id:expr, $uri:expr, $req_id:expr, $connect_info:expr, $headers:expr, $body:expr) => {
        Incoming {
            channel_id: $channel_id,
            request: A2aHttpRequest {
                uri: $uri,
                req_id: $req_id,
                connect_info: $connect_info,
                headers: $headers,
                body: $body,
            },
        }
    };
}

type ReqId = Option<Extension<crate::middleware::RequestId>>;
type Peer = Option<Extension<ConnectInfo<std::net::SocketAddr>>>;

async fn send_message(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::with_body("SendMessage", &body, &[]);
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn stream_message(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::with_body("SendStreamingMessage", &body, &[]);
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

// Axum extractors, one per argument.
#[allow(clippy::too_many_arguments)]
async fn list_tasks(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::new("ListTasks", query_params(query));
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

// Axum extractors, one per argument.
#[allow(clippy::too_many_arguments)]
async fn get_task(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut params = query_params(query);
    params["id"] = Value::String(task);
    let op = Operation::new("GetTask", params);
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

/// `POST /tasks/{id}:cancel` and `POST /tasks/{id}:subscribe`. The action is
/// part of the last path segment, so any other suffix is an unmatched route.
async fn task_action(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let method = match task.rsplit_once(':') {
        Some((_, "cancel")) => "CancelTask",
        Some((_, "subscribe")) => "SubscribeToTask",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let id = task.rsplit_once(':').map_or("", |(id, _)| id);
    let op = Operation::new(method, json!({ "id": id }));
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn create_push_config(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::with_body(
        "CreateTaskPushNotificationConfig",
        &body,
        &[("taskId", task.as_str())],
    );
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn list_push_configs(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::new("ListTaskPushNotificationConfigs", json!({ "taskId": task }));
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn get_push_config(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task, config)): Path<(String, String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let params = json!({ "taskId": task, "id": config });
    let op = Operation::new("GetTaskPushNotificationConfig", params);
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn delete_push_config(
    State(state): State<ChannelA2aState>,
    Path((channel_id, task, config)): Path<(String, String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let params = json!({ "taskId": task, "id": config });
    let op = Operation::new("DeleteTaskPushNotificationConfig", params);
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

async fn extended_agent_card(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    req_id: ReqId,
    connect_info: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let op = Operation::new("GetExtendedAgentCard", json!({}));
    run(
        state,
        incoming!(channel_id, uri, req_id, connect_info, headers, body),
        op,
    )
    .await
}

/// Query parameters as request params (spec §11.5): numbers and booleans are
/// sent as strings, so the fields the handlers read as typed values are
/// converted back. A value that does not convert stays a string and fails the
/// handler's own validation.
fn query_params(query: HashMap<String, String>) -> Value {
    let fields = query
        .into_iter()
        .map(|(name, raw)| {
            let value = match name.as_str() {
                "pageSize" | "historyLength" => raw
                    .parse::<u64>()
                    .map_or_else(|_| Value::String(raw.clone()), Value::from),
                "includeArtifacts" => match raw.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    _ => Value::String(raw.clone()),
                },
                _ => Value::String(raw.clone()),
            };
            (name, value)
        })
        .collect::<Map<_, _>>();
    Value::Object(fields)
}

async fn run(state: ChannelA2aState, incoming: Incoming, op: Operation) -> Response {
    let Incoming {
        channel_id,
        request,
    } = incoming;
    let app_id = match channel_app_id(&state, &channel_id).await {
        Ok(app_id) => app_id,
        Err(err) => return err.into_response(),
    };
    let peer_addr = request
        .connect_info
        .map(|Extension(ConnectInfo(addr))| addr);
    let auth = match authenticate_request(
        &state,
        &app_id,
        &channel_id,
        &request.headers,
        peer_addr,
        &request.body,
    )
    .await
    {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };

    // The PascalCase method name makes an absent version read as 1.0.
    match wire::negotiate(&request.headers, &request.uri, op.method) {
        Ok(WireVersion::V1_0) => {}
        Ok(WireVersion::V0_3) => {
            return a2a_error(
                i64::from(wire::VERSION_NOT_SUPPORTED),
                &wire::version_not_supported_message("0.3"),
            );
        }
        Err(requested) => {
            return a2a_error(
                i64::from(wire::VERSION_NOT_SUPPORTED),
                &wire::version_not_supported_message(&requested),
            );
        }
    }
    let params = match op.params {
        Ok(params) => params,
        Err(message) => return a2a_error(-32602, message),
    };

    let ctx = MessageSendContext {
        app_id,
        channel_id,
        req_id: request.req_id,
        version: WireVersion::V1_0,
        binding: Binding::HttpJson,
    };
    let parsed = JsonRpcRequest {
        jsonrpc: None,
        id: None,
        method: op.method.to_string(),
        params,
    };
    let response = dispatch(&state, auth, parsed, Value::Null, ctx).await;
    unwrap_json_rpc(response).await
}

/// Turn a dispatcher response into this binding's response. Only a JSON-RPC
/// envelope (a `200` JSON body) is rewritten; streams are already in this
/// binding's shape, and plain HTTP errors (auth, rate limit, not found) are
/// the same in both bindings.
async fn unwrap_json_rpc(response: Response) -> Response {
    let is_envelope = response.status() == StatusCode::OK
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"));
    if !is_envelope {
        return response;
    }
    let envelope = match axum::body::to_bytes(response.into_body(), MAX_RESULT_BYTES)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    {
        Some(envelope) => envelope,
        None => return a2a_error(-32603, "Internal error"),
    };
    if let Some(error) = envelope.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Internal error");
        return a2a_error(code, message);
    }
    let result = envelope.get("result").cloned().unwrap_or_else(|| json!({}));
    a2a_json(StatusCode::OK, &result)
}

pub(super) fn a2a_json(status: StatusCode, body: &Value) -> Response {
    (status, [(header::CONTENT_TYPE, A2A_JSON)], body.to_string()).into_response()
}

/// A JSON-RPC error code as its HTTP status, `google.rpc` status name and A2A
/// `ErrorInfo` reason (spec §5.4). The JSON-RPC protocol errors map to
/// `INVALID_PARAMS` and `INTERNAL`, the reasons A2A clients already handle.
fn error_mapping(code: i64) -> (StatusCode, &'static str, &'static str) {
    const BAD: StatusCode = StatusCode::BAD_REQUEST;
    match code {
        -32001 => (StatusCode::NOT_FOUND, "NOT_FOUND", "TASK_NOT_FOUND"),
        -32002 => (BAD, "FAILED_PRECONDITION", "TASK_NOT_CANCELABLE"),
        -32003 => (
            BAD,
            "FAILED_PRECONDITION",
            "PUSH_NOTIFICATION_NOT_SUPPORTED",
        ),
        -32004 => (BAD, "FAILED_PRECONDITION", "UNSUPPORTED_OPERATION"),
        -32005 => (BAD, "INVALID_ARGUMENT", "CONTENT_TYPE_NOT_SUPPORTED"),
        -32006 => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL",
            "INVALID_AGENT_RESPONSE",
        ),
        -32007 => (
            BAD,
            "FAILED_PRECONDITION",
            "EXTENDED_AGENT_CARD_NOT_CONFIGURED",
        ),
        -32008 => (BAD, "FAILED_PRECONDITION", "EXTENSION_SUPPORT_REQUIRED"),
        -32009 => (BAD, "FAILED_PRECONDITION", "VERSION_NOT_SUPPORTED"),
        -32600 | -32602 | -32700 => (BAD, "INVALID_ARGUMENT", "INVALID_PARAMS"),
        -32601 => (BAD, "FAILED_PRECONDITION", "UNSUPPORTED_OPERATION"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", "INTERNAL"),
    }
}

/// The spec §11.6 error body for a JSON-RPC error code.
pub(super) fn a2a_error(code: i64, message: &str) -> Response {
    let (status, status_name, reason) = error_mapping(code);
    a2a_json(
        status,
        &json!({
            "error": {
                "code": status.as_u16(),
                "status": status_name,
                "message": message,
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": reason,
                    "domain": "a2a-protocol.org",
                }],
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn json_rpc_result_becomes_the_body() {
        let envelope = axum::Json(json!({
            "jsonrpc": "2.0", "id": null, "result": { "task": { "id": "t-1" } }
        }))
        .into_response();
        let response = unwrap_json_rpc(envelope).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], A2A_JSON);
        assert_eq!(
            body_json(response).await,
            json!({ "task": { "id": "t-1" } })
        );
    }

    #[tokio::test]
    async fn json_rpc_error_becomes_a_google_rpc_status() {
        let envelope = axum::Json(json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32001, "message": "Task not found" }
        }))
        .into_response();
        let response = unwrap_json_rpc(envelope).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CONTENT_TYPE], A2A_JSON);
        assert_eq!(
            body_json(response).await,
            json!({ "error": {
                "code": 404,
                "status": "NOT_FOUND",
                "message": "Task not found",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "TASK_NOT_FOUND",
                    "domain": "a2a-protocol.org",
                }],
            }})
        );
    }

    #[tokio::test]
    async fn plain_http_errors_pass_through() {
        let response = (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "no" })),
        )
            .into_response();
        let response = unwrap_json_rpc(response).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_json(response).await, json!({ "error": "no" }));
    }

    #[test]
    fn every_a2a_error_has_its_spec_mapping() {
        let cases = [
            (-32001, 404, "TASK_NOT_FOUND"),
            (-32002, 400, "TASK_NOT_CANCELABLE"),
            (-32003, 400, "PUSH_NOTIFICATION_NOT_SUPPORTED"),
            (-32004, 400, "UNSUPPORTED_OPERATION"),
            (-32005, 400, "CONTENT_TYPE_NOT_SUPPORTED"),
            (-32006, 500, "INVALID_AGENT_RESPONSE"),
            (-32007, 400, "EXTENDED_AGENT_CARD_NOT_CONFIGURED"),
            (-32008, 400, "EXTENSION_SUPPORT_REQUIRED"),
            (-32009, 400, "VERSION_NOT_SUPPORTED"),
            (-32602, 400, "INVALID_PARAMS"),
            (-32603, 500, "INTERNAL"),
        ];
        for (code, status, reason) in cases {
            let (actual_status, _, actual_reason) = error_mapping(code);
            assert_eq!(
                (actual_status.as_u16(), actual_reason),
                (status, reason),
                "{code}"
            );
        }
    }

    #[test]
    fn query_params_restore_typed_fields() {
        let query = HashMap::from([
            ("pageSize".to_string(), "20".to_string()),
            ("includeArtifacts".to_string(), "true".to_string()),
            ("contextId".to_string(), "ctx".to_string()),
            ("historyLength".to_string(), "many".to_string()),
        ]);
        assert_eq!(
            query_params(query),
            json!({
                "pageSize": 20,
                "includeArtifacts": true,
                "contextId": "ctx",
                "historyLength": "many",
            })
        );
    }

    #[test]
    fn body_params_take_path_fields_and_reject_non_objects() {
        let op = Operation::with_body(
            "CreateTaskPushNotificationConfig",
            &Bytes::from_static(br#"{"url":"https://hooks.example.com"}"#),
            &[("taskId", "t-1")],
        );
        assert_eq!(
            op.params.unwrap(),
            json!({ "url": "https://hooks.example.com", "taskId": "t-1" })
        );
        let op = Operation::with_body("SendMessage", &Bytes::from_static(b"[]"), &[]);
        assert!(op.params.is_err());
    }
}
