// Sessions domain — commands, queries, and types.

pub mod commands;
mod ensure_platform_chat;
pub mod record;
pub use ensure_platform_chat::EnsurePlatformChat;
pub mod limits;
#[cfg(test)]
mod list_filters_tests;
pub(crate) mod platform_chat_starter;
#[cfg(test)]
mod platform_chat_tests;
pub(crate) mod playground;
#[cfg(test)]
mod playground_tests;
pub mod queries;
pub mod service;
pub mod types;
mod validation;

pub use commands::*;
pub use service::*;

pub(crate) async fn platform_chat_owner_matches_session(
    db: &crate::storage::StorageBackend,
    caller: &everruns_core::Caller,
    session: &crate::domains::sessions::record::Session,
) -> anyhow::Result<bool> {
    let agent = match session.agent_id {
        Some(id) => {
            db.get_agent_by_public_id(caller.org_id, &id.to_string())
                .await?
        }
        None => None,
    };

    Ok(platform_chat_owner_matches(
        caller,
        session,
        agent.is_some_and(|a| a.is_built_in && a.name == crate::platform_chat_agent::NAME),
    ))
}

pub(crate) fn platform_chat_owner_matches(
    caller: &everruns_core::Caller,
    session: &crate::domains::sessions::record::Session,
    is_platform_chat: bool,
) -> bool {
    // THREAT[TM-AGENT-017]: Platform Chat can act with its persisted owner's authority,
    // and context-aware commands can read private history. Every user-driven surface
    // must bind to that same owner; internal worker paths retain their existing bypass.
    !is_platform_chat || caller.is_internal || caller.user_id == session.resolved_owner_user_id
}
