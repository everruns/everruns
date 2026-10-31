//! Consumer-only Work Chat extensions. Every resource is reached through an
//! authenticated channel and session; this router never accepts management ids.
use super::*;
pub(crate) mod experience;
use crate::domains::agent_channels::record::work_chat::{
    ChatFeatures, ChatInterface, WorkChatConfig, host_label_ok,
};
use crate::domains::session_files::service::{CreateFileInput, WorkspaceFileService};
use axum::extract::Query;
use sha2::{Digest, Sha256};

fn file_revision(file: &everruns_core::SessionFile) -> String {
    hex::encode(Sha256::digest(format!(
        "{}\0{}",
        file.encoding,
        file.content.as_deref().unwrap_or("")
    )))
}

pub(super) fn routes() -> Router<ChannelApiState> {
    let mut router = Router::new().route("/v1/work-chat/hosts/{label}", get(get_work_chat_by_host));
    for prefix in ["/v1/channels", "/v1/e"] {
        router = router
            .route(
                &format!("{prefix}/{{channel_id}}/chat/config"),
                get(get_work_chat_config),
            )
            .route(
                &format!("{prefix}/{{channel_id}}/chat/main"),
                post(resolve_work_chat_main),
            )
            .route(
                &format!("{prefix}/{{channel_id}}/sessions/{{session_id}}/work"),
                get(list_work_chat_tasks),
            )
            .route(
                &format!("{prefix}/{{channel_id}}/sessions/{{session_id}}/files"),
                get(get_work_chat_files)
                    .post(write_work_chat_file)
                    .delete(delete_work_chat_file),
            );
    }
    router.merge(experience::routes())
}

/// This channel's agent avatar, as a public circle preset, or nothing.
async fn agent_logo(
    state: &ChannelApiState,
    context: &crate::domains::agent_channels::ingress::IngressContext,
) -> Option<String> {
    let agent = state
        .db
        .get_agent(context.org_id, context.agent_id?)
        .await
        .ok()??;
    let avatar = crate::domains::agents::record::AgentAvatar::from_uuid(agent.avatar_id?);
    Some(avatar.path(crate::domains::agents::avatar::AvatarShape::Circle, 64))
}

fn chat_domain() -> Option<String> {
    let domain = std::env::var("CHAT_DOMAIN").ok()?;
    if domain.is_empty()
        || domain.split('.').any(|label| {
            label.is_empty()
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return None;
    }
    Some(domain)
}

/// `https://<assigned-label>.<CHAT_DOMAIN>`. A channel id is not a hostname.
fn public_url(label: &str) -> Option<String> {
    if !host_label_ok(label) {
        return None;
    }
    Some(format!("https://{label}.{}", chat_domain()?))
}

async fn work_chat_bootstrap(state: &ChannelApiState, id: &str) -> ApiResponse<Response> {
    let (context, channel) = resolve_api_channel(state, id).await?;
    if channel_liveness(&context, &channel).is_err() {
        return Err(not_found());
    }
    let cfg = config(&channel)?;
    let features = validate_agent(state, &context, &channel).await?;
    let host = state
        .db
        .work_chat_host(&channel.public_id.to_string())
        .await
        .map_err(internal_error)?;
    Ok(Json(json!({
        "channel_id": channel.public_id,
        "name": context.name,
        "description": context.description,
        "experience": "work_chat",
        "interface_mode": cfg.interface_mode,
        "membership": cfg.membership,
        "features": features,
        "org_members": cfg.execution.org_members,
        "logo": agent_logo(state, &context).await,
        "public_url": host.as_deref().and_then(public_url)
    }))
    .into_response())
}

fn config(channel: &IngressChannel) -> ApiResponse<WorkChatConfig> {
    if channel.channel_type != ChannelType::WorkChat {
        return Err(not_found());
    }
    serde_json::from_value(channel.channel_config.clone()).map_err(|_| not_found())
}

pub(super) async fn validate_agent(
    state: &ChannelApiState,
    context: &IngressContext,
    channel: &IngressChannel,
) -> ApiResponse<ChatFeatures> {
    let cfg = config(channel)?;
    let agent = context.agent_id.ok_or_else(not_found)?;
    crate::domains::agent_channels::work_chat::validate(
        &state.db,
        context.org_id,
        agent,
        context.harness_id,
        &cfg,
    )
    .await
    .map_err(|err| {
        tracing::warn!(%err, "Work Chat capability requirements no longer satisfied");
        error(
            StatusCode::CONFLICT,
            "This chat's Agent no longer supports its configured interface",
        )
    })
}

#[utoipa::path(get, operation_id="get_work_chat_config", path="/v1/channels/{channel_id}/chat/config", description="Public bootstrap for one live Work Chat channel.", params(("channel_id" = String, Path)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn get_work_chat_config(
    State(state): State<ChannelApiState>,
    Path(id): Path<String>,
) -> Response {
    work_chat_bootstrap(&state, &id)
        .await
        .unwrap_or_else(|response| response)
}

