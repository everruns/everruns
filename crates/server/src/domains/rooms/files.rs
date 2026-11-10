//! Room files: a shared folder per room for what should outlast the chat
//! (notes, plans, decisions, data), edited in place, with every version kept
//! (knowledge/runtime-resources/rooms.md).
//!
//! Members read and change them here, through the API, the CLI and the MCP
//! room tools. An agent's room session sees the same files at `/room` and
//! changes them with its ordinary file tools; those changes are versioned
//! under its handle too (`session_files::memory_mounts`).
//!
//! Access follows messages: every org member can read a room's files, and
//! changing one makes the caller a member, as posting does.

use super::queries::{self as q};
use super::rules;
use super::types::RoomAuthor;
use crate::domains::common::*;
use crate::storage::{
    RoomFileAuthor, RoomFileChange, RoomFileRow, RoomFileStore, RoomFileVersionRow, RoomMemberRow,
    RoomRow,
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Largest room file, in bytes.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Most edits in one `edit_room_file` call.
const MAX_EDITS: usize = 64;

// ============================================================================
// API shapes
// ============================================================================

/// A file in a room's shared folder, as it is now.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFile {
    /// Path in the room's folder, from its root.
    #[schema(example = "/plans/launch.md")]
    pub path: String,
    /// The file's current version; every change adds one.
    #[schema(example = 3)]
    pub version: i32,
    /// Size of the current content, in bytes.
    #[schema(example = 2048)]
    pub size_bytes: i64,
    /// Who made the current version, when a member did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<RoomAuthor>,
    /// When the file last changed.
    #[schema(example = "2026-11-10T09:30:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// A room's files.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFileListResponse {
    /// The files, by path.
    pub data: Vec<RoomFile>,
}

/// One version of a room file: one change and who made it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFileVersion {
    /// Path of the file after this change.
    #[schema(example = "/plans/launch.md")]
    pub path: String,
    /// Version number, from 1, per file.
    #[schema(example = 3)]
    pub version: i32,
    /// `write`, `move`, `delete` or `restore`.
    #[schema(example = "write")]
    pub action: String,
    /// Size of the content after this change, in bytes; 0 for a delete.
    #[schema(example = 2048)]
    pub size_bytes: i64,
    /// For a move: where the file was before.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "/plan.md")]
    pub moved_from: Option<String>,
    /// The member who made the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<RoomAuthor>,
    /// When the change was made.
    #[schema(example = "2026-11-10T09:30:00Z")]
    pub created_at: DateTime<Utc>,
}

/// A room file's history, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFileHistoryResponse {
    /// Versions, newest first.
    pub data: Vec<RoomFileVersion>,
}

/// A room file's content at one version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFileContent {
    /// Path of the file at that version.
    #[schema(example = "/plans/launch.md")]
    pub path: String,
    /// The version read.
    #[schema(example = 3)]
    pub version: i32,
    /// `text` (UTF-8) or `base64` (anything else).
    #[schema(example = "text")]
    pub encoding: String,
    /// The content, in `encoding`.
    #[schema(example = "# Launch plan\n\n- [ ] Draft the announcement\n")]
    pub content: String,
    /// Size of the content, in bytes.
    #[schema(example = 2048)]
    pub size_bytes: i64,
    /// Who made that version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<RoomAuthor>,
    /// When that version was made.
    #[schema(example = "2026-11-10T09:30:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// One replacement in an edit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoomFileEdit {
    /// Text to replace. It must occur exactly once in the file.
    #[schema(example = "- [ ] Draft the announcement")]
    pub old_text: String,
    /// What to put in its place; empty deletes it.
    #[schema(example = "- [x] Draft the announcement")]
    pub new_text: String,
}

// ============================================================================
// Shared steps
// ============================================================================

async fn store(ctx: &Ctx, room: &RoomRow) -> Result<RoomFileStore, CommandError> {
    Ok(ctx.db.ensure_room_memory(room.org_id, room.id).await?)
}

fn clean_path(raw: &str) -> Result<String, CommandError> {
    rules::clean_file_path(raw).map_err(CommandError::bad_request)
}

async fn authors(ctx: &Ctx, room: &RoomRow) -> Result<HashMap<Uuid, RoomMemberRow>, CommandError> {
    Ok(ctx
        .db
        .list_room_members(room.id, true)
        .await?
        .into_iter()
        .map(|member| (member.id.uuid(), member))
        .collect())
}

fn author_of(
    members: &HashMap<Uuid, RoomMemberRow>,
    id: Option<crate::kernel_imports::contracts::typed_id::RoomMemberId>,
) -> Option<RoomAuthor> {
    id.and_then(|id| members.get(&id.uuid())).map(q::author)
}

