//! Connection check for OAuth catalog presets.
//!
//! Decision: when an admin adds an OAuth preset (or switches one to OAuth),
//! the server runs the same discovery and dynamic client registration the
//! first sign-in would, and records the outcome on the preset
//! (`settings.connection_check`). A host the network policy blocks then shows
//! up on the catalog row when it is added, not halfway through somebody's
//! login. The check is best effort: saving never fails because of it.
//!
//! Decision: the work runs in a spawned task and the save waits for it only
//! up to [`CHECK_ON_SAVE_WAIT`]. A healthy server answers well inside that,
//! so the saved preset usually comes back with its result; a slow upstream
//! cannot hold the HTTP response open, and its task keeps going and records
//! the result when it finishes (`not_checked` until then). The commands that
//! run it are therefore not transactional: the task writes on its own
//! connection, so the preset must already be committed.
//!
//! Layering: discovery and registration live with the OAuth connect flow in
//! the API layer, which this domain may not import. The domain declares
//! [`McpOAuthConnectionChecker`]; the API layer implements it and hands it to
//! `Ctx`. Surfaces without one (worker-internal contexts) leave the preset
//! `not_checked`, and "Check again" runs it later.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::domains::common::{CommandError, Ctx};
use crate::domains::mcp_servers::record::McpConnectionCheck;

/// How long saving a preset waits for its check before answering.
pub const CHECK_ON_SAVE_WAIT: Duration = Duration::from_secs(4);

/// How long "Check again" waits. Longer: the admin asked for the answer.
pub const MANUAL_CHECK_WAIT: Duration = Duration::from_secs(20);

/// Runs discovery and registration for an OAuth preset and records the
/// outcome on it. Implemented in the API layer (`api::user_connections`).
#[async_trait]
pub trait McpOAuthConnectionChecker: Send + Sync {
    /// Check `server_id` in `org_id` and persist the result. Never fails:
    /// every failure becomes a recorded status. A preset that is missing,
    /// archived, or not OAuth returns `not_checked` and records nothing.
    async fn check_and_record(&self, org_id: i64, server_id: Uuid) -> McpConnectionCheck;
}

/// Start the check for a just-saved preset and wait for it at most `wait`.
/// Returns whether it finished in time. A check still running after `wait`
/// keeps running in the background and records its result itself.
pub async fn run_check(ctx: &Ctx, server_id: Uuid, wait: Duration) -> bool {
    let Some(checker) = ctx.mcp_oauth_checker.clone() else {
        return false;
    };
    let org_id = ctx.org_id();
    let task = tokio::spawn(async move { checker.check_and_record(org_id, server_id).await });
    matches!(tokio::time::timeout(wait, task).await, Ok(Ok(_)))
}

/// The checker for `ctx`, or a 503 for "Check again" on a surface without one.
pub fn require_checker(ctx: &Ctx) -> Result<Arc<dyn McpOAuthConnectionChecker>, CommandError> {
    ctx.mcp_oauth_checker
        .clone()
        .ok_or_else(|| CommandError::unavailable("MCP connection checks are not available here"))
}
