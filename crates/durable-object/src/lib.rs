//! Experimental Durable Object authority for the shared native session actor driver.
//!
//! The Worker owns durable admission, the journal, execution epochs and alarms.
//! Native executors supply ordinary Everruns hosts; no agent planner lives in JS.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod client;
#[cfg(feature = "native")]
mod executor;
mod hosting;
mod session_api;
pub use session_api::SessionApi;
#[cfg(feature = "native")]
mod http_transport;
mod stores;
pub use hosting::ObjectHosting;
#[cfg(all(feature = "wasm", target_arch = "wasm32"))]
pub mod wasm;

pub use client::{
    Authority, AuthorityClient, AuthorityTransport, Claim, EventBatch, SessionBinding,
};
#[cfg(feature = "native")]
pub use executor::Executor;