#[utoipa::path(get, operation_id="get_work_chat_by_host", path="/v1/work-chat/hosts/{label}", description="Public bootstrap for the live Work Chat published at this hostname.", params(("label" = String, Path)), responses((status=200,description="Success",body=Value),(status=404,description="No live Work Chat uses this hostname")),tag="agent-execution")]
pub(crate) async fn get_work_chat_by_host(
    State(state): State<ChannelApiState>,
    Path(label): Path<String>,
) -> Response {
    let result = async {
        // A channel id is 32 hex characters. This route does not accept one.
        let label = label.to_ascii_lowercase();
        if !host_label_ok(&label) {
            return Err(not_found());
        }
        let id = state
            .db
            .work_chat_public_id_by_host(&label)
            .await
            .map_err(internal_error)?
            .ok_or_else(not_found)?;
        work_chat_bootstrap(&state, &id).await
    }
    .await;
    result.unwrap_or_else(|response| response)
}

#[utoipa::path(post, operation_id="resolve_work_chat_main", path="/v1/channels/{channel_id}/chat/main", description="Resolve the consumer’s permanent main conversation.", params(("channel_id" = String, Path)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn resolve_work_chat_main(
    State(state): State<ChannelApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let auth = authorize_by_id(&state, &id, &headers, None).await?;
        if config(&auth.channel)?.interface_mode != ChatInterface::SingleWithThreads {
            return Err(error(
                StatusCode::CONFLICT,
                "This chat uses separate conversations",
            ));
        }
        let key = format!("work_chat:main:{}", auth.caller.rate_scope());
        if let Some(session) = state
            .db
            .channel_binding(auth.channel.internal_id, &key)
            .await
            .map_err(internal_error)?
        {
            let row =
                callers_session(&state, &auth, &SessionId::from_uuid(session).to_string()).await?;
            return Ok(Json(AgentSessionView::from(&row)).into_response());
        }
        within_spend_limit(&state, &auth).await?;
        let created = create_api_session(
            &state.db,
            &state.session_service,
            &auth.context,
            &auth.channel,
            &auth.caller,
            Some("Chat".into()),
        )
        .await
        .map_err(|e| super::super::channel_api::command_error_response(e).into_response())?;
        let winner = state
            .db
            .bind_channel_session(auth.channel.internal_id, &key, created.id.uuid())
            .await
            .map_err(internal_error)?;
        if winner != created.id.uuid() {
            state
                .db
                .delete_session(auth.context.org_id, created.id)
                .await
                .map_err(internal_error)?;
            state
                .session_service
                .workspace_file_service()
                .evict_virtual_mounts(created.id.uuid());
        }
        let row = callers_session(&state, &auth, &SessionId::from_uuid(winner).to_string()).await?;
        Ok(Json(AgentSessionView::from(&row)).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}

