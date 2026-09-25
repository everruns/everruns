//! # serve (experimental)
//!
//! An agent framework in the style of [Topcoat], with hosting modelled on
//! [eve], built on the `everruns` runtime. It is part of the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! > **Experimental.** serve is a proof of concept. Every API here may change
//! > or disappear; it is published as `everruns-serve` but not covered by
//! > the everruns stability policy.
//!
//! ```no_run
//! use serve::prelude::*;
//!
//! /// Rolls dice for board-game nights.
//! #[agent]
//! fn assistant() -> Agent {
//!     Agent::builder()
//!         .model("anthropic/claude-sonnet-5")
//!         .instructions("Roll dice when asked.")
//!         .build()
//! }
//!
//! /// Roll one die with the given number of sides.
//! #[tool]
//! async fn roll_dice(cx: &Cx, sides: u32) -> Result<u32> {
//!     cx.progress(format!("rolling a d{sides}")).await;
//!     Ok(sides)
//! }
//!
//! #[tokio::main]
//! async fn main() -> serve::Result {
//!     serve::start(App::builder().discover().build()).await
//! }
//! ```
//!
//! A real app also calls `serve::assets!()` once and embeds `agent/**` with
//! `serve_build::embed()` in `build.rs`, so prompts and skills can live in
//! Markdown files (see `md!`).
//!
//! The pieces:
//!
//! - **Attribute macros** ([`agent`], [`tool`], [`channel`], [`schedule`],
//!   [`connection`], [`eval`]) register items at link time.
//! - **[`App::builder().discover()`](AppBuilder::discover)** collects them,
//!   validates the file-layout conventions, and resolves `agent/**` assets
//!   embedded by `serve-build`.
//! - **[`Manifest`]** is what the build declares and the host provides:
//!   schedules, channel routes, secrets, the sandbox, model strings.
//! - **[`Cx`]** is the one context type: the tool call (session, turn, call
//!   id, progress), connections, secrets, starting sessions.
//! - **[`start`]** runs the binary as `dev`, `start`, `manifest`, `eval` or
//!   `deploy`, serving everywhere the same `/v1` wire API, a subset of the
//!   everruns server's session API (so the everruns SDK can drive it).
//!
//! [Topcoat]: https://github.com/tokio-rs/topcoat
//! [eve]: https://vercel.com/docs/eve

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

// Lets the macros' `::serve::…` paths resolve in this crate's own tests.
extern crate self as serve;

mod agent;
mod app;
mod channel;
mod cli;
mod config;
mod connection;
mod cx;
mod eval;
mod gateway;
mod host;
mod manifest;
mod registry;
mod scheduler;
mod server;
mod store;
#[cfg(test)]
mod wire_tests;

pub use agent::{Agent, AgentBuilder, Instructions, Markdown};
pub use app::{App, AppBuilder, Mode};
pub use channel::{Channel, ChannelEvent, Inbound, Slack, Webhook};
pub use cli::start;
pub use config::{AppConfig, SandboxKind};
pub use connection::{McpServer, Secret};
pub use cx::{Cx, DeliveryTarget, StartSession};
pub use eval::{EvalCx, EvalReport, EvalResult, OnApproval, TurnCheck, TurnRecord};
pub use manifest::Manifest;

pub use serve_macros::{agent, channel, connection, eval, schedule, tool};

/// Simulated models for running agents offline, re-exported from `everruns`.
pub mod sim {
    pub use everruns::{LlmSimConfig, OnExhausted, SimToolCall, SimTurn};

    /// A scripted simulator that replays `turns` for every user message: tool
    /// calls first, then the reply. Loops, so each message sees the same
    /// script. Used when no model gateway is configured.
    pub fn script(turns: impl IntoIterator<Item = SimTurn>) -> LlmSimConfig {
        LlmSimConfig::scripted(turns.into_iter().collect()).with_on_exhausted(OnExhausted::Loop)
    }

    /// A scripted tool call for [`script`].
    pub fn call(name: impl Into<String>, arguments: serde_json::Value) -> SimTurn {
        SimTurn::ToolCalls(vec![SimToolCall {
            name: name.into(),
            arguments,
            id: None,
        }])
    }

    /// A scripted assistant reply for [`script`].
    pub fn reply(text: impl Into<String>) -> SimTurn {
        SimTurn::Assistant(text.into())
    }
}

/// The error type used across serve. Anything `std::error::Error` converts
/// into it with `?`.
pub type Error = anyhow::Error;

/// `Result` with serve's [`Error`]. `-> Result` alone means `Result<()>`.
pub type Result<T = (), E = Error> = std::result::Result<T, E>;

/// Everything an application file usually imports.
pub mod prelude {
    pub use crate::{
        Agent, App, Channel, Cx, DeliveryTarget, EvalCx, McpServer, Result, Secret, Slack, Webhook,
        agent, channel, connection, eval, md, schedule, tool,
    };
    pub use anyhow::{anyhow, bail, ensure};
    pub use serde::{Deserialize, Serialize};
    pub use serde_json::json;
}

/// Include the assets `serve_build::embed()` generated for this crate.
///
/// Call once, at the crate root. Without it `agent/**` files are not in the
/// binary and `discover()` reports them missing.
#[macro_export]
macro_rules! assets {
    () => {
        include!(concat!(env!("OUT_DIR"), "/serve_assets.rs"));
        $crate::__private::inventory::submit! {
            $crate::__private::AppInfoRegistration {
                name: env!("CARGO_PKG_NAME"),
                version: env!("CARGO_PKG_VERSION"),
            }
        }
    };
}

/// Embed a Markdown file from this crate's `agent/` directory.
///
/// The text is compiled in (so a missing file is a build error), and in `dev`
/// mode it is re-read from disk on every new session, so prompt edits
/// hot-reload without a rebuild.
#[macro_export]
macro_rules! md {
    ($path:literal) => {
        $crate::Markdown::__embedded(
            $path,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/agent/", $path)),
            concat!(env!("CARGO_MANIFEST_DIR"), "/agent/", $path),
        )
    };
}

/// Runtime support for macro expansions. Not an API.
#[doc(hidden)]
pub mod __private {
    pub use crate::registry::*;
    pub use futures::future::BoxFuture;
    pub use inventory;
    pub use schemars;
    pub use serde;
    pub use serde_json::Value;
}
