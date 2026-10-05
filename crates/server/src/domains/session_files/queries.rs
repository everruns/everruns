use crate::domains::common::{CommandError, Ctx};
use crate::domains::session_files::WorkspaceFileService;
use everruns_contracts::typed_id::SessionId;
use std::sync::Arc;
use uuid::Uuid;

pub const USER_MEMORY_MOUNT_PATH: &str = "/memory/user";

#[derive(Debug, Clone, Copy)]
pub struct SessionFileAccess {
    pub workspace_key: Uuid,
    pub user_memory_allowed: bool,
}

impl SessionFileAccess {
    pub fn ensure_user_memory_access(&self, path: &str) -> Result<(), CommandError> {
        if !self.user_memory_allowed && is_user_memory_path(path) {
            return Err(user_memory_forbidden());
        }
        Ok(())
    }

    pub fn ensure_user_memory_mutation_access(&self, path: &str) -> Result<(), CommandError> {
        if !self.user_memory_allowed && is_user_memory_path_or_ancestor(path) {
            return Err(user_memory_forbidden());
        }
        Ok(())
    }
}

pub fn service(ctx: &Ctx) -> Arc<WorkspaceFileService> {
    ctx.session_file_service
        .clone()
        .unwrap_or_else(|| Arc::new(WorkspaceFileService::new(ctx.db.clone())))
}

