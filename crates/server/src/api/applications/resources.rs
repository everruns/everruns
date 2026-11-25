//! Narrow consumer adapters: server-selected resources and member attribution.
use super::*;
use crate::domains::memory::files::{MemoryFileService, NewFileInput};
use crate::domains::{
    common::Command,
    rooms::{ListRoomMessages, ListRoomReplies, PostRoomMessage},
};
use axum::extract::Query;
use serde::Deserialize;

async fn resource(
    state: &AppState,
    id: ApplicationId,
    slug: &str,
    headers: &HeaderMap,
    kind: ApplicationResourceKind,
) -> Result<(crate::auth::runtime::RuntimeAccount, ApplicationResource), StatusCode> {
    // THREAT[TM-AUTHZ-029]: Resolve only a live binding inside the admitted Application.
    let caller = account(state, id, headers).await?;
    let resource = state
        .rooms
        .db
        .application_resources(id.uuid())
        .await
        .map_err(internal)?
        .into_iter()
        .find(|r| r.slug == slug && r.kind == kind)
        .ok_or(StatusCode::NOT_FOUND)?;
    if kind == ApplicationResourceKind::Room {
        let room = state
            .rooms
            .db
            .get_room(
                caller.org_id,
                resource.resource_id.parse().map_err(internal)?,
            )
            .await
            .map_err(internal)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if room.archived_at.is_some() {
            return Err(StatusCode::NOT_FOUND);
        }
        // A removed participant cannot rejoin just by sending another request.
        let user = state
            .rooms
            .db
            .get_virtual_user(caller.org_id, caller.id)
            .await
            .map_err(internal)?
            .ok_or(StatusCode::UNAUTHORIZED)?;
        state
            .rooms
            .db
            .application_room_member(caller.org_id, id, caller.id, room.id, &user.name)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
    }
    Ok((caller, resource))
}
#[utoipa::path(get,operation_id="application_room",description="Read the bound Room and its public participant roster.",path="/v1/applications/{application_id}/rooms/{slug}",params(("application_id"=String,Path),("slug"=String,Path)),responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_room(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let (caller, r) = resource(&state, id, &slug, &headers, ApplicationResourceKind::Room).await?;
    let room_id = r.resource_id.parse().map_err(internal)?;
    let room = state
        .rooms
        .db
        .get_room(caller.org_id, room_id)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let members = state
        .rooms
        .db
        .list_room_members(room_id, false)
        .await
        .map_err(internal)?;
    let members: Vec<_> = members
        .into_iter()
        .map(|m| json!({"id":m.id,"handle":m.handle,"display_name":m.display_name,"kind":m.kind}))
        .collect();
    Ok(Json(
        json!({"name":r.name,"topic":room.topic,"members":members}),
    ))
}
#[derive(Default, Deserialize)]
pub struct Page {
    before: Option<chrono::DateTime<chrono::Utc>>,
    after: Option<chrono::DateTime<chrono::Utc>>,
    limit: Option<i64>,
}
fn public_messages(mut value: Value) -> Value {
    if let Some(messages) = value.get_mut("data").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(m) = message.as_object_mut() {
                m.remove("session_id");
                m.remove("url");
            }
        }
    }
    value
}
#[utoipa::path(get,operation_id="application_messages",description="Read a bounded page of messages from the bound Room.",path="/v1/applications/{application_id}/rooms/{slug}/messages",params(("application_id"=String,Path),("slug"=String,Path),("before"=Option<String>,Query),("after"=Option<String>,Query),("limit"=Option<i64>,Query)),responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_messages(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    Query(page): Query<Page>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let (caller, r) = resource(&state, id, &slug, &headers, ApplicationResourceKind::Room).await?;
    let result = ListRoomMessages {
        room_id: r.resource_id,
        before: page.before,
        after: page.after,
        limit: page.limit,
    }
    .execute(&state.consumer_ctx(caller.org_id))
    .await
    .map_err(command_error)?;
    Ok(Json(public_messages(
        serde_json::to_value(result).map_err(internal)?,
    )))
}
#[utoipa::path(get,operation_id="application_replies",description="Read replies in a thread of the bound Room.",path="/v1/applications/{application_id}/rooms/{slug}/replies/{message_id}",params(("application_id"=String,Path),("slug"=String,Path),("message_id"=String,Path),("before"=Option<String>,Query),("after"=Option<String>,Query),("limit"=Option<i64>,Query)),responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_replies(
    State(state): State<AppState>,
    Path((id, slug, message_id)): Path<(ApplicationId, String, String)>,
    Query(page): Query<Page>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let (caller, r) = resource(&state, id, &slug, &headers, ApplicationResourceKind::Room).await?;
    let result = ListRoomReplies {
        room_id: r.resource_id,
        message_id,
        before: page.before,
        after: page.after,
        limit: page.limit,
    }
    .execute(&state.consumer_ctx(caller.org_id))
    .await
    .map_err(command_error)?;
    Ok(Json(public_messages(
        serde_json::to_value(result).map_err(internal)?,
    )))
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Post {
    text: String,
    #[serde(default)]
    reply_to: Option<String>,
}
#[utoipa::path(post,operation_id="application_post_message",description="Post as the signed-in Application participant; caller-selected attribution is rejected.",path="/v1/applications/{application_id}/rooms/{slug}/messages",params(("application_id"=String,Path),("slug"=String,Path)),request_body=Post,responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_post_message(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    headers: HeaderMap,
    Json(input): Json<Post>,
) -> Result<Json<Value>, StatusCode> {
    let (caller, r) = resource(&state, id, &slug, &headers, ApplicationResourceKind::Room).await?;
    let user = state
        .rooms
        .db
        .get_virtual_user(caller.org_id, caller.id)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let member = state
        .rooms
        .db
        .application_room_member(
            caller.org_id,
            id,
            caller.id,
            r.resource_id.parse().map_err(internal)?,
            &user.name,
        )
        .await
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let result = PostRoomMessage {
        room_id: r.resource_id,
        text: input.text,
        reply_to: input.reply_to,
        as_member: Some(member.to_string()),
    }
    .execute(&state.consumer_ctx(caller.org_id))
    .await
    .map_err(command_error)?;
    let mut message = serde_json::to_value(result.message).map_err(internal)?;
    if let Some(m) = message.as_object_mut() {
        m.remove("session_id");
        m.remove("url");
    }
    Ok(Json(message))
}
#[derive(Deserialize)]
pub struct MemoryPath {
    #[serde(default = "root")]
    path: String,
    #[serde(default)]
    file: bool,
}
fn root() -> String {
    "/".into()
}
async fn memory_target(
    state: &AppState,
    id: ApplicationId,
    slug: &str,
    headers: &HeaderMap,
    write: bool,
) -> Result<(MemoryFileService, crate::storage::MemoryRow), StatusCode> {
    let (caller, r) = resource(state, id, slug, headers, ApplicationResourceKind::Memory).await?;
    if write && !r.writable {
        return Err(StatusCode::FORBIDDEN);
    }
    let service = MemoryFileService::new(state.rooms.db.clone());
    let memory = service
        .resolve_memory(caller.org_id, &r.resource_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if memory.status != "active" || memory.scope != "org" || (write && memory.is_readonly) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok((service, memory))
}
#[utoipa::path(get,operation_id="application_memory",description="List shared memory files or read a text file from the bound organization Memory.",path="/v1/applications/{application_id}/memory/{slug}",params(("application_id"=String,Path),("slug"=String,Path),("path"=Option<String>,Query),("file"=Option<bool>,Query)),responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_memory(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    Query(path): Query<MemoryPath>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let (service, memory) = memory_target(&state, id, &slug, &headers, false).await?;
    if path.file {
        let file = service
            .read_file(memory.id, &path.path)
            .await
            .map_err(memory_error)?;
        if file.content.len() > 1024 * 1024 {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let text =
            String::from_utf8(file.content).map_err(|_| StatusCode::UNSUPPORTED_MEDIA_TYPE)?;
        Ok(Json(json!({"path":file.path,"content":text})))
    } else {
        let files = service
            .list_directory(memory.id, &path.path)
            .await
            .map_err(memory_error)?;
        Ok(Json(json!({"files":files})))
    }
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteMemory {
    path: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    is_directory: bool,
    #[serde(default)]
    create: bool,
}
#[utoipa::path(put,operation_id="application_write_memory",description="Create or update a text file when the bound Memory grants write access.",path="/v1/applications/{application_id}/memory/{slug}",params(("application_id"=String,Path),("slug"=String,Path)),request_body=WriteMemory,responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_write_memory(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    headers: HeaderMap,
    Json(input): Json<WriteMemory>,
) -> Result<Json<Value>, StatusCode> {
    let (service, memory) = memory_target(&state, id, &slug, &headers, true).await?;
    if input.content.len() > 1024 * 1024 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let row = if input.create {
        service
            .create_file(
                &memory,
                NewFileInput {
                    path: input.path,
                    content: Some(input.content.into_bytes()),
                    is_directory: input.is_directory,
                },
            )
            .await
    } else {
        service
            .update_file(&memory, &input.path, input.content.into_bytes())
            .await
    }
    .map_err(memory_error)?;
    Ok(Json(json!({"path":row.path,"size_bytes":row.size_bytes})))
}
#[utoipa::path(delete,operation_id="application_delete_memory",description="Delete a file or empty directory when the bound Memory grants write access.",path="/v1/applications/{application_id}/memory/{slug}",params(("application_id"=String,Path),("slug"=String,Path),("path"=Option<String>,Query),("file"=Option<bool>,Query)),responses((status=200,description="Success",body=serde_json::Value),(status=204,description="Deleted"),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Resource unavailable")),tag="applications")]
pub async fn application_delete_memory(
    State(state): State<AppState>,
    Path((id, slug)): Path<(ApplicationId, String)>,
    Query(path): Query<MemoryPath>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    let (service, memory) = memory_target(&state, id, &slug, &headers, true).await?;
    service
        .delete(&memory, &path.path, false)
        .await
        .map_err(memory_error)?;
    Ok(StatusCode::NO_CONTENT)
}
fn memory_error(error: crate::domains::memory::files::MemoryFsError) -> StatusCode {
    use crate::domains::memory::files::MemoryFsError::*;
    match error {
        NotFound | MemoryNotFound => StatusCode::NOT_FOUND,
        Readonly => StatusCode::FORBIDDEN,
        Conflict(_) => StatusCode::CONFLICT,
        Other(error) => internal(error),
        _ => StatusCode::BAD_REQUEST,
    }
}