#[utoipa::path(get, operation_id="list_work_chat_tasks", path="/v1/channels/{channel_id}/sessions/{session_id}/work", description="Background work and threads in this consumer conversation.", params(("channel_id" = String, Path),("session_id" = String, Path)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn list_work_chat_tasks(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let auth = authorize_by_id(&state, &channel, &headers, None).await?;
        config(&auth.channel)?;
        let session = callers_session(&state, &auth, &session).await?;
        let tasks = state
            .db
            .list_session_tasks(session.id, None, None)
            .await
            .map_err(internal_error)?;
        // Expose the consumer board, never task specs, worker identifiers or internal errors.
        let data: Vec<Value> = tasks
            .iter()
            .take(200)
            .map(|task| {
                json!({"id":task.id,"kind":task.kind,
            "title":task.display_name,"state":task.state,"summary":task.summary,
            "links":{"child_session_id":task.links.get("child_session_id").and_then(Value::as_str)},"input_request":task.input_request.as_ref().map(|input|json!({"id":input.get("id").and_then(Value::as_str),"prompt":input.get("prompt").and_then(Value::as_str)}))})
            })
            .collect();
        Ok(Json(json!({"data":data})).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}

#[derive(Deserialize, Default)]
pub(crate) struct FileQuery {
    #[serde(default)]
    path: Option<String>,
}
// THREAT[TM-FS-022]: normalize aliases before excluding private runtime mounts.
fn workspace_path(path: Option<&str>) -> ApiResponse<String> {
    let path = path.unwrap_or("/workspace");
    if path.len() > 2048
        || path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|part| part == "..")
        || !(path == "/workspace"
            || path.starts_with("/workspace/")
            || path == "/attachments"
            || path.starts_with("/attachments/"))
    {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Only workspace files and attachments are available",
        ));
    }
    let canonical = everruns_core::session_path::to_session_path(path);
    let root = canonical
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("");
    if matches!(
        root,
        "memory"
            | "knowledge"
            | "knowledge-text"
            | "skills"
            | "docs"
            | ".background"
            | ".tasks"
            | ".agent-runs"
            | ".everruns"
    ) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "This path is private runtime context",
        ));
    }
    Ok(canonical)
}
async fn file_context(
    state: &ChannelApiState,
    channel: &str,
    session: &str,
    headers: &HeaderMap,
) -> ApiResponse<(Authorized, SessionRow, WorkspaceFileService)> {
    let auth = authorize_by_id(state, channel, headers, None).await?;
    config(&auth.channel)?;
    let features = validate_agent(state, &auth.context, &auth.channel).await?;
    if !features.files {
        return Err(error(
            StatusCode::FORBIDDEN,
            "This Agent has no workspace tools",
        ));
    }
    let session = callers_session(state, &auth, session).await?;
    if session.workspace_id != session.id.uuid() {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Shared workspaces are unavailable to consumers",
        ));
    }
    let mut files = WorkspaceFileService::new(state.db.clone());
    if let Some(registry) = state
        .session_service
        .workspace_file_service()
        .virtual_registry()
    {
        files = files.with_virtual_registry(registry.clone());
    }
    Ok((auth, session, files))
}
#[utoipa::path(get, operation_id="get_work_chat_files", path="/v1/channels/{channel_id}/sessions/{session_id}/files", description="Browse or read authorized workspace files.", params(("channel_id" = String, Path),("session_id" = String, Path),("path" = Option<String>, Query)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn get_work_chat_files(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    Query(query): Query<FileQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (_, session, service) = file_context(&state, &channel, &session, &headers).await?;
        let path = workspace_path(query.path.as_deref())?;
        let file = service
            .read_file(session.workspace_id, &path)
            .await
            .map_err(internal_error)?;
        if let Some(mut file) = file
            && !file.is_directory
        {
            let revision = file_revision(&file);
            file.path = if file.path.starts_with("/attachments/") {
                file.path.clone()
            } else {
                format!("/workspace{}", file.path)
            };
            return Ok(Json(json!({"file":file,"revision":revision})).into_response());
        }
        if service
            .stat(session.workspace_id, &path)
            .await
            .map_err(internal_error)?
            .is_none()
        {
            return Err(error(StatusCode::NOT_FOUND, "File not found"));
        }
        let data = service
            .list_directory(session.workspace_id, &path)
            .await
            .map_err(internal_error)?;
        let data: Vec<_> = data
            .into_iter()
            .filter_map(|mut file| {
                let display =
                    if file.path.starts_with("/attachments/") || file.path == "/attachments" {
                        file.path.clone()
                    } else {
                        format!("/workspace{}", file.path)
                    };
                workspace_path(Some(&display)).ok()?;
                file.path = display;
                Some(file)
            })
            .collect();
        Ok(Json(json!({"data":data})).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}
