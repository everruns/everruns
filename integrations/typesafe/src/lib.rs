#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! [TypeSafe](https://typesafe.ai) typed decision for Everruns agents.
//!
//! One crate covers the whole surface: the vendor [`client`], the agent-facing
//! `jev` capability, the connector an operator configures, and the
//! [`TypeSafeAI`] the platform wires in to back guardrail checks. It
//! brings typed decision — a calibrated number rather than prose — to the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! The `jev` capability contributes one tool, `jev_decision`: the
//! agent hands it content and its own typed questions, and gets calibrated
//! numbers back — a probability, a selected option, a graded level — instead of
//! forming a second impression in prose. Use it to verify, rate, route, or
//! classify.
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_typesafe::JevCapability;
//!
//! assert_eq!(JevCapability.id(), "jev");
//! ```
//!
//! # The vendor's own documentation
//!
//! Worth reading alongside this crate, because the concepts are theirs:
//!
//! - [System One](https://docs.typesafe.ai/concepts/system-one) — the class of
//!   model, and why it returns typed decisions rather than text.
//! - [Primitives](https://docs.typesafe.ai/primitives) — the three question
//!   types: [Noul](https://docs.typesafe.ai/primitives/noul),
//!   [Choice](https://docs.typesafe.ai/primitives/choice), and
//!   [Score](https://docs.typesafe.ai/primitives/score).
//! - [State](https://docs.typesafe.ai/concepts/state) — what to put in the
//!   value a question is asked about.
//! - [Confidence](https://docs.typesafe.ai/confidence) — how certainty is
//!   reported, and why it is not the probability.
//! - [Patterns](https://docs.typesafe.ai/patterns) — fan-out,
//!   confidence-gated routing, composite scoring, intent routing.
//!
//! For an embedded agent, hand the credential in directly:
//!
//! ```no_run
//! # fn build_agent() -> Result<(), everruns::BuildError> {
//! use everruns::{Agent, Model};
//! use everruns_integrations_typesafe::Jev;
//!
//! let agent = Agent::builder()
//!     .instructions("Rate jokes. Use jev_decision rather than judging by vibes.")
//!     .model(Model::simulated("Ready."))
//!     .capability(Jev::new("your-api-key"))
//!     .build()?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod capability;
pub mod client;
#[cfg(feature = "hosted")]
mod connection;
mod decisions;
mod evaluate;
mod framework;

pub use capability::JevCapability;
#[cfg(feature = "hosted")]
pub use capability::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};
#[cfg(feature = "hosted")]
pub use connection::TypeSafeAIConnector;
pub use evaluate::EvaluateInput;
pub use framework::Jev;

/// The vendor client this capability runs on, re-exported at the crate root so
/// callers reach it without naming the module.
pub use client::{Error, Evaluation, Question, Result, RetryPolicy, TypeSafeAIClient};
/// The decisions provider the platform wires into its host composition, and
/// the deployment credential that enables it.
pub use decisions::{
    DECISIONS_MODEL, SystemDecisionsConfig, TypeSafeAI, UTILITY_TYPESAFE_API_KEY_ENV,
};

/// Capability id.
///
/// The capability, its tool, and the decisions are named for the model that
/// answers, Jev; the client, the connection, the crate, and the API keys stay
/// named for the vendor whose account issues them.
pub const CAPABILITY_ID: &str = "jev";
/// Session-secret name used when no user connection is configured.
pub const TYPESAFE_API_KEY_SECRET: &str = client::API_KEY_ENV;
/// Connection provider id for the hosted connector catalog.
pub const TYPESAFE_CONNECTION_PROVIDER: &str = "typesafe";