fn version_out(
    members: &HashMap<Uuid, RoomMemberRow>,
    row: &RoomFileVersionRow,
) -> RoomFileVersion {
    RoomFileVersion {
        path: row.path.clone(),
        version: row.version,
        action: row.action.clone(),
        size_bytes: row.size_bytes,
        moved_from: row.moved_from.clone(),
        author: author_of(members, row.author_member_id),
        created_at: row.created_at,
    }
}

fn file_out(members: &HashMap<Uuid, RoomMemberRow>, row: &RoomFileRow) -> RoomFile {
    RoomFile {
        path: row.path.clone(),
        version: row.version,
        size_bytes: row.size_bytes,
        updated_by: author_of(members, row.author_member_id),
        updated_at: row.updated_at,
    }
}

/// Turn a storage outcome into the version it made, or the error to report.
fn change_result(change: RoomFileChange, path: &str) -> Result<RoomFileVersionRow, CommandError> {
    match change {
        RoomFileChange::Done(row) => Ok(row),
        RoomFileChange::Conflict { current } => Err(CommandError::conflict(match current {
            Some(version) => {
                format!("{path} changed: it is at version {version}. Read it again and retry.")
            }
            None => format!("{path} does not exist any more. Read the folder again."),
        })),
        RoomFileChange::NotFound => Err(CommandError::not_found_msg(format!("No file at {path}"))),
        RoomFileChange::WrongKind(why) => Err(CommandError::bad_request(why)),
    }
}

/// The member changing a file, joining the room if needed, in an open room.
async fn writer(
    ctx: &Ctx,
    room: &RoomRow,
    as_member: Option<&str>,
) -> Result<RoomMemberRow, CommandError> {
    q::require_open(room)?;
    q::acting_member(ctx, room, as_member, true).await
}

async fn written(
    ctx: &Ctx,
    room: &RoomRow,
    change: RoomFileChange,
    path: &str,
) -> Result<RoomFileVersion, CommandError> {
    let row = change_result(change, path)?;
    Ok(version_out(&authors(ctx, room).await?, &row))
}

