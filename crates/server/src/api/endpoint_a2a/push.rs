// A2A push notifications (A2A 1.0 §3.1.7-3.1.10, §4.3.3; 0.3
// `tasks/pushNotificationConfig/*`).
//
// THREAT[TM-A2A-012]: a config is read and written only through a task the
// calling channel owns (`bound_task_session`), so one channel's key never
// learns of, or redirects, another channel's task updates.
// THREAT[TM-A2A-015]: the webhook URL is client-chosen. HTTPS only, checked
// against private and metadata ranges when it is stored, and delivered with
// DNS pinning so a name cannot rebind to an internal address between the two
// (TM-API-020). The egress client follows no redirects.
//
// Design Decisions:
// - One notification per settle: the turn completed, failed or was canceled,
//   or it parked on an `ask_user` question (`input-required`). The payload is
//   the full task, the `StreamResponse.task` case of §4.3.3 (a bare `Task` in
//   0.3), so one POST carries the state and the answer artifacts and the
//   receiver needs no follow-up `GetTask`.
// - The client's `token` and `authentication.credentials` are secrets the
//   receiver checks, so they are stored encrypted and never echoed back by
//   Get or List. The token travels as `X-A2A-Notification-Token`, the
//   credentials as `Authorization: {scheme} {credentials}`.
// - Delivery runs on the replica that persisted the event, off the event path,
//   with two retries. There is no durable outbox, so a replica that dies mid
//   retry drops that notification; a client can always fall back to
//   `GetTask`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{TOOL_CALL_REQUESTED, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED};
use everruns_core::{EgressRequest, EgressRequestKind, EgressService, Event, EventListener};
use serde_json::{Value, json};

use super::wire::{self, WireVersion};
use super::{
    AuthorizedA2a, EndpointA2aState, JsonRpcRequest, RpcRejection, bound_task_session,
    internal_error, rpc_success, task_view,
};
use crate::storage::encryption::EncryptionService;
use crate::storage::{A2aPushConfigRow, StorageBackend, UpsertA2aPushConfig};

/// Configs one task may hold. Each settle fans out to all of them.
const MAX_CONFIGS_PER_TASK: usize = 10;
const MAX_URL_LEN: usize = 2048;
const MAX_FIELD_LEN: usize = 4096;
const REQUEST_TIMEOUT_MS: u64 = 10_000;
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(10)];
pub(super) const TOKEN_HEADER: &str = "x-a2a-notification-token";

/// A push config as a client sent it, either version's shape.
#[derive(Debug, Default, PartialEq)]
pub(super) struct PushConfigInput {
    pub id: Option<String>,
    pub url: String,
    pub token: Option<String>,
    pub scheme: Option<String>,
    pub credentials: Option<String>,
}

/// The four config methods, normalized from either version's name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PushMethod {
    Create,
    Get,
    List,
    Delete,
}

pub(super) fn push_method(method: &str) -> Option<PushMethod> {
    Some(match method {
        "CreateTaskPushNotificationConfig" | "tasks/pushNotificationConfig/set" => {
            PushMethod::Create
        }
        "GetTaskPushNotificationConfig" | "tasks/pushNotificationConfig/get" => PushMethod::Get,
        "ListTaskPushNotificationConfigs" | "tasks/pushNotificationConfig/list" => PushMethod::List,
        "DeleteTaskPushNotificationConfig" | "tasks/pushNotificationConfig/delete" => {
            PushMethod::Delete
        }
        _ => return None,
    })
}

