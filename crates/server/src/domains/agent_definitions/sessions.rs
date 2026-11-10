//! What a leased session runs on the Engine: its binding, claimed under the
//! actor lease (data plane v2 step 4). The gRPC and in-process worker links
//! both answer `ClaimEngineSession` with [`claim`].
//!
//! Decisions:
//! - An unbound session of an organization enrolled in Engine hosting binds
//!   at its first claim between turns, to the key of its harness and agent.
//!   Its log is already the journal the Engine reads, so nothing is imported:
//!   the next turn simply runs on the Engine.
//! - The live part of a session (its model and session capabilities) is the
//!   Engine's session configuration, read from the session row at each
//!   claim, so a model switch applies from the next turn. A session that
//!   carries overrides the Engine cannot express yet (its own system prompt,
//!   client tools, MCP servers, initial files, network access, iteration
//!   cap) stays on the worker's own host; binding it would silently drop
//!   them. Data plane v2 open question 3 decides whether they become live
//!   controls or retire. So does a session whose agent does not compile.

use everruns::hosting::SESSION_RUNTIME_VERSION;
use everruns::hosting::spi::session_commands::SessionConfiguration;
use everruns_contracts::CapabilityRef;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::typed_id::SessionId;
use serde_json::Value;

use crate::storage::{EngineBindingRow, SessionRow, StorageBackend};

/// The answer to a claim.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineClaim {
    /// The lease is not the caller's any more: stop without writing.
    Lost,
    /// Run this lease on the worker's own host.
    Legacy,
    /// Run it on the Engine.
    Bound(Box<EngineSession>),
}

/// A claimed session's Engine binding and live configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineSession {
    pub binding: EngineBindingRow,
    /// The Engine's `SessionConfiguration` as JSON.
    pub configuration: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Claim `session_id`'s Engine binding for the lease `holder` holds at
/// `fence`. See the module notes.
pub async fn claim(
    db: &StorageBackend,
    session_id: SessionId,
    holder: &str,
    fence: i64,
) -> anyhow::Result<EngineClaim> {
    let Some(actor) = db.get_session_actor(session_id).await? else {
        return Ok(EngineClaim::Lost);
    };
    if actor.lease_holder.as_deref() != Some(holder) || actor.lease_fence != fence {
        return Ok(EngineClaim::Lost);
    }
    let Some(session) = db.get_session(actor.org_id, session_id).await? else {
        return Ok(EngineClaim::Lost);
    };
    let binding = match db.claim_engine_binding(session_id, holder, fence).await? {
        Some(binding) => binding,
        None => {
            let Some(harness_id) = session.harness_id else {
                return Ok(EngineClaim::Legacy);
            };
            if !enrolled(db, actor.org_id).await? || overrides(&session).is_some() {
                return Ok(EngineClaim::Legacy);
            }
            // Decision: a session whose agent does not compile to a
            // declaration (an empty prompt, a capability the Engine cannot
            // name) keeps running where it runs today.
            let (key, revision) =
                match super::key_for(db, actor.org_id, harness_id, session.agent_id).await {
                    Ok(published) => published,
                    Err(error) => {
                        tracing::warn!(%error, %session_id, "session stays on the worker host");
                        return Ok(EngineClaim::Legacy);
                    }
                };
            match db
                .bind_engine_session(
                    session_id,
                    holder,
                    fence,
                    &key,
                    &revision,
                    i32::try_from(SESSION_RUNTIME_VERSION).unwrap_or(i32::MAX),
                )
                .await?
            {
                Some(binding) => binding,
                // A turn is open (it finishes where it started) or the
                // lease went in between.
                None => return Ok(EngineClaim::Legacy),
            }
        }
    };
    Ok(EngineClaim::Bound(Box::new(EngineSession {
        configuration: serde_json::to_value(configuration(db, &session).await?)?,
        created_at: session.created_at,
        binding,
    })))
}

/// The organization enrolled in Engine hosting.
async fn enrolled(db: &StorageBackend, org_id: i64) -> anyhow::Result<bool> {
    Ok(db
        .list_org_feature_flags(org_id)
        .await?
        .get(crate::storage::ENGINE_HOSTING_FLAG)
        .copied()
        .unwrap_or(false))
}

/// The first session-level override the Engine cannot express, if any.
pub fn overrides(session: &SessionRow) -> Option<&'static str> {
    let set = |value: &Value| match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        _ => true,
    };
    if session
        .system_prompt
        .as_deref()
        .is_some_and(|p| !p.is_empty())
    {
        Some("system_prompt")
    } else if set(&session.tools) {
        Some("tools")
    } else if set(&session.mcp_servers) {
        Some("mcp_servers")
    } else if set(&session.initial_files) {
        Some("initial_files")
    } else if session.network_access.as_ref().is_some_and(set) {
        Some("network_access")
    } else if session.max_iterations.is_some() {
        Some("max_iterations")
    } else if session.parallel_tool_calls.is_some() {
        Some("parallel_tool_calls")
    } else {
        None
    }
}

/// The session's live configuration: its model and session capabilities.
async fn configuration(
    db: &StorageBackend,
    session: &SessionRow,
) -> anyhow::Result<SessionConfiguration> {
    let model = match session.model_id {
        Some(model_id) => db
            .get_model_with_provider(session.org_id, model_id.uuid())
            .await?
            .map(|row| ModelSpec::on(row.provider_name, row.model_id)),
        None => None,
    };
    let capabilities: Vec<CapabilityRef> = if session.capabilities.is_null() {
        Vec::new()
    } else {
        serde_json::from_value(session.capabilities.clone())?
    };
    Ok(SessionConfiguration {
        model: Some(model),
        capabilities: Some(capabilities),
        ..SessionConfiguration::default()
    })
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
