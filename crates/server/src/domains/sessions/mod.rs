// Sessions domain — commands, queries, and types.

pub mod commands;
pub mod limits;
#[cfg(test)]
mod list_filters_tests;
pub(crate) mod platform_chat_starter;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

pub(crate) async fn platform_chat_owner_matches_session(
    db: &crate::storage::StorageBackend,
    caller: &everruns_core::Caller,
    session: &everruns_platform::Session,
) -> anyhow::Result<bool> {
    let harness = db
        .get_harness(caller.org_id, session.harness_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session harness not found"))?;
    Ok(platform_chat_owner_matches(
        caller,
        session,
        harness.is_built_in && harness.name == "platform-chat",
    ))
}

pub(crate) fn platform_chat_owner_matches(
    caller: &everruns_core::Caller,
    session: &everruns_platform::Session,
    is_platform_chat: bool,
) -> bool {
    // THREAT[TM-AGENT-017]: Platform Chat can act with its persisted owner's authority,
    // and context-aware commands can read private history. Every user-driven surface
    // must bind to that same owner; internal worker paths retain their existing bypass.
    !is_platform_chat || caller.is_internal || caller.user_id == session.resolved_owner_user_id
}