fn decode(content: &str, encoding: Option<&str>) -> Result<Vec<u8>, CommandError> {
    let bytes = match encoding.unwrap_or("text") {
        "text" => content.as_bytes().to_vec(),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(content)
            .map_err(|_| CommandError::bad_request("content is not valid base64"))?,
        other => {
            return Err(CommandError::bad_request(format!(
                "encoding must be text or base64, not {other}"
            )));
        }
    };
    if bytes.len() > MAX_FILE_BYTES {
        return Err(CommandError::bad_request(format!(
            "A room file is at most {MAX_FILE_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

fn encode(bytes: Vec<u8>) -> (String, String) {
    match String::from_utf8(bytes) {
        Ok(text) => ("text".into(), text),
        Err(error) => (
            "base64".into(),
            base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
        ),
    }
}

/// Apply replacements in order; each `old_text` must occur exactly once.
pub fn apply_edits(text: &str, edits: &[RoomFileEdit]) -> Result<String, String> {
    if edits.is_empty() {
        return Err("Give at least one edit".into());
    }
    if edits.len() > MAX_EDITS {
        return Err(format!("At most {MAX_EDITS} edits at a time"));
    }
    let mut out = text.to_string();
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(format!("Edit {}: old_text is empty", index + 1));
        }
        match out.matches(edit.old_text.as_str()).count() {
            1 => out = out.replacen(edit.old_text.as_str(), &edit.new_text, 1),
            0 => return Err(format!("Edit {}: old_text is not in the file", index + 1)),
            n => {
                return Err(format!(
                    "Edit {}: old_text occurs {n} times; include more around it",
                    index + 1
                ));
            }
        }
    }
    Ok(out)
}

// ============================================================================
// Commands
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomFiles {
    #[param(ignore)]
    pub room_id: String,
    /// Only files under this folder, such as `/plans`.
    pub prefix: Option<String>,
}

#[command(
    name = "list_room_files",
    summary = "List a room's shared files",
    category = "rooms",
    description = "List the files in a room's shared folder, with each file's current version and who made it.",
    method = "GET",
    path = "/v1/rooms/{room_id}/files",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "list").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("See what notes and plans a room keeps", "everruns rooms files list room_01h9")]),
    positional = "room_id",
    http = plain,
    params(ListRoomFiles),
)]
impl Command for ListRoomFiles {
    type Output = RoomFileListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileListResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let prefix = match self.prefix.as_deref() {
            Some(raw) => Some(clean_path(raw)?),
            None => None,
        };
        let store = store(ctx, &room).await?;
        let members = authors(ctx, &room).await?;
        let data = ctx
            .db
            .list_room_files(store)
            .await?
            .iter()
            .filter(|row| {
                prefix.as_deref().is_none_or(|prefix| {
                    row.path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
            })
            .map(|row| file_out(&members, row))
            .collect();
        Ok(RoomFileListResponse { data })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReadRoomFile {
    #[param(ignore)]
    pub room_id: String,
    /// The file, such as `/plans/launch.md`.
    pub path: String,
    /// An earlier version to read; the current one when absent. Works for a
    /// deleted file too.
    pub version: Option<i32>,
}

#[command(
    name = "read_room_file",
    summary = "Read a room file",
    category = "rooms",
    description = "Read a file in a room's shared folder, as it is now or at an earlier version.",
    method = "GET",
    path = "/v1/rooms/{room_id}/files/content",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "read").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("Read a room's launch plan", "everruns rooms files read room_01h9 --path /plans/launch.md")]),
    positional = "room_id",
    http = plain,
    params(ReadRoomFile),
)]
impl Command for ReadRoomFile {
    type Output = RoomFileContent;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileContent, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let store = store(ctx, &room).await?;
        let not_found = || CommandError::not_found_msg(format!("No file at {path}"));
        let (file_id, latest) = ctx
            .db
            .find_room_file(store, &path)
            .await?
            .ok_or_else(not_found)?;
        let members = authors(ctx, &room).await?;
        let (row, content) = match self.version {
            Some(version) => ctx
                .db
                .get_room_file_version(room.id, file_id, version)
                .await?
                .ok_or_else(|| {
                    CommandError::not_found_msg(format!("{path} has no version {version}"))
                })?,
            None => {
                let content = ctx
                    .db
                    .read_room_file(store, &path)
                    .await?
                    .ok_or_else(not_found)?;
                let row = latest.ok_or_else(not_found)?;
                (row, Some(content))
            }
        };
        if row.action == "delete" {
            return Err(CommandError::not_found_msg(format!(
                "{path} was deleted at version {}; read an earlier version",
                row.version
            )));
        }
        let content = content.unwrap_or_default();
        let size_bytes = content.len() as i64;
        let (encoding, content) = encode(content);
        Ok(RoomFileContent {
            path: row.path.clone(),
            version: row.version,
            encoding,
            content,
            size_bytes,
            author: author_of(&members, row.author_member_id),
            updated_at: row.created_at,
        })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct WriteRoomFile {
    pub room_id: String,
    /// The file, such as `/plans/launch.md`. Folders are created as needed.
    pub path: String,
    /// The whole new content.
    pub content: String,
    /// `text` (default) or `base64`.
    pub encoding: Option<String>,
    /// The version you read, so a change someone made since is not
    /// overwritten. `0` writes only when there is no file yet.
    pub if_version: Option<i32>,
    /// Write as this member (org admins, for a product acting for its users).
    pub as_member: Option<String>,
}

#[command(
    name = "write_room_file",
    summary = "Write a room file",
    category = "rooms",
    description = "Create a file in a room's shared folder, or replace its whole content. The earlier version is kept.",
    method = "PUT",
    path = "/v1/rooms/{room_id}/files",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "write").with_examples(&[CliExample::new("Start a shared plan in a room", "everruns rooms files write --room-id room_01h9 --path /plans/launch.md --content '# Launch plan'")]),
    http = plain,
)]
impl Command for WriteRoomFile {
    type Output = RoomFileVersion;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileVersion, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let content = decode(&self.content, self.encoding.as_deref())?;
        let member = writer(ctx, &room, self.as_member.as_deref()).await?;
        let store = store(ctx, &room).await?;
        let change = ctx
            .db
            .write_room_file(
                store,
                &path,
                &content,
                self.if_version,
                "write",
                None,
                RoomFileAuthor {
                    member_id: Some(member.id),
                    session_id: None,
                },
            )
            .await?;
        written(ctx, &room, change, &path).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct EditRoomFile {
    pub room_id: String,
    /// The file to change.
    pub path: String,
    /// Replacements, applied in order. Each `old_text` must occur exactly
    /// once in the file.
    pub edits: Vec<RoomFileEdit>,
    /// Edit as this member (org admins).
    pub as_member: Option<String>,
}

#[command(
    name = "edit_room_file",
    summary = "Edit part of a room file",
    category = "rooms",
    description = "Change part of a text file in a room's shared folder by replacing exact text, without rewriting the rest. Fails, and changes nothing, when the file changed under you.",
    method = "POST",
    path = "/v1/rooms/{room_id}/files/edit",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "edit").with_examples(&[CliExample::new("Tick an item off a room's plan", "everruns rooms files edit --room-id room_01h9 --path /plans/launch.md --edits '[{\"old_text\":\"- [ ] Draft\",\"new_text\":\"- [x] Draft\"}]'")]),
    http = plain,
)]
impl Command for EditRoomFile {
    type Output = RoomFileVersion;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileVersion, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let member = writer(ctx, &room, self.as_member.as_deref()).await?;
        let store = store(ctx, &room).await?;
        let not_found = || CommandError::not_found_msg(format!("No file at {path}"));
        let (_, latest) = ctx
            .db
            .find_room_file(store, &path)
            .await?
            .ok_or_else(not_found)?;
        let bytes = ctx
            .db
            .read_room_file(store, &path)
            .await?
            .ok_or_else(not_found)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CommandError::bad_request(format!("{path} is not a text file")))?;
        let edited = apply_edits(&text, &self.edits).map_err(CommandError::bad_request)?;
        let edited = decode(&edited, None)?;
        let change = ctx
            .db
            .write_room_file(
                store,
                &path,
                &edited,
                // The version the edits were made against: a write in
                // between fails this one instead of being lost.
                Some(latest.map_or(0, |row| row.version)),
                "write",
                None,
                RoomFileAuthor {
                    member_id: Some(member.id),
                    session_id: None,
                },
            )
            .await?;
        written(ctx, &room, change, &path).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MoveRoomFile {
    pub room_id: String,
    /// The file's path now.
    pub from: String,
    /// Its new path. Nothing may be there yet.
    pub to: String,
    /// Move as this member (org admins).
    pub as_member: Option<String>,
}

