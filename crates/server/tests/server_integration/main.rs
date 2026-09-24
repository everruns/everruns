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

mod api_integration_test;
mod mcp_catalog_integration_test;
mod platform_chat_starter_test;
mod repository_conformance_test;
mod repository_integration_test;
