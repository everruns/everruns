//! The shared channel runtime: how a message from a platform becomes a turn,
//! and how the turn's replies get back to the platform.
//!
//! The contract types (inbound events, drivers, delivery adapters, bindings)
//! live in [`crate::channel`]. This module holds the one implementation the
//! Framework, serve and the server share:
//!
//! - [`TurnDelivery`]: one turn's events in, platform calls out.
//! - [`ChannelHost`]: request in, response out; binds threads to sessions,
//!   drops duplicates, starts turns and runs their delivery.
//! - [`ChannelStore`]: what a host persists, with an in-memory default.
//! - [`approval_prompt`] and [`TaskProgress`]: what a delivery shows for a
//!   `request_approval` pause and for a turn's task fan-out.
//!
//! Hosts plug in only a [`ChannelSessionPort`], a store and their HTTP stack;
//! platforms plug in a [`ChannelDriver`](crate::channel::ChannelDriver).

// Private: children name the contract types through `super::`.
use everruns_contracts::runtime::channel::*;

mod approval;
mod delivery;
mod host;
mod progress;
mod store;

pub use approval::{REQUEST_APPROVAL_TOOL, approval_prompt};

pub use delivery::{
    DeliveryEvent, DeliveryOptions, DeliveryStep, STREAM_FLUSH_CHARS, TurnDelivery,
    is_terminal_turn_event, response_text,
};
pub use host::{
    ChannelConfig, ChannelEventStream, ChannelHost, ChannelHostBuilder, ChannelSessionPort,
    Conversation, DEFAULT_FLUSH_INTERVAL, NewChannelSession, SendOutcome, StreamTurn,
};
pub use progress::TaskProgress;
pub use store::{ChannelStore, MemoryChannelStore, PendingDelivery};