fn text(value: Option<&Value>, name: &str) -> Option<String> {
    value?
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Parse a config object. 1.0 carries `authentication.scheme`; 0.3 carries
/// `authentication.schemes`, of which the first is used.
pub(super) fn parse_config(config: &Value) -> Result<PushConfigInput, &'static str> {
    let url = text(Some(config), "url").ok_or("Invalid params: missing required `url`")?;
    if url.len() > MAX_URL_LEN {
        return Err("Invalid params: `url` is too long");
    }
    // Static SSRF check at write time; delivery pins DNS (TM-A2A-015).
    if !url.starts_with("https://")
        || everruns_contracts::url_validation::validate_safe_url(&url).is_err()
    {
        return Err("Invalid params: `url` must be a public https URL");
    }
    let auth = config.get("authentication");
    let scheme = text(auth, "scheme").or_else(|| {
        auth?
            .get("schemes")?
            .as_array()?
            .iter()
            .find_map(|s| s.as_str().filter(|s| !s.is_empty()).map(str::to_owned))
    });
    let input = PushConfigInput {
        id: text(Some(config), "id"),
        url,
        token: text(Some(config), "token"),
        credentials: text(auth, "credentials"),
        scheme,
    };
    let too_long = [&input.id, &input.token, &input.scheme, &input.credentials]
        .into_iter()
        .flatten()
        .any(|field| field.len() > MAX_FIELD_LEN);
    if too_long {
        return Err("Invalid params: a push config field is too long");
    }
    if input.scheme.as_deref().is_some_and(|s| {
        !s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    }) {
        return Err("Invalid params: `authentication.scheme` is not a valid HTTP auth scheme");
    }
    if input
        .credentials
        .iter()
        .chain(&input.token)
        .any(|value| value.chars().any(char::is_control))
    {
        return Err("Invalid params: push credentials must not contain control characters");
    }
    Ok(input)
}

/// The push config a `SendMessage` carries in its `configuration`:
/// `taskPushNotificationConfig` in 1.0, `pushNotificationConfig` in 0.3.
pub(super) fn send_config(
    configuration: Option<&Value>,
) -> Result<Option<PushConfigInput>, &'static str> {
    let Some(configuration) = configuration else {
        return Ok(None);
    };
    match configuration
        .get("taskPushNotificationConfig")
        .or_else(|| configuration.get("pushNotificationConfig"))
    {
        Some(Value::Null) | None => Ok(None),
        Some(config) => parse_config(config).map(Some),
    }
}

/// `(task id, config object)` from a create request: 1.0 sends the config
/// itself with `taskId`, 0.3 nests it under `pushNotificationConfig`.
fn create_params(params: &Value) -> Result<(String, &Value), &'static str> {
    let task_id =
        text(Some(params), "taskId").ok_or("Invalid params: missing required `taskId`")?;
    let config = params.get("pushNotificationConfig").unwrap_or(params);
    Ok((task_id, config))
}

/// `(task id, config id)` from get/list/delete: 1.0 names them `taskId` and
/// `id`, 0.3 `id` and `pushNotificationConfigId`.
fn lookup_params(params: &Value, version: WireVersion) -> (Option<String>, Option<String>) {
    match version {
        WireVersion::V1_0 => (text(Some(params), "taskId"), text(Some(params), "id")),
        WireVersion::V0_3 => (
            text(Some(params), "id"),
            text(Some(params), "pushNotificationConfigId"),
        ),
    }
}

/// Encrypt the config's secrets for storage. `None` when it has none.
fn seal_secrets(
    encryption: Option<&Arc<EncryptionService>>,
    input: &PushConfigInput,
) -> anyhow::Result<Option<Vec<u8>>> {
    if input.token.is_none() && input.credentials.is_none() {
        return Ok(None);
    }
    let encryption = encryption
        .ok_or_else(|| anyhow::anyhow!("A2A push secrets need secrets encryption configured"))?;
    let secrets = json!({ "token": input.token, "credentials": input.credentials });
    Ok(Some(encryption.encrypt_string(&secrets.to_string())?))
}

/// Store a config on a task the caller already owns. Shared by
/// `CreateTaskPushNotificationConfig` and `SendMessage` configuration.
pub(super) async fn store_config(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org_id: i64,
    session_id: SessionId,
    version: WireVersion,
    input: &PushConfigInput,
) -> anyhow::Result<A2aPushConfigRow> {
    db.upsert_a2a_push_config(UpsertA2aPushConfig {
        org_id,
        session_id,
        config_id: input
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
        url: input.url.clone(),
        auth_scheme: input.scheme.clone(),
        secrets_encrypted: seal_secrets(encryption, input)?,
        wire_version: version_label(version).to_string(),
    })
    .await
}

