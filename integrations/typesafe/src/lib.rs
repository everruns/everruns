//! [TypeSafe](https://typesafe.ai) System One judgments for Rust, and the
//! Everruns capability built on them.
//!
//! A System One model answers *typed questions* about state: a probability, a
//! selected option, a graded level. It does not write prose. That makes it a
//! programming primitive — your code keeps the workflow and asks the model only
//! for the semantic judgment it cannot compute itself.
//!
//! # Standalone client
//!
//! The client layer has no Everruns dependency. Depend on it with
//! `default-features = false` and it is a plain TypeSafe SDK:
//!
//! ```no_run
//! # async fn run() -> Result<(), everruns_integrations_typesafe::Error> {
//! use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeClient};
//!
//! let client = TypeSafeClient::from_env()?; // TYPESAFE_API_KEY
//!
//! // Independent questions over the same state go in one request and run in
//! // parallel. Batching is the cheap path.
//! let judgment = client
//!     .evaluate(
//!         Evaluation::new("I've been on hold for two hours and nobody can tell me why.")
//!             .ask("is_urgent", Question::noul("Does this convey urgency?"))
//!             .ask(
//!                 "team",
//!                 Question::choice(
//!                     "Which team should handle this?",
//!                     [
//!                         ("billing", "Payments, invoicing, refunds"),
//!                         ("technical", "Bugs, outages, integrations"),
//!                     ],
//!                 ),
//!             ),
//!     )
//!     .await?;
//!
//! if judgment.noul("is_urgent")? > 0.8 {
//!     let team = judgment.choice("team")?;
//!     // Confidence is a second axis: the answer says what, confidence says
//!     // whether to act on it without a human.
//!     if team.confidence > 0.7 {
//!         println!("routing to {}", team.choice);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # As an Everruns capability
//!
//! With the default features, the crate also contributes the `typesafe`
//! capability: a `typesafe_evaluate` tool that lets an agent verify its own or
//! another party's claims and get calibrated numbers back instead of a second
//! opinion in prose.
//!
//! ```
//! # #[cfg(feature = "capability")] {
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_typesafe::TypeSafeCapability;
//!
//! assert_eq!(TypeSafeCapability.id(), "typesafe");
//! # }
//! ```
//!
//! For an embedded agent, hand the credential in directly:
//!
//! ```no_run
//! # #[cfg(feature = "capability")]
//! # fn build_agent() -> Result<(), everruns::BuildError> {
//! use everruns::{Agent, Model};
//! use everruns_integrations_typesafe::TypeSafe;
//!
//! let agent = Agent::builder()
//!     .instructions("Rate jokes. Use typesafe_evaluate rather than judging by vibes.")
//!     .model(Model::simulated("Ready."))
//!     .capability(TypeSafe::new("your-api-key"))
//!     .build()?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

pub mod answer;
pub mod client;
pub mod error;
pub mod question;

pub use answer::{Answer, ChoiceAnswer, Judgment, NoulAnswer, ScoreAnswer, Usage};
pub use client::{
    API_KEY_ENV, DEFAULT_BASE_URL, RetryPolicy, TypeSafeClient, TypeSafeClientBuilder,
};
pub use error::{Error, Result};
pub use question::{DEFAULT_MODEL, Evaluation, NoulCriteria, Question};

#[cfg(feature = "capability")]
mod capability;
#[cfg(feature = "hosted")]
mod connection;
#[cfg(feature = "capability")]
mod evaluate;
#[cfg(feature = "capability")]
mod framework;

#[cfg(feature = "capability")]
pub use capability::TypeSafeCapability;
#[cfg(feature = "hosted")]
pub use connection::TypeSafeConnector;
#[cfg(feature = "capability")]
pub use evaluate::EvaluateInput;
#[cfg(feature = "capability")]
pub use framework::TypeSafe;

/// Capability id, and the connection provider it resolves credentials from.
#[cfg(feature = "capability")]
pub const CAPABILITY_ID: &str = "typesafe";
/// Session-secret name used when no user connection is configured.
#[cfg(feature = "capability")]
pub const TYPESAFE_API_KEY_SECRET: &str = API_KEY_ENV;
/// Connection provider id for the hosted connector catalog.
#[cfg(feature = "capability")]
pub const TYPESAFE_CONNECTION_PROVIDER: &str = "typesafe";