/// Revision-checked creation or update of one consumer workspace file.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileWrite {
    #[serde(default)]
    /// Revision returned by the previous read; required when replacing a file.
    #[schema(example = "f6d4cf68d1e6c37f7d24a5a85db31ba4")]
    expected_revision: Option<String>,
    /// Absolute path within the consumer workspace.
    #[schema(example = "/workspace/notes.md")]
    path: String,
    /// UTF-8 text or base64-encoded bytes.
    #[schema(example = "Meeting notes")]
    content: String,
    #[serde(default)]
    /// Content encoding, either text or base64.
    #[schema(example = "text")]
    encoding: Option<String>,
}
#[utoipa::path(post, operation_id="write_work_chat_file", path="/v1/channels/{channel_id}/sessions/{session_id}/files", description="Create or update a writable workspace file.", params(("channel_id" = String, Path),("session_id" = String, Path),("path" = Option<String>, Query)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn write_work_chat_file(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<FileWrite>,
) -> Response {
    let result = async {
        let (_, session, service) = file_context(&state, &channel, &session, &headers).await?;
        let path = workspace_path(Some(&body.path))?;
        if body.content.len() > 4 * 1024 * 1024 {
            return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "File is too large"));
        }
        let existing = service
            .read_file(session.workspace_id, &path)
            .await
            .map_err(internal_error)?;
        let file = if let Some(existing) = existing {
            if existing.is_readonly || existing.is_directory {
                return Err(error(StatusCode::FORBIDDEN, "File is not writable"));
            }
            if body.expected_revision.as_deref() != Some(file_revision(&existing).as_str()) {
                return Err(error(
                    StatusCode::CONFLICT,
                    "File changed. Reload it before saving",
                ));
            }
            service
                .update_file_if_content_matches(
                    session.workspace_id,
                    &path,
                    existing.content.as_deref().unwrap_or(""),
                    &existing.encoding,
                    &body.content,
                    body.encoding.as_deref().unwrap_or("text"),
                )
                .await
        } else {
            if body.expected_revision.is_some() {
                return Err(error(
                    StatusCode::CONFLICT,
                    "File changed. Reload it before saving",
                ));
            }
            service
                .create_file(
                    session.workspace_id,
                    CreateFileInput {
                        path,
                        content: Some(body.content),
                        encoding: body.encoding,
                        is_readonly: None,
                    },
                )
                .await
                .map(Some)
        }
        .map_err(|e| {
            tracing::debug!(%e,"Consumer file write refused");
            error(StatusCode::FORBIDDEN, "File could not be written")
        })?;
        let file = file.ok_or_else(|| {
            error(
                StatusCode::CONFLICT,
                "File changed. Reload it before saving",
            )
        })?;
        Ok(Json(file).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}
