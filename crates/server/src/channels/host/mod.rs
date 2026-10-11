//! The server's channels on the shared channel host
//! (knowledge/integrations/channels.md, "Server on the host").
//!
//! Core's `ChannelHost` owns intake: dedup, the conversation's session,
//! sending, streaming and reply delivery. The server plugs in three things:
//! - [`PgChannelStore`]: bindings, delivery keys and leased pending
//!   deliveries in PostgreSQL;
//! - [`ServerChannelPort`]: sessions and messages through the server's own
//!   services, so tenancy, owners, budgets and audit stay where they are;
//! - [`ServerChannelResolver`]: a channel row, as the host's channel.
//!
//! Decision: a server channel's host name is its row's internal id. Public ids
//! are what callers send; the HTTP layer resolves those, checks the caller,
//! and hands the host the resolved channel, so the host never sees a name it
//! could confuse across tenants.

mod port;
mod resolver;
mod store;
mod webhook;

pub use port::{HISTORY_NOTICE, ServerChannelPort, THREAD_VIEW_CONTEXT};
pub use resolver::ServerChannelResolver;
pub use store::PgChannelStore;
pub use webhook::{WEBHOOK_PLATFORM, WebhookDriver};

use std::sync::Arc;

use everruns_core::channel_runtime::ChannelHost;

use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::storage::{EncryptionService, StorageBackend};

/// The host and the resolver the server's channel routes share.
#[derive(Clone)]
pub struct ServerChannels {
    pub host: ChannelHost,
    pub resolver: ServerChannelResolver,
}

impl ServerChannels {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        sessions: Arc<SessionService>,
        messages: Arc<MessageService>,
    ) -> Self {
        Self::with_resolver(
            ServerChannelResolver::new(db.clone(), encryption),
            db,
            sessions,
            messages,
        )
    }

    /// Channels resolved by `resolver`: one that also runs Slack.
    pub fn with_resolver(
        resolver: ServerChannelResolver,
        db: Arc<StorageBackend>,
        sessions: Arc<SessionService>,
        messages: Arc<MessageService>,
    ) -> Self {
        let host = ChannelHost::builder(Arc::new(ServerChannelPort::new(
            db.clone(),
            sessions,
            messages,
        )))
        .store(Arc::new(PgChannelStore::new(db)))
        .resolver(Arc::new(resolver.clone()))
        .build();
        Self { host, resolver }
    }
}

use std::collections::HashMap;

use everruns_contracts::typed_id::PrincipalId;
use serde_json::Value;

use crate::domains::agent_channels::ingress::{IngressChannel, IngressContext};

/// What a server channel's host scope carries: the channel row and its
/// tenant, agent and owner, as ingress resolved them, and what the request
/// adds.
#[derive(Debug, Clone)]
pub struct ServerChannelScope {
    pub context: IngressContext,
    pub channel: IngressChannel,
    /// The identified end user the request runs as (an AG-UI or Public Chat
    /// visitor's virtual-user principal). Their sessions are theirs, and
    /// their turns resolve their grants. `None` runs as the channel's owner.
    pub runtime_owner: Option<PrincipalId>,
    /// The end user a turn runs as without owning the session (a PACT
    /// delegation's company user). Falls back to `runtime_owner`.
    pub runtime_subject: Option<PrincipalId>,
    /// Tags added to the sessions the request creates (a PACT caller's tag,
    /// which its later reads are confined by).
    pub session_tags: Vec<String>,
    /// The HTTP request, for log correlation of the turn it starts.
    pub request_id: Option<String>,
    /// Client hints the channel's sessions declare (Slack's approval cards).
    pub session_hints: Option<HashMap<String, Value>>,
    /// Set for a channel whose speakers join its sessions: each sender becomes
    /// a session participant the turn runs as, and is kept on the session's
    /// thread context. Names the message metadata key holding the platform
    /// thread.
    pub participant_thread_key: Option<&'static str>,
}

impl ServerChannelScope {
    pub fn new(context: IngressContext, channel: IngressChannel) -> Self {
        Self {
            context,
            channel,
            runtime_owner: None,
            runtime_subject: None,
            session_tags: Vec::new(),
            request_id: None,
            session_hints: None,
            participant_thread_key: None,
        }
    }
}

/// A channel host failure as the server's command error. The session port
/// hands back the services' own errors, so a budget refusal or a missing
/// session keeps its kind.
pub fn channel_command_error(
    error: everruns_contracts::runtime::channel::ChannelError,
) -> crate::domains::common::CommandError {
    use crate::domains::common::CommandError;
    use everruns_contracts::runtime::channel::ChannelError;
    match error {
        ChannelError::BadRequest(message) => CommandError::bad_request(message),
        ChannelError::NotFound(what) => CommandError::not_found(&what),
        ChannelError::Unauthorized(message) => CommandError::forbidden(message),
        ChannelError::Session(message) => CommandError::internal(anyhow::anyhow!(message)),
        ChannelError::Other(error) => error.into(),
    }
}

#[cfg(test)]
mod tests;
