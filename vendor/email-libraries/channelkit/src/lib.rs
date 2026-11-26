//! In-process messaging coordination. No server, database, or durable worker.
//! Applications own HTTP acknowledgement, authorization, persistence, and agents.
mod adapter;
mod content;
mod coordinator;
mod error;
#[cfg(feature = "http")]
pub mod http;
mod models;
mod state;
mod webhook;
pub use adapter::*;
pub use async_trait::async_trait;
pub use content::*;
pub use coordinator::*;
pub use error::*;
pub use models::*;
pub use state::*;
pub use tokio_util::sync::CancellationToken;
pub use webhook::*;

#[doc = include_str!("../docs/getting-started.md")]
pub mod guide {}
