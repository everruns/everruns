//! A v2 session's Memory mounts (data plane v2 parity, Phase B: Console
//! Chat and every agent with Memory).
//!
//! Decisions:
//! - Memory stays in the control plane. Its files are shared across
//!   sessions, people and agents of the org (`/memory/shared` is read by
//!   every Platform Chat thread, `/memory/agent` by every session of one
//!   agent), so no one session's shard can own them. A shard reaches them
//!   through the Memory files server (`injected/hosted.rs`), which runs
//!   [`answer`] per call.
//! - Existing Memory mounts are resolved here from its record, as for a
//!   v1 session (`session_files::memory_mounts`), and sent with the create
//!   ([`mounts`]). The reserved Application root routes late-bound resources;
//!   it confers no access without a current binding.
//! - A call names the session and a path; it is served only when the path
//!   is inside one of that session's own mounts, resolved again from the
//!   record. A shard cannot reach another session's Memory, nor the
//!   session's workspace (which on v2 lives in the shard anyway).
//! - The answer carries the file service's own paths (`/memory/user/...`);
//!   the shard maps them into its mount.
//! - No content search over Memory yet: the file service searches a whole
//!   workspace, not one mount. A shell's `grep` reads the files instead.

use crate::domains::data_plane::calls::MEMORY_FILE;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_files::SessionFileSystem;
use serde_json::{Value, json};

use super::ControlPlane;
use crate::domains::common::Ctx;
use crate::domains::session_files::{MemoryMountRouter, WorkspaceFileService};

/// The Memory mount points `session` has (`/memory/agent`,
/// `/memory/user`, `/memory/shared`, `/team`, `/room`), from its record.
pub async fn mounts(cp: &ControlPlane, session: SessionId) -> anyhow::Result<Vec<String>> {
    let router = MemoryMountRouter::new(cp.db.clone());
    let mut mounts: Vec<String> = router
        .mounts_for(session.uuid())
        .await
        .iter()
        .filter(|mount| !mount.mount_path.starts_with("/application/"))
        .map(|mount| mount.mount_path.clone())
        .collect();
    // Channel and Room bindings are committed after session creation. Keep a
    // stable routing mount so later bindings, renamed resources and new grants
    // reach the control plane. This is routing only: answer rechecks authority.
    mounts.push("/application/memory".to_string());
    if cp
        .db
        .get_session_unscoped(session)
        .await?
        .is_some_and(|row| row.source == "room")
        && !mounts.iter().any(|path| path == "/room")
    {
        mounts.push("/room".to_string());
    }
    Ok(mounts)
}

/// Whether `path` is `mount` or below it.
fn under(path: &str, mount: &str) -> bool {
    path == mount
        || path
            .strip_prefix(mount)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Answer a [`MEMORY_FILE`] call: `{"ok": ...}`, or `{"error": message}`
/// when the operation failed (a missing file is `{"ok": null}`).
pub async fn answer(ctx: &Ctx, params: &Value) -> Value {
    match run(ctx, params).await {
        Ok(value) => json!({ "ok": value }),
        Err(message) => json!({ "error": message }),
    }
}

async fn run(ctx: &Ctx, params: &Value) -> Result<Value, String> {
    let session = params["session_id"]
        .as_str()
        .and_then(|id| SessionId::parse(id).ok())
        .ok_or("invalid session_id")?;
    ctx.db
        .get_session(ctx.org_id(), session)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("session not found")?;
    // The route's context carries no file service; Memory paths need only
    // the database (`session_files::memory_mounts`).
    let files = ctx
        .session_file_service
        .clone()
        .unwrap_or_else(|| std::sync::Arc::new(WorkspaceFileService::new(ctx.db.clone())));
    let path = params["path"].as_str().ok_or("path is required")?;
    let router = MemoryMountRouter::new(ctx.db.clone());
    let mounts = router.mounts_for(session.uuid()).await;
    // THREAT[TM-TENANT]: only the session's own Memory, resolved from its
    // record, whatever the shard names.
    let application_directory = path == "/application/memory"
        && matches!(params["op"].as_str(), Some("list" | "stat"))
        && mounts.iter().any(|mount| under(&mount.mount_path, path));
    if !application_directory && !mounts.iter().any(|mount| under(path, &mount.mount_path)) {
        return Err(format!("{path} is not in this session's Memory"));
    }
    let text = |key: &str| params[key].as_str().unwrap_or_default();
    let encoding = |key: &str| params[key].as_str().unwrap_or("text");
    let to_value = |result: serde_json::Result<Value>| result.map_err(|e| e.to_string());
    let failed = |e: everruns_contracts::error::AgentLoopError| e.to_string();
    match params["op"].as_str().unwrap_or_default() {
        "read" => to_value(serde_json::to_value(
            files.read_file(session, path).await.map_err(failed)?,
        )),
        "write" => to_value(serde_json::to_value(
            files
                .write_file(session, path, text("content"), encoding("encoding"))
                .await
                .map_err(failed)?,
        )),
        "write_if_matches" => to_value(serde_json::to_value(
            files
                .write_file_if_content_matches(
                    session,
                    path,
                    text("expected_content"),
                    encoding("expected_encoding"),
                    text("content"),
                    encoding("encoding"),
                )
                .await
                .map_err(failed)?,
        )),
        "create_if_absent" => to_value(serde_json::to_value(
            files
                .create_file_if_absent(session, path, text("content"))
                .await
                .map_err(failed)?,
        )),
        "delete" => to_value(serde_json::to_value(
            files
                .delete_file(
                    session,
                    path,
                    params["recursive"].as_bool().unwrap_or(false),
                )
                .await
                .map_err(failed)?,
        )),
        "list" => to_value(serde_json::to_value(
            files.list_directory(session, path).await.map_err(failed)?,
        )),
        "stat" => to_value(serde_json::to_value(
            files.stat_file(session, path).await.map_err(failed)?,
        )),
        "mkdir" => to_value(serde_json::to_value(
            files
                .create_directory(session, path)
                .await
                .map_err(failed)?,
        )),
        other => Err(format!("unknown {MEMORY_FILE} operation {other:?}")),
    }
}
