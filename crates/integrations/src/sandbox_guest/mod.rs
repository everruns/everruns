//! Sandbox guest agent: the provider-neutral way to run commands and move
//! files in a sandbox whose provider has no exec or file API.
//!
//! - [`server`]: the guest itself, an HTTP server inside the sandbox
//!   (binary `everruns-sandbox-guest`).
//! - [`client`]: what Everruns uses to talk to it. A provider supplies only a
//!   [`client::GuestTransport`] (how a request reaches the guest); auth,
//!   timeouts, readiness and error mapping are shared.
//!
//! The wire types live in `everruns_contracts::sandbox_guest`, so the guest
//! and the server cannot drift apart without a compile error.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use everruns_contracts::sandbox_guest::GuestExecRequest;
//! use everruns_integrations::sandbox_guest::client::{DirectTransport, GuestClient};
//!
//! let transport = Arc::new(DirectTransport::new("http://10.0.0.12:8080")?);
//! let guest = GuestClient::new(transport, "per-sandbox-token");
//! guest.wait_ready(std::time::Duration::from_secs(60)).await?;
//! let output = guest
//!     .exec(&GuestExecRequest { command: "uname -a".into(), ..Default::default() })
//!     .await?;
//! println!("{}", output.stdout);
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod server;

pub use client::{DirectTransport, GuestClient, GuestClientError, GuestTransport};
pub use server::{GuestConfig, router};
