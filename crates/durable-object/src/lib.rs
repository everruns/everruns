//! Experimental Durable Object authority for the shared native session actor driver.
//!
//! The Worker owns durable admission, the journal, execution epochs and alarms.
//! Native executors supply ordinary Everruns hosts; no agent planner lives in JS.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod client;
mod executor;
mod stores;

pub use client::{Authority, AuthorityClient, Claim, EventBatch, SessionBinding};
pub use executor::{Executor, HostFactory};
