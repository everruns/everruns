//! # Source-only Durable Object hosting reference
//!
//! An independently resolved, unpublished implementation of Framework hosting.
//! Coherent session authority beneath Framework Engine, part of the
//! [Everruns](https://everruns.com) ecosystem. Native and Wasm placements run
//! ordinary Agents through the same actor driver; JavaScript supplies storage,
//! transport and alarms.
//!
//! ```no_run
//! use everruns::{Agent, Engine, MessageId, OpenAI, SessionId, hosting::AgentBuild};
//! use everruns_durable_object::{AuthorityClient, ObjectHosting};
//!
//! # #[cfg(feature = "native")]
//! # async fn submit() -> Result<(), Box<dyn std::error::Error>> {
//! let client = AuthorityClient::new(
//!     "https://authority.example.com", "tenant", "replace-with-a-long-random-service-secret",
//! )?;
//! let agent = Agent::builder()
//!     .instructions("Answer concisely.")
//!     .provider(OpenAI::from_env()?)
//!     .model("gpt-6.1-sol")
//!     .build()?;
//! let build = AgentBuild::new("assistant", "immutable-build-v1");
//! let engine = Engine::builder()
//!     .hosting(ObjectHosting::new(client))
//!     .register(build.clone(), agent)
//!     .build();
//! let session = engine.create_registered(SessionId::new(), &build).await?;
//! let receipt = session.send_with_id(MessageId::new(), "Hello.").await?;
//! println!("admitted {}", receipt.message_id);
//! # Ok(())
//! # }
//! # fn main() {}
//! ```
//!
//! See the [hosting guide](https://everruns.com/docs/framework/durable-objects/).

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
