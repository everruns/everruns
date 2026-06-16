use crate::domains::common::{CommandError, Ctx};
use crate::domains::session_files::WorkspaceFileService;
use everruns_core::typed_id::SessionId;
use std::sync::Arc;
use uuid::Uuid;

const WORKSPACE_PREFIX: &str = "/workspace";

pub fn service(ctx: &Ctx) -> Arc<WorkspaceFileService> {
    ctx.session_file_service
        .clone()
        .unwrap_or_else(|| Arc::new(WorkspaceFileService::new(ctx.db.clone())))
}

pub fn parse_session_id(session_id: &str) -> Result<SessionId, CommandError> {
    session_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// Verify the session exists in the caller's org and return the internal id of
/// the workspace it is attached to — i.e. the file-store key. For the default
/// 1:1 session this equals the session uuid; for a shared workspace it differs,
/// so file operations must key by this value rather than `session_id.uuid()`.
pub async fn verify_session(ctx: &Ctx, session_id: SessionId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await
        .map_err(classify_storage)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    Ok(row.workspace_id)
}

/// Resolve the session's workspace for a *write* and enforce the workspace
/// write boundary, matching the canonical `/v1/workspaces/{workspace_id}/fs/*`
/// behavior:
///
/// * Shared workspaces (where the resolved key differs from the session uuid)
///   require `WORKSPACE_MANAGE`, not just the session policy. Without this a
///   caller holding only `SESSION_MANAGE` could mutate an attached shared
///   workspace through the legacy session-fs alias, bypassing the workspace
///   authorization boundary.
/// * Archived (non-`active`) workspaces are read-only per `specs/workspace.md`,
///   so writes are rejected for default and shared workspaces alike. A deleted
///   workspace is filtered at the storage layer and treated as gone (404);
///   an archived one still exists but is read-only (403).
pub async fn verify_session_for_write(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<Uuid, CommandError> {
    let workspace_key = verify_session(ctx, session_id).await?;

    if workspace_key != session_id.uuid() {
        crate::domains::workspaces::WORKSPACE_MANAGE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| CommandError::forbidden(e.message))?;
    }

    let workspace = ctx
        .db
        .get_workspace_by_id(ctx.org_id(), workspace_key)
        .await
        .map_err(classify_storage)?
        .ok_or_else(|| CommandError::not_found("Workspace"))?;
    if workspace.status != "active" {
        return Err(CommandError::forbidden(format!(
            "Workspace is {} and cannot be modified",
            workspace.status
        )));
    }

    Ok(workspace_key)
}

pub fn normalize_path(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return "/".to_string();
    }

    let abs_path = format!("/{}", path);

    if abs_path == WORKSPACE_PREFIX {
        "/".to_string()
    } else if let Some(stripped) = abs_path.strip_prefix(WORKSPACE_PREFIX) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            abs_path
        }
    } else {
        abs_path
    }
}

pub fn is_reserved_path(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.starts_with('_') || path.split('/').any(|segment| segment.starts_with('_'))
}

pub fn classify_storage(error: anyhow::Error) -> CommandError {
    tracing::error!("Session file domain error: {error}");
    CommandError::internal(error)
}
