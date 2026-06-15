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

/// Verify a session-scoped filesystem write and return the file-store key.
///
/// Default 1:1 sessions keep the historical SESSION_MANAGE authorization. When
/// the alias targets a shared workspace, preserve the workspace write boundary
/// used by `/v1/workspaces/{workspace_id}/fs/*`.
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