#[command(
    name = "move_room_file",
    summary = "Move or rename a room file",
    category = "rooms",
    description = "Move or rename a file in a room's shared folder. The file keeps its history.",
    method = "POST",
    path = "/v1/rooms/{room_id}/files/move",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "move").with_examples(&[CliExample::new("File a draft plan under plans", "everruns rooms files move --room-id room_01h9 --from /draft.md --to /plans/launch.md")]),
    http = plain,
)]
impl Command for MoveRoomFile {
    type Output = RoomFileVersion;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileVersion, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let from = clean_path(&self.from)?;
        let to = clean_path(&self.to)?;
        if from == to {
            return Err(CommandError::bad_request("from and to are the same path"));
        }
        let member = writer(ctx, &room, self.as_member.as_deref()).await?;
        let store = store(ctx, &room).await?;
        let change = ctx
            .db
            .move_room_file(
                store,
                &from,
                &to,
                RoomFileAuthor {
                    member_id: Some(member.id),
                    session_id: None,
                },
            )
            .await?;
        written(ctx, &room, change, &from).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteRoomFile {
    #[param(ignore)]
    pub room_id: String,
    /// The file to delete.
    pub path: String,
    /// The version you read; the delete fails when the file changed since.
    pub if_version: Option<i32>,
    /// Delete as this member (org admins).
    pub as_member: Option<String>,
}

#[command(
    name = "delete_room_file",
    summary = "Delete a room file",
    category = "rooms",
    description = "Delete a file from a room's shared folder. Its versions stay: it can be read at an earlier version or restored.",
    method = "DELETE",
    path = "/v1/rooms/{room_id}/files",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "delete").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("Remove a finished draft from a room", "everruns rooms files delete room_01h9 --path /draft.md")]),
    positional = "room_id",
    http = plain,
    params(DeleteRoomFile),
)]
impl Command for DeleteRoomFile {
    type Output = RoomFileVersion;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileVersion, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let member = writer(ctx, &room, self.as_member.as_deref()).await?;
        let store = store(ctx, &room).await?;
        let change = ctx
            .db
            .delete_room_file(
                store,
                &path,
                self.if_version,
                RoomFileAuthor {
                    member_id: Some(member.id),
                    session_id: None,
                },
            )
            .await?;
        written(ctx, &room, change, &path).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRoomFileHistory {
    #[param(ignore)]
    pub room_id: String,
    /// The file, as it is now or was when deleted.
    pub path: String,
    pub limit: Option<i64>,
}

#[command(
    name = "list_room_file_history",
    summary = "Read a room file's history",
    category = "rooms",
    description = "List every version of a room file, newest first: who changed it, how, and when. Follows the file across moves.",
    method = "GET",
    path = "/v1/rooms/{room_id}/files/history",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "history").with_args(&[CliArg::new("room_id").at(1)]).with_examples(&[CliExample::new("See who changed a room's plan", "everruns rooms files history room_01h9 --path /plans/launch.md")]),
    positional = "room_id",
    http = plain,
    params(ListRoomFileHistory),
)]
impl Command for ListRoomFileHistory {
    type Output = RoomFileHistoryResponse;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileHistoryResponse, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let store = store(ctx, &room).await?;
        let (file_id, _) = ctx
            .db
            .find_room_file(store, &path)
            .await?
            .ok_or_else(|| CommandError::not_found_msg(format!("No file at {path}")))?;
        let limit = self.limit.unwrap_or(50).clamp(1, 500);
        let members = authors(ctx, &room).await?;
        let data = ctx
            .db
            .list_room_file_versions(room.id, file_id, limit)
            .await?
            .iter()
            .map(|row| version_out(&members, row))
            .collect();
        Ok(RoomFileHistoryResponse { data })
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RestoreRoomFile {
    pub room_id: String,
    /// The file, as it is now or was when deleted.
    pub path: String,
    /// The version to bring back. It becomes a new version; nothing is lost.
    pub version: i32,
    /// Restore as this member (org admins).
    pub as_member: Option<String>,
}

#[command(
    name = "restore_room_file",
    summary = "Restore an earlier version of a room file",
    category = "rooms",
    description = "Bring back an earlier version of a room file, also after it was deleted. The restore is a new version, so nothing is lost.",
    method = "POST",
    path = "/v1/rooms/{room_id}/files/restore",
    policy = super::ROOM_ACCESS,
    cli = CliRoute::new(&["rooms", "files"], "restore").with_examples(&[CliExample::new("Undo a bad change to a room's plan", "everruns rooms files restore --room-id room_01h9 --path /plans/launch.md --version 2")]),
    http = plain,
)]
impl Command for RestoreRoomFile {
    type Output = RoomFileVersion;

