//! Channels: the doors through which people reach an agent.
//!
//! The wire types (inbound events, delivery adapters, bindings) come from
//! `everruns-contracts`. With the `channels` feature this module also holds the
//! one channel implementation the Framework, serve and the server share
//! (knowledge/integrations/channels.md):
//!
//! - [`ChannelDriver`]: one platform, request in, inbound message out, replies
//!   delivered through its [`ChannelDeliveryAdapter`].
//! - [`TurnDelivery`]: one turn's events in, platform calls out.
//! - [`ChannelHost`]: request in, response out; binds threads to sessions,
//!   drops duplicates, starts turns and runs their delivery.
//! - [`ChannelStore`]: what a host persists, with an in-memory default.
//!
//! Hosts plug in only a [`ChannelSessionPort`], a store and their HTTP stack.

pub use everruns_contracts::runtime::channel::*;
pub use everruns_contracts::runtime::message::ExternalActor;

#[cfg(feature = "channels")]
mod delivery;
#[cfg(feature = "channels")]
mod driver;
#[cfg(feature = "channels")]
mod host;
#[cfg(feature = "channels")]
mod store;
#[cfg(feature = "channels")]
pub mod webhook;

#[cfg(feature = "channels")]
pub use delivery::{
    DeliveryEvent, DeliveryOptions, DeliveryStep, STREAM_FLUSH_CHARS, TurnDelivery,
    is_terminal_turn_event, response_text,
};
#[cfg(feature = "channels")]
pub use driver::{
    ChannelDriver, ChannelError, ChannelRequest, ChannelResponse, Inbound, InboundMessage,
};
#[cfg(feature = "channels")]
pub use host::{
    ChannelConfig, ChannelEventStream, ChannelHost, ChannelHostBuilder, ChannelSessionPort,
    DEFAULT_FLUSH_INTERVAL, NewChannelSession, SendOutcome,
};
#[cfg(feature = "channels")]
pub use store::{ChannelStore, MemoryChannelStore, PendingDelivery};
