use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::storage::StorageBackend;
use anyhow::Context;
use everruns_platform::Session;
use everruns_provider::typed_id::HarnessId;
use everruns_scale::RunController;
use std::sync::Arc;

pub fn session_service(
    ctx: &Ctx,
) -> Result<Arc<crate::domains::sessions::SessionService>, CommandError> {
    ctx.session_service
        .clone()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Session service not configured")))
}

pub fn runner(ctx: &Ctx) -> Result<Arc<dyn RunController>, CommandError> {
    ctx.runner
        .clone()
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("Agent runner not configured")))
}

pub fn parse_session_id(
    session_id: &str,
) -> Result<everruns_provider::typed_id::SessionId, CommandError> {
    session_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// Resolve the harness for a new session using the agent-first precedence (D4):
/// explicit request harness → the agent's harness → org default → built-in
/// fallback. `agent_harness_id` is `None` for no-agent sessions, in which case
/// behavior is unchanged from harness-first creation.
pub async fn resolve_session_harness_id(
    db: &StorageBackend,
    org_id: i64,
    requested_harness_id: Option<HarnessId>,
    agent_harness_id: Option<HarnessId>,
    fallback_harness_name: Option<&str>,
) -> anyhow::Result<HarnessId> {
    if let Some(harness_id) = requested_harness_id {
        return Ok(harness_id);
    }

    if let Some(harness_id) = agent_harness_id {
        return Ok(harness_id);
    }

    let settings = db.get_organization_settings(org_id).await?;
    if let Some(harness_id) = settings.and_then(|row| row.default_harness_id) {
        return Ok(harness_id);
    }

    let fallback_name = fallback_harness_name.context(
        "Session creation requires a default harness but no fallback harness is configured",
    )?;
    let harnesses = db
        .list_harnesses(org_id, Some(fallback_name), false)
        .await?;
    harnesses
        .into_iter()
        .find(|h| h.is_built_in && h.name == fallback_name)
        .map(|h| h.id)
        .context(format!(
            "Built-in harness '{fallback_name}' is not provisioned for org {org_id}"
        ))
}

pub async fn resolve_named_built_in_harness_id(
    db: &StorageBackend,
    org_id: i64,
    harness_name: &str,
) -> anyhow::Result<HarnessId> {
    let harnesses = db.list_harnesses(org_id, Some(harness_name), false).await?;
    harnesses
        .into_iter()
        .find(|h| h.is_built_in && h.name == harness_name)
        .map(|h| h.id)
        .context(format!(
            "Built-in harness '{harness_name}' is not provisioned for org {org_id}"
        ))
}

pub async fn get_session(
    ctx: &Ctx,
    session_id: everruns_provider::typed_id::SessionId,
    user_id: Option<uuid::Uuid>,
) -> Result<Session, CommandError> {
    session_service(ctx)?
        .get(&ctx.caller, session_id.uuid(), user_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))
}