fn version_label(version: WireVersion) -> &'static str {
    match version {
        WireVersion::V1_0 => "1.0",
        WireVersion::V0_3 => "0.3",
    }
}

/// A stored config for the wire, without its secrets.
fn render_config(row: &A2aPushConfigRow, version: WireVersion) -> Value {
    let task_id = row.session_id.to_string();
    match version {
        WireVersion::V1_0 => {
            let mut config = json!({ "id": row.config_id, "taskId": task_id, "url": row.url });
            if let Some(scheme) = &row.auth_scheme {
                config["authentication"] = json!({ "scheme": scheme });
            }
            config
        }
        WireVersion::V0_3 => {
            let mut inner = json!({ "id": row.config_id, "url": row.url });
            if let Some(scheme) = &row.auth_scheme {
                inner["authentication"] = json!({ "schemes": [scheme] });
            }
            json!({ "taskId": task_id, "pushNotificationConfig": inner })
        }
    }
}

const CONFIG_NOT_FOUND: RpcRejection = RpcRejection(-32001, "Push notification config not found");

/// Dispatch one of the four config methods.
pub(super) async fn handle(
    state: &EndpointA2aState,
    auth: AuthorizedA2a,
    method: PushMethod,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    match run(state, &auth, method, &parsed.params, version).await {
        Ok(result) => (StatusCode::OK, rpc_success(rpc_id, result)).into_response(),
        Err(Failure::Rejected(rejection)) => rejection.into_response_with(rpc_id),
        Err(Failure::Internal(err)) => internal_error(err).into_response(),
    }
}

enum Failure {
    Rejected(RpcRejection),
    Internal(anyhow::Error),
}

impl From<RpcRejection> for Failure {
    fn from(rejection: RpcRejection) -> Self {
        Self::Rejected(rejection)
    }
}

impl From<anyhow::Error> for Failure {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

async fn run(
    state: &EndpointA2aState,
    auth: &AuthorizedA2a,
    method: PushMethod,
    params: &Value,
    version: WireVersion,
) -> Result<Value, Failure> {
    let invalid = |msg| Failure::Rejected(RpcRejection(-32602, msg));
    let (task_id, config_id) = match method {
        PushMethod::Create => (create_params(params).map_err(invalid)?.0, None),
        _ => {
            let (task_id, config_id) = lookup_params(params, version);
            (
                task_id.ok_or_else(|| invalid("Invalid params: missing task id"))?,
                config_id,
            )
        }
    };
    let session = bound_task_session(state, auth, &json!({ "id": task_id })).await?;
    let configs = state.db.list_a2a_push_configs(session.id).await?;
    match method {
        PushMethod::Create => {
            let (_, raw) = create_params(params).map_err(invalid)?;
            let input = parse_config(raw).map_err(invalid)?;
            let replaces = input
                .id
                .as_deref()
                .is_some_and(|id| configs.iter().any(|c| c.config_id == id));
            if !replaces && configs.len() >= MAX_CONFIGS_PER_TASK {
                return Err(invalid(
                    "Invalid params: a task holds at most 10 push configs",
                ));
            }
            let row = store_config(
                &state.db,
                state.encryption.as_ref(),
                auth.org_id,
                session.id,
                version,
                &input,
            )
            .await?;
            Ok(render_config(&row, version))
        }
        PushMethod::Get => {
            // 0.3 lets `get` omit the config id: the task's first config.
            let row = match &config_id {
                Some(id) => configs.iter().find(|c| &c.config_id == id),
                None if version == WireVersion::V0_3 => configs.first(),
                None => return Err(invalid("Invalid params: missing required `id`")),
            };
            Ok(render_config(row.ok_or(CONFIG_NOT_FOUND)?, version))
        }
        PushMethod::List => {
            let rendered: Vec<Value> = configs.iter().map(|c| render_config(c, version)).collect();
            Ok(match version {
                WireVersion::V1_0 => json!({ "configs": rendered, "nextPageToken": "" }),
                WireVersion::V0_3 => Value::Array(rendered),
            })
        }
        PushMethod::Delete => {
            let config_id =
                config_id.ok_or_else(|| invalid("Invalid params: missing config id"))?;
            if !state
                .db
                .delete_a2a_push_config(session.id, &config_id)
                .await?
            {
                return Err(CONFIG_NOT_FOUND.into());
            }
            Ok(match version {
                WireVersion::V1_0 => json!({}),
                WireVersion::V0_3 => Value::Null,
            })
        }
    }
}

// ============================================================================
// Delivery
// ============================================================================

/// Delivers task updates to the configs registered on a task.
#[derive(Clone)]
pub struct A2aPushListener {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    egress: Arc<dyn EgressService>,
    frontend_url: String,
}

impl A2aPushListener {
    /// The listener as the server wires it.
    pub fn shared(
        db: &Arc<StorageBackend>,
        encryption: &Option<Arc<EncryptionService>>,
        host: &everruns_host::HostComposition,
        auth: &crate::auth::AuthState,
    ) -> Arc<dyn EventListener> {
        Arc::new(Self {
            db: db.clone(),
            encryption: encryption.clone(),
            egress: host.egress_service(),
            frontend_url: auth.config.frontend_url.clone(),
        })
    }