    async fn execute(self, ctx: &Ctx) -> Result<RoomFileVersion, CommandError> {
        let room = q::load_room(ctx, &self.room_id).await?;
        let path = clean_path(&self.path)?;
        let member = writer(ctx, &room, self.as_member.as_deref()).await?;
        let store = store(ctx, &room).await?;
        let not_found = || CommandError::not_found_msg(format!("No file at {path}"));
        let (file_id, latest) = ctx
            .db
            .find_room_file(store, &path)
            .await?
            .ok_or_else(not_found)?;
        let latest = latest.ok_or_else(not_found)?;
        let (old, content) = ctx
            .db
            .get_room_file_version(room.id, file_id, self.version)
            .await?
            .ok_or_else(|| {
                CommandError::not_found_msg(format!("{path} has no version {}", self.version))
            })?;
        if old.action == "delete" {
            return Err(CommandError::bad_request(format!(
                "Version {} is the delete; restore the one before it",
                self.version
            )));
        }
        let deleted = latest.action == "delete";
        let change = ctx
            .db
            .write_room_file(
                store,
                // A live file is restored where it is now; a deleted one
                // where it was when it was deleted.
                &latest.path,
                &content.unwrap_or_default(),
                Some(if deleted { 0 } else { latest.version }),
                "restore",
                deleted.then_some(file_id),
                RoomFileAuthor {
                    member_id: Some(member.id),
                    session_id: None,
                },
            )
            .await?;
        written(ctx, &room, change, &path).await
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn edit(old: &str, new: &str) -> RoomFileEdit {
        RoomFileEdit {
            old_text: old.into(),
            new_text: new.into(),
        }
    }

    #[test]
    fn edits_replace_text_that_occurs_once() {
        let text = "- [ ] Draft\n- [ ] Review\n";
        assert_eq!(
            apply_edits(text, &[edit("- [ ] Draft", "- [x] Draft")]).unwrap(),
            "- [x] Draft\n- [ ] Review\n"
        );
    }

    #[test]
    fn an_edit_whose_text_is_missing_or_repeated_changes_nothing() {
        let text = "- [ ] a\n- [ ] a\n";
        assert!(
            apply_edits(text, &[edit("- [ ] a", "x")])
                .unwrap_err()
                .contains("2 times")
        );
        assert!(
            apply_edits(text, &[edit("nope", "x")])
                .unwrap_err()
                .contains("not in the file")
        );
        assert!(apply_edits(text, &[]).is_err());
    }

    #[test]
    fn binary_content_reads_back_as_base64() {
        assert_eq!(encode(b"hi".to_vec()).0, "text");
        assert_eq!(encode(vec![0xff, 0xfe]).0, "base64");
    }
}
