//! API Integration tests for Everruns using in-process server with PostgreSQL
//!
//! These tests run against a real PostgreSQL database but don't require
//! a running server process - the server routes are tested in-process
//! using tower's oneshot method.
//!
//! Run with: cargo test -p everruns-server --test api_integration_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)

mod test_harness;

#[path = "api_integration_test/agents.rs"]
mod agents;
#[path = "api_integration_test/apps.rs"]
mod apps;
#[path = "api_integration_test/files_misc.rs"]
mod files_misc;
#[path = "api_integration_test/harnesses.rs"]
mod harnesses;
#[path = "api_integration_test/providers_models.rs"]
mod providers_models;
#[path = "api_integration_test/sessions.rs"]
mod sessions;
#[path = "api_integration_test/support.rs"]
mod support;