/// The file-store key for a session, resolved without a [`Ctx`].
///
/// Trusted internal orchestration — the worker gRPC surface — addresses sessions
/// by id alone and has no `Caller` to build a `Ctx` from, but it must key files
/// the same way [`verify_session`] does: by the workspace the session is attached
/// to, never by the session uuid. The two coincide for the default 1:1 session,
/// which is why keying by the uuid looks correct until a session is attached to a
/// shared workspace.
pub async fn workspace_key_unscoped(
    db: &crate::storage::StorageBackend,
    session_id: SessionId,
) -> anyhow::Result<Option<Uuid>> {
    Ok(db
        .get_session_unscoped(session_id)
        .await?
        .map(|row| row.workspace_id))
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
pub async fn verify_session(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<SessionFileAccess, CommandError> {
    let row = ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await
        .map_err(classify_storage)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    // THREAT[TM-TENANT-013]: `/memory/user` is private to the session's owner.
    // Two callers reach it: that owner, and the agent runtime executing that
    // owner's session on their behalf.
    //
    // The runtime arm used to read `caller.is_internal`, which was far wider
    // than the relationship it stood for. `Caller::internal` carries Owner
    // role and bypasses policy evaluation (TM-AUTHZ-002), so this was the one
    // check it did not already pass — the only thing between *any* internal
    // path and a person's private files, including paths driven by inbound
    // traffic (`api/slack_events`, `api/fcp`, the capability service, the
    // durable seal). None of them read the file surface at all.
    //
    // `acting_for_session` names the relationship instead: the worker declares
    // which session's turn it is running, and only that session's memory
    // opens. An internal caller that declares nothing gets nothing.
    let user_memory_allowed = caller_is_memory_owner(row.resolved_owner_user_id, &ctx.caller)
        || ctx.acting_for_session == Some(session_id);
    Ok(SessionFileAccess {
        workspace_key: row.workspace_id,
        user_memory_allowed,
    })
}

/// Whether `caller` *is* the owner of the `/memory/user` subtree owned by
/// `owner`. Org role is deliberately irrelevant: an admin with
/// `WORKSPACE_MANAGE` is still not the owner of another member's private
/// memory.
///
/// Owner identity only. The internal-path arm this used to carry is not an
/// ownership fact, and conflating the two is what let every internal caller
/// into the subtree; each surface now states its own trust rule beside its own
/// call — the session surface through `Ctx::acting_for_session`, the workspace
/// surface through its explicit early return.
pub fn caller_is_memory_owner(owner: Option<Uuid>, caller: &everruns_core::Caller) -> bool {
    owner
        .zip(caller.user_id)
        .is_some_and(|(owner, caller)| owner == caller)
}

/// `/memory/user` access for the canonical `/v1/workspaces/{id}/fs/*` surface,
/// which is keyed by workspace rather than by session.
///
// THREAT[TM-TENANT-013]: `MemoryMountRouter::resolve` mounts `/memory/user`
// from the session whose id equals the workspace key (the default 1:1
// session), so that session's `resolved_owner_user_id` is the owner here too.
// A workspace with no such session (a shared workspace) has no private owner
// and only internal callers may touch the subtree. The caller has already
// passed the workspace org/policy check; this adds the owner check the
// workspace policy cannot express.
pub async fn user_memory_allowed_for_workspace(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    workspace_key: Uuid,
    caller: &everruns_core::Caller,
) -> anyhow::Result<bool> {
    if caller.is_internal {
        return Ok(true);
    }
    let owner = db
        .get_session(org_id, SessionId::from_uuid(workspace_key))
        .await?
        .and_then(|row| row.resolved_owner_user_id);
    Ok(caller_is_memory_owner(owner, caller))
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
/// * Archived (non-`active`) workspaces are read-only,
///   so writes are rejected for default and shared workspaces alike. A deleted
///   workspace is filtered at the storage layer and treated as gone (404);
///   an archived one still exists but is read-only (403).
pub async fn verify_session_for_write(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<Uuid, CommandError> {
    let access = verify_session(ctx, session_id).await?;
    let workspace_key = access.workspace_key;

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

/// Normalize a control-plane FS path to its canonical session-store key.
///
/// Delegates to the single cross-surface normalizer (EVE-670) so the HTTP FS API
/// and the agent resolve a given path to the same key: collapse repeated
/// slashes, strip the `/workspace` alias, leading slash, no trailing slash.
pub fn normalize_path(path: &str) -> String {
    everruns_core::session_path::to_session_path(path)
}

pub fn is_reserved_path(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.starts_with('_') || path.split('/').any(|segment| segment.starts_with('_'))
}

pub fn is_user_memory_path(path: &str) -> bool {
    let path = normalize_path(path);
    path == USER_MEMORY_MOUNT_PATH
        || path
            .strip_prefix(USER_MEMORY_MOUNT_PATH)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn is_user_memory_path_or_ancestor(path: &str) -> bool {
    let path = normalize_path(path);
    is_user_memory_path(&path)
        || USER_MEMORY_MOUNT_PATH
            .strip_prefix(path.trim_end_matches('/'))
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn redact_user_memory_files<T>(files: Vec<T>, path: impl Fn(&T) -> &str) -> Vec<T> {
    files
        .into_iter()
        .filter(|file| !is_user_memory_path(path(file)))
        .collect()
}

fn user_memory_forbidden() -> CommandError {
    CommandError::forbidden("User memory is private to the session owner")
}

pub fn classify_storage(error: anyhow::Error) -> CommandError {
    tracing::error!("Session file domain error: {error}");
    CommandError::internal(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EVE-670: the control-plane FS API normalizes to the same canonical key as
    /// the agent (strips the `/workspace` alias, collapses repeated slashes,
    /// trims the trailing slash), so a file resolves identically regardless of
    /// entry point. Full behavior is covered in `everruns_core::session_path`.
    #[test]
    fn normalize_path_matches_canonical_session_path() {
        assert_eq!(normalize_path("/workspace/src/lib.rs"), "/src/lib.rs");
        assert_eq!(normalize_path("src/lib.rs"), "/src/lib.rs");
        assert_eq!(normalize_path("/workspace"), "/");
        assert_eq!(normalize_path("/a//b/"), "/a/b");
        // A real top-level `workspace/` dir is not the alias.
        assert_eq!(normalize_path("/workspacefoo"), "/workspacefoo");
    }

    #[test]
    fn detects_user_memory_paths_after_normalization() {
        assert!(is_user_memory_path("/memory/user"));
        assert!(is_user_memory_path("memory/user/notes.md"));
        assert!(is_user_memory_path("/workspace/memory/user/notes.md"));
        assert!(!is_user_memory_path("/memory/userland"));
        assert!(!is_user_memory_path("/memory/agent"));
        assert!(is_user_memory_path_or_ancestor("/"));
        assert!(is_user_memory_path_or_ancestor("/memory"));
        assert!(is_user_memory_path_or_ancestor("/memory/user/notes.md"));
        assert!(!is_user_memory_path_or_ancestor("/notes.md"));
    }
}
