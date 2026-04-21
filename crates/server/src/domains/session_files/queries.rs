use crate::domains::common::{CommandError, Ctx};
use crate::services::session_file::SessionFileService;
use everruns_core::typed_id::SessionId;
use std::sync::Arc;

const WORKSPACE_PREFIX: &str = "/workspace";

pub fn service(ctx: &Ctx) -> Arc<SessionFileService> {
    ctx.session_file_service
        .clone()
        .unwrap_or_else(|| Arc::new(SessionFileService::new(ctx.db.clone())))
}

pub fn parse_session_id(session_id: &str) -> Result<SessionId, CommandError> {
    session_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

pub async fn verify_session(ctx: &Ctx, session_id: SessionId) -> Result<(), CommandError> {
    ctx.db
        .get_session(ctx.org_id(), session_id)
        .await
        .map_err(classify_storage)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    Ok(())
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
    CommandError::Internal(error)
}
