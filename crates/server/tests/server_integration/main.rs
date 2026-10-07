//! Single integration-test binary for the server shard's PostgreSQL-backed API
//! and repository suites.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns-server` crate from scratch (see `knowledge/project/ci-build-time.md`).
//! Merging these modules into one target pays that link cost once instead of
//! once per file.
//!
//! Run with: `cargo test -p everruns-server --test server_integration -- --test-threads=1`
//! Run one module: `cargo test -p everruns-server --test server_integration <module>:: -- --test-threads=1`

#[path = "../test_harness.rs"]
mod test_harness;

mod agent_avatar_storage_test;
mod agents_api_lifecycle_test;
mod api_integration_test;
mod harness_levels_test;
mod late_generation_usage_test;
mod mcp_catalog_integration_test;
mod organization_connections_test;
mod platform_chat_starter_test;
mod platform_chat_upgrade_test;
mod repository_conformance_test;
mod repository_integration_test;
mod sandbox_fleet_test;
mod sandbox_state_test;
mod session_row_fixture;
