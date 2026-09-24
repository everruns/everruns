//! API Integration tests for Everruns using in-process server with PostgreSQL
//!
//! These tests run against a real PostgreSQL database but don't require
//! a running server process - the server routes are tested in-process
//! using tower's oneshot method.
//!
//! Run with: cargo test -p everruns-server --test server_integration api_integration_test:: -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)

mod agents;
mod apps;
mod files_misc;
mod harnesses;
mod providers_models;
mod sessions;
mod support;
