//! [TypeSafe](https://typesafe.ai) typed judgments for Everruns agents.
//!
//! One crate covers the whole surface: the vendor [`client`], the agent-facing
//! `jev` capability, the connector an operator configures, and the
//! [`TypeSafeJudgmentService`] the platform wires in to back guardrail checks.
//!
//! The `jev` capability contributes one tool, `jev_evaluate`: the
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
//! For an embedded agent, hand the credential in directly:
//!
//! ```no_run
//! # fn build_agent() -> Result<(), everruns::BuildError> {
//! use everruns::{Agent, Model};
//! use everruns_integrations_typesafe::Jev;
//!
//! let agent = Agent::builder()
//!     .instructions("Rate jokes. Use jev_evaluate rather than judging by vibes.")
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
mod evaluate;
mod framework;
mod judgment;

pub use capability::JevCapability;
#[cfg(feature = "hosted")]
pub use connection::TypeSafeConnector;
pub use evaluate::EvaluateInput;
pub use framework::Jev;

/// The vendor client this capability runs on, re-exported at the crate root so
/// callers reach it without naming the module.
pub use client::{Error, Evaluation, Question, Result, RetryPolicy, TypeSafeClient};
/// The judgment service the platform wires into its host composition, and the
/// deployment credential that enables it.
pub use judgment::{
    JUDGMENT_MODEL, SystemJudgmentConfig, TypeSafeJudgmentService, UTILITY_TYPESAFE_API_KEY_ENV,
};

/// Capability id.
///
/// The capability and its tool are named for the model that answers, Jev;
/// the connection, the crate, and the API key stay named for the vendor whose
/// account issues them.
pub const CAPABILITY_ID: &str = "jev";
/// Session-secret name used when no user connection is configured.
pub const TYPESAFE_API_KEY_SECRET: &str = client::API_KEY_ENV;
/// Connection provider id for the hosted connector catalog.
pub const TYPESAFE_CONNECTION_PROVIDER: &str = "typesafe";
