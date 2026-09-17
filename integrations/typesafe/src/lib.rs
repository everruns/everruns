//! [TypeSafe](https://typesafe.ai) typed classification for Everruns agents.
//!
//! One crate covers the whole surface: the vendor [`client`], the agent-facing
//! `jev` capability, the connector an operator configures, and the
//! [`TypeSafeClassifier`] the platform wires in to back guardrail checks. It
//! brings typed classification — a calibrated number rather than prose — to the
//! [Everruns](https://everruns.com) ecosystem.
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
mod classifier;
pub mod client;
#[cfg(feature = "hosted")]
mod connection;
mod evaluate;
mod framework;

#[cfg(feature = "hosted")]
pub use capability::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};
pub use capability::JevCapability;
#[cfg(feature = "hosted")]
pub use connection::TypeSafeConnector;
pub use evaluate::EvaluateInput;
pub use framework::Jev;

/// The classifier the platform wires into its host composition, and the
/// deployment credential that enables it.
pub use classifier::{
    CLASSIFIER_MODEL, SystemClassifierConfig, TypeSafeClassifier, UTILITY_TYPESAFE_API_KEY_ENV,
};
/// The vendor client this capability runs on, re-exported at the crate root so
/// callers reach it without naming the module.
pub use client::{Error, Evaluation, Question, Result, RetryPolicy, TypeSafeClient};

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