    async fn dispatch(&self, session_id: SessionId) -> anyhow::Result<()> {
        let configs = self.db.list_a2a_push_configs(session_id).await?;
        if configs.is_empty() {
            return Ok(());
        }
        let Some(session) = self.db.get_session_unscoped(session_id).await? else {
            return Ok(());
        };
        let task = task_view::load_task_for(&self.db, &self.frontend_url, session.org_id, &session)
            .await?;
        for config in configs.iter().filter(|c| c.org_id == session.org_id) {
            match self.request(config, &task) {
                Ok(request) => self.deliver(config, request).await,
                Err(err) => {
                    tracing::warn!(error = %err, config_id = %config.config_id, "A2A push skipped");
                }
            }
        }
        Ok(())
    }

    fn request(&self, config: &A2aPushConfigRow, task: &Value) -> anyhow::Result<EgressRequest> {
        let secrets = match &config.secrets_encrypted {
            Some(sealed) => {
                let encryption = self
                    .encryption
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("secrets encryption is not configured"))?;
                serde_json::from_str(&encryption.decrypt_to_string(sealed)?)?
            }
            None => Value::Null,
        };
        let version = if config.wire_version == "0.3" {
            WireVersion::V0_3
        } else {
            WireVersion::V1_0
        };
        build_request(
            &config.url,
            version,
            task.clone(),
            secrets["token"].as_str(),
            config.auth_scheme.as_deref(),
            secrets["credentials"].as_str(),
        )
    }

    async fn deliver(&self, config: &A2aPushConfigRow, request: EgressRequest) {
        let mut delays = RETRY_DELAYS.iter();
        loop {
            match self.egress.send(request.clone()).await {
                Ok(response) if (200..300).contains(&response.status) => return,
                // A client error will not change on retry.
                Ok(response) if (400..500).contains(&response.status) && response.status != 429 => {
                    tracing::warn!(status = response.status, config_id = %config.config_id, "A2A push rejected");
                    return;
                }
                Ok(_) | Err(_) => {}
            }
            let Some(delay) = delays.next() else {
                tracing::warn!(config_id = %config.config_id, "A2A push delivery gave up");
                return;
            };
            tokio::time::sleep(*delay).await;
        }
    }
}

/// The webhook POST for one task update (§4.3.3).
pub(super) fn build_request(
    url: &str,
    version: WireVersion,
    task: Value,
    token: Option<&str>,
    scheme: Option<&str>,
    credentials: Option<&str>,
) -> anyhow::Result<EgressRequest> {
    let body = serde_json::to_vec(&wire::stream_frame(version, task))?;
    let content_type = match version {
        WireVersion::V1_0 => "application/a2a+json",
        WireVersion::V0_3 => "application/json",
    };
    let mut request = EgressRequest::new("POST", url, EgressRequestKind::Integration)
        .header("content-type", content_type)
        .body(body)
        .timeout_ms(REQUEST_TIMEOUT_MS)
        .require_dns_pinning();
    if let Some(token) = token {
        request = request.header(TOKEN_HEADER, token);
    }
    if let (Some(scheme), Some(credentials)) = (scheme, credentials) {
        request = request.header("authorization", format!("{scheme} {credentials}"));
    }
    Ok(request)
}