#[utoipa::path(delete, operation_id="delete_work_chat_file", path="/v1/channels/{channel_id}/sessions/{session_id}/files", description="Delete a writable workspace file.", params(("channel_id" = String, Path),("session_id" = String, Path),("path" = Option<String>, Query)), responses((status=200,description="Success",body=Value),(status=401,description="Consumer sign-in required"),(status=404,description="Channel or resource unavailable")),tag="agent-execution")]
pub(crate) async fn delete_work_chat_file(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    Query(query): Query<FileQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let (_, session, service) = file_context(&state, &channel, &session, &headers).await?;
        let path = workspace_path(query.path.as_deref())?;
        if path == "/" || path == "/attachments" {
            return Err(error(
                StatusCode::FORBIDDEN,
                "Cannot delete a workspace root",
            ));
        }
        let deleted = service
            .delete(session.workspace_id, &path, false)
            .await
            .map_err(|_| error(StatusCode::FORBIDDEN, "File could not be deleted"))?;
        Ok(Json(json!({"deleted":deleted})).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}

/// `send_message` and `no_reply` are the reply, not a work step. Their narration
/// is the message body, which the transcript shows on its own.
fn communication_tool(event: &Value) -> bool {
    let data = &event["data"];
    let name = data["tool_call"]["name"]
        .as_str()
        .or_else(|| data["tool_name"].as_str())
        .unwrap_or("");
    matches!(name, "send_message" | "channel_post_message" | "no_reply")
}

/// Work Chat projects the canonical stream rather than exposing debug events.
pub(super) fn project(
    event: Value,
    config: &crate::domains::agent_channels::record::api::AgentApiChannelConfig,
) -> Option<Value> {
    let kind = event["type"].as_str()?;
    if kind == "conversation.message" {
        let text = event["data"]["text"].as_str()?;
        if text.trim().is_empty() {
            return None;
        }
        return Some(json!({
            "id": event["id"],
            "type": kind,
            "ts": event["ts"],
            "session_id": event["session_id"],
            "sequence": event["sequence"],
            "data": { "message_id": event["data"]["message_id"], "text": text }
        }));
    }
    if matches!(kind, "tool.started" | "tool.completed") && communication_tool(&event) {
        return None;
    }
    if !matches!(
        kind,
        "input.message"
            | "output.message.completed"
            | "output.message.delta"
            | "turn.started"
            | "turn.completed"
            | "turn.cancelled"
            | "turn.failed"
            | "tool.started"
            | "tool.completed"
            | "tool.call_requested"
            | "session.title_updated"
    ) {
        return None;
    }
    // Public summaries retain progress without exposing tool results, internal
    // errors, routing metadata or provider/debug fields from full API events.
    let data = if kind == "output.message.delta" {
        json!({"message_id":event["data"]["message_id"],"delta":event["data"]["delta"],"accumulated":event["data"]["accumulated"]})
    } else if kind == "session.title_updated" {
        json!({"title":event["data"]["title"]})
    } else {
        let mut policy = config.clone();
        if policy.visibility == crate::domains::agent_channels::record::api::ApiVisibility::Full {
            policy.visibility =
                crate::domains::agent_channels::record::api::ApiVisibility::Activity;
        }
        let mut data = project_event(event.clone(), &policy)?["data"].clone();
        if config.visibility == crate::domains::agent_channels::record::api::ApiVisibility::Full {
            if kind == "tool.started" {
                data["tool_name"] = event["data"]["tool_call"]["name"].clone();
                data["display_name"] = event["data"]["display_name"].clone();
                data["narration"] = event["data"]["narration"].clone();
            } else if kind == "tool.completed" {
                data["tool_name"] = event["data"]["tool_name"].clone();
                data["display_name"] = event["data"]["display_name"].clone();
            }
        }
        data
    };
    Some(
        json!({"id":event["id"],"type":event["type"],"ts":event["ts"],"session_id":event["session_id"],"sequence":event["sequence"],"data":data}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_private_mounts_and_traversal() {
        for path in [
            "/memory/user",
            "/workspace/../memory/user",
            "/workspace\\secret",
            "/secrets",
            "/attachments/../../other",
            "/workspace/memory/user",
            "/workspace/.tasks/private",
            "/workspace//memory/user",
            "/workspace/knowledge",
            "/workspace/knowledge/kn_fixture/private.md",
            "/workspace/knowledge-text/kn_fixture/private.md",
            "/workspace//knowledge/kn_fixture/private.md",
        ] {
            assert!(workspace_path(Some(path)).is_err(), "{path}");
        }
        assert!(workspace_path(Some("/workspace/report.md")).is_ok());
    }
    #[test]
    fn work_projection_never_includes_reasoning_or_debug_records() {
        let config = Default::default();
        for kind in [
            "reason.started",
            "reason.completed",
            "llm.generation",
            "tool.debug",
            "runtime.invocation",
        ] {
            assert!(project(json!({"type":kind,"data":{"secret":"hidden"}}), &config).is_none());
        }
    }

    #[test]
    fn a_sent_message_is_the_reply_and_not_a_work_step() {
        use crate::domains::agent_channels::record::api::{AgentApiChannelConfig, ApiVisibility};
        let config = AgentApiChannelConfig {
            visibility: ApiVisibility::Full,
            ..Default::default()
        };
        let sent = project(
            json!({
                "id": "evt_sent",
                "type": "conversation.message",
                "ts": "2026-10-30T00:00:00Z",
                "session_id": "session_1",
                "sequence": 3,
                "data": {
                    "message_id": "message_1",
                    "text": "It's the **About** panel.",
                    "tool_call_id": "call_1",
                    "delivery": {"platform": "slack", "channel": "C1", "message_ref": "1.2"}
                }
            }),
            &config,
        )
        .expect("sent message");
        assert_eq!(sent["data"]["text"], "It's the **About** panel.");
        assert!(sent["data"].get("tool_call_id").is_none());
        assert!(sent["data"].get("delivery").is_none());
        assert!(
            project(
                json!({"type":"conversation.message","data":{"text":"  "}}),
                &config
            )
            .is_none()
        );
        for event in [
            json!({
                "type": "tool.started",
                "data": {
                    "tool_call": {"id": "call_1", "name": "send_message"},
                    "narration": "Sending message: secret preview"
                }
            }),
            json!({
                "type": "tool.completed",
                "data": {"tool_call_id": "call_1", "tool_name": "no_reply", "success": true}
            }),
            json!({
                "type": "tool.started",
                "data": {"tool_call": {"id": "call_9", "name": "channel_post_message"}}
            }),
        ] {
            assert!(project(event, &config).is_none());
        }
        let read = project(
            json!({
                "id": "evt_tool",
                "type": "tool.started",
                "ts": "2026-10-30T00:00:00Z",
                "session_id": "session_1",
                "sequence": 2,
                "data": {
                    "tool_call": {"id": "call_2", "name": "read_file"},
                    "narration": "Reading report.md"
                }
            }),
            &config,
        )
        .expect("real tool");
        assert_eq!(read["data"]["narration"], "Reading report.md");
    }

    #[test]
    fn public_url_uses_the_assigned_host_not_the_channel_id() {
        unsafe { std::env::set_var("CHAT_DOMAIN", "chat.example") };
        assert_eq!(
            public_url("k3m9pq2r7x").as_deref(),
            Some("https://k3m9pq2r7x.chat.example")
        );
        assert!(public_url("01933b5a000070008000000000000001").is_none());
        assert!(public_url("appchan_01933b5a000070008000000000000001").is_none());
        unsafe { std::env::set_var("CHAT_DOMAIN", "not a domain") };
        assert!(public_url("k3m9pq2r7x").is_none());
        unsafe { std::env::set_var("CHAT_DOMAIN", "chat.example") };
    }
}
