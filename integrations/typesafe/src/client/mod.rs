//! A client for [TypeSafe](https://typesafe.ai)'s System One API.
//!
//! A System One model answers *typed questions* about state: a probability, a
//! selected option, a graded level. It does not write prose and it does not
//! explain itself. That makes it a programming primitive — your code keeps the
//! workflow and asks the model only for the semantic judgment it cannot
//! compute.
//!
//! Reach for it where you would otherwise prompt a chat model and parse JSON
//! out of its answer: verification, rating, routing, moderation, reranking.
//!
//! This module is the vendor edge and depends on nothing else in this crate:
//! the capability, the connector, and the judgment service above it all speak
//! to the API through [`TypeSafeAIClient`].
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), everruns_integrations_typesafe::Error> {
//! use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeAIClient};
//!
//! let client = TypeSafeAIClient::from_env()?; // TYPESAFE_API_KEY
//!
//! // Independent questions over the same state go in one request and are
//! // answered in parallel. Batching is the cheap path.
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
//!     // whether to act on it without a person.
//!     if team.confidence > 0.7 {
//!         println!("routing to {}", team.choice);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Reading the answers
//!
//! - A [`Question::noul`] returns the probability of *yes*. A value near 0.5
//!   means yes and no are near-equally likely — not "medium intensity".
//! - A [`Question::choice`] returns the selected option, the probability of
//!   every option, and a confidence derived from that distribution.
//! - A [`Question::score`] returns a probability-weighted position across your
//!   ordered levels. For an "any serious hit" rule, read
//!   [`ScoreAnswer::probability_at_or_above`] rather than the weighted score:
//!   an answer that is probably fine and possibly awful must not average into
//!   fine.
//!
//! Typed output guarantees the interface, not the truth. Validate thresholds
//! against your own data and consequences.

#![warn(missing_docs)]

pub mod answer;
pub mod error;
pub mod http;
pub mod question;

pub use answer::{Answer, ChoiceAnswer, Judgment, NoulAnswer, ScoreAnswer, Usage};
pub use error::{Error, Result};
pub use http::{
    API_KEY_ENV, DEFAULT_BASE_URL, RetryPolicy, TypeSafeAIClient, TypeSafeAIClientBuilder,
};
pub use question::{DEFAULT_MODEL, Evaluation, NoulCriteria, Question};
