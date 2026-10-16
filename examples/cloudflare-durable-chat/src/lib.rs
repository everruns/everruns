//! Durable multiplayer chat hosted directly by Cloudflare Durable Objects.
//!
//! The portable Everruns kernel advances one engine step per commit. The
//! Cloudflare host supplies native SQLite, alarms, fetch and WebSockets.

pub mod chat;
pub mod engine;
pub mod openai;

#[cfg(target_arch = "wasm32")]
mod worker;
