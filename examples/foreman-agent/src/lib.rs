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
//! foreman demo
//! ```
//!
//! Both are real runs on real credentials. `demo` only supplies the repository
//! and the job, so the starting state is fixed and the ending is checkable;
//! nothing about the worker or the supervisor changes. The runtime is a library
//! so the offline test beside it can drive the same loop with a scripted worker
//! without that scaffolding living in the example anyone reads.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod agent;
pub mod cli;
pub mod factory;
pub mod fixture;
pub mod foreman;
pub mod observation;
pub mod policy;
pub mod run;
pub mod terminal;
pub mod worker;