#[async_trait]
impl EventListener for A2aPushListener {
    async fn on_event(&self, event: &Event) {
        if !task_view::settles_task(&event.data) {
            return;
        }
        let listener = self.clone();
        let session_id = event.session_id;
        tokio::spawn(async move {
            if let Err(err) = listener.dispatch(session_id).await {
                tracing::warn!(error = %err, session_id = %session_id, "A2A push dispatch failed");
            }
        });
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            TURN_COMPLETED,
            TURN_FAILED,
            TURN_CANCELLED,
            TOOL_CALL_REQUESTED,
        ])
    }

    fn name(&self) -> &'static str {
        "A2aPushListener"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn https(url: &str) -> Value {
        json!({ "url": url })
    }

    #[test]
    fn parse_config_reads_either_versions_authentication() {
        let v1 = parse_config(&json!({
            "id": "c1",
            "url": "https://hooks.example.com/a2a",
            "token": "tok",
            "authentication": { "scheme": "Bearer", "credentials": "secret" },
        }))
        .unwrap();
        assert_eq!(
            v1,
            PushConfigInput {
                id: Some("c1".into()),
                url: "https://hooks.example.com/a2a".into(),
                token: Some("tok".into()),
                scheme: Some("Bearer".into()),
                credentials: Some("secret".into()),
            }
        );
        let v03 = parse_config(&json!({
            "url": "https://hooks.example.com/a2a",
            "authentication": { "schemes": ["Basic"], "credentials": "dXNlcjpwdw==" },
        }))
        .unwrap();
        assert_eq!(v03.scheme.as_deref(), Some("Basic"));
    }

    #[test]
    fn parse_config_rejects_unsafe_urls_and_header_injection() {
        for url in [
            "http://hooks.example.com/a2a",
            "https://localhost/a2a",
            "https://169.254.169.254/latest",
            "https://10.0.0.5/hook",
            "ftp://hooks.example.com",
        ] {
            assert!(parse_config(&https(url)).is_err(), "{url} accepted");
        }
        assert!(parse_config(&json!({})).is_err());
        let injected = json!({
            "url": "https://hooks.example.com",
            "authentication": { "scheme": "Bearer\r\nX-Evil: 1", "credentials": "x" },
        });
        assert!(parse_config(&injected).is_err());
        let injected_token = json!({ "url": "https://hooks.example.com", "token": "a\nb" });
        assert!(parse_config(&injected_token).is_err());
    }

    #[test]
    fn push_method_accepts_both_spellings() {
        assert_eq!(
            push_method("CreateTaskPushNotificationConfig"),
            Some(PushMethod::Create)
        );
        assert_eq!(
            push_method("tasks/pushNotificationConfig/set"),
            Some(PushMethod::Create)
        );
        assert_eq!(
            push_method("tasks/pushNotificationConfig/list"),
            Some(PushMethod::List)
        );
        assert_eq!(push_method("tasks/get"), None);
    }

    #[test]
    fn build_request_carries_the_task_and_the_receivers_secrets() {
        let task = json!({
            "id": "t1", "contextId": "t1", "kind": "task",
            "status": { "state": "completed" },
        });
        let request = build_request(
            "https://hooks.example.com/a2a",
            WireVersion::V1_0,
            task.clone(),
            Some("tok"),
            Some("Bearer"),
            Some("secret"),
        )
        .unwrap();
        let header = |name: &str| request.headers.get(name).map(String::as_str);
        assert_eq!(header("content-type"), Some("application/a2a+json"));
        assert_eq!(header(TOKEN_HEADER), Some("tok"));
        assert_eq!(header("authorization"), Some("Bearer secret"));
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["task"]["status"]["state"], "TASK_STATE_COMPLETED");

        let legacy = build_request(
            "https://h.example.com",
            WireVersion::V0_3,
            task,
            None,
            None,
            None,
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&legacy.body).unwrap();
        assert_eq!(body["kind"], "task");
        assert!(!legacy.headers.contains_key("authorization"));
    }
}
