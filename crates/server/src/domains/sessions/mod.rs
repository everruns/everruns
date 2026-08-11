// Sessions domain — commands, queries, and types.

pub mod commands;
pub mod limits;
#[cfg(test)]
mod list_filters_tests;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

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
