//! A classifier supervising a coding agent it never has to stop.
//!
//! A Framework port of [Foreman](https://github.com/thruwire/foreman): a fast
//! decision model placed above a slower coding agent. The worker keeps its own
//! loop — an Everruns session, or an external CLI like Codex or yolop — while
//! the supervisor turns bounded evidence into nine probabilities in one request
//! and hands them to a policy written in ordinary Rust.
//!
//! ```text
//! foreman run --repo ./my-project --job "Add rate limiting, and test it."
//! ```
//!
//! The runtime is a library so that the binary beside it stays the real thing:
//! a run against a repository you name, on a model you pay for. The credential-
//! free walkthrough is a separate crate in `demo/`, built on this one, and
//! nothing in here knows it exists.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod agent;
pub mod cli;
pub mod factory;
pub mod foreman;
pub mod observation;
pub mod policy;
pub mod run;
pub mod terminal;
pub mod worker;
