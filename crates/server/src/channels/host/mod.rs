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

pub use port::ServerChannelPort;
pub use resolver::ServerChannelResolver;
pub use store::PgChannelStore;

use crate::domains::agent_channels::ingress::{IngressChannel, IngressContext};

/// What a server channel's host scope carries: the channel row and its
/// tenant, agent and owner, as ingress resolved them.
#[derive(Debug, Clone)]
pub struct ServerChannelScope {
    pub context: IngressContext,
    pub channel: IngressChannel,
}

#[cfg(test)]
mod tests;
