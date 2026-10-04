//! Concurrent semantic supervision of a coding worker. Start with `factory`, then `foreman` and `policy`.

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

#[cfg(test)]
extern crate self as everruns_foreman_agent;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;
