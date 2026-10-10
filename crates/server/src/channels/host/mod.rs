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

pub use port::ServerChannelPort;
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
        let resolver = ServerChannelResolver::new(db.clone(), encryption);
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

use everruns_contracts::typed_id::PrincipalId;

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
    /// The HTTP request, for log correlation of the turn it starts.
    pub request_id: Option<String>,
}

impl ServerChannelScope {
    pub fn new(context: IngressContext, channel: IngressChannel) -> Self {
        Self {
            context,
            channel,
            runtime_owner: None,
            request_id: None,
        }
    }
}

#[cfg(test)]
mod tests;
