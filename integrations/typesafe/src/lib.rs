//! [TypeSafe](https://typesafe.ai) typed judgments for Everruns agents.
//!
//! This crate is the [Everruns](https://everruns.com) capability. The vendor
//! client it runs on is the standalone [`typesafe_systemone`] crate, which
//! carries no Everruns dependency and can be used on its own.
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
#[cfg(feature = "hosted")]
mod connection;
mod evaluate;
mod framework;
mod system_judgment;

pub use capability::JevCapability;
#[cfg(feature = "hosted")]
pub use connection::TypeSafeConnector;
pub use evaluate::EvaluateInput;
pub use framework::Jev;
pub use system_judgment::{
    JUDGMENT_MODEL, SystemJudgmentConfig, TypeSafeJudgmentService, UTILITY_TYPESAFE_API_KEY_ENV,
};

/// The client this capability runs on, re-exported so embedders do not need a
/// second dependency to build one.
pub use typesafe_systemone as client;
pub use typesafe_systemone::{Error, Evaluation, Question, Result, TypeSafeClient};

/// Capability id.
///
/// The capability and its tool are named for the model that answers, Jev;
/// the connection, the crate, and the API key stay named for the vendor whose
/// account issues them.
pub const CAPABILITY_ID: &str = "jev";
/// Session-secret name used when no user connection is configured.
pub const TYPESAFE_API_KEY_SECRET: &str = typesafe_systemone::API_KEY_ENV;
/// Connection provider id for the hosted connector catalog.
pub const TYPESAFE_CONNECTION_PROVIDER: &str = "typesafe";
