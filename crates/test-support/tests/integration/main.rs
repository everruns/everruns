//! Single integration-test binary for `everruns-test-support`.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns-test-support` crate from scratch (see
//! `knowledge/project/ci-build-time.md`). Merging these modules into one
//! target pays that link cost once instead of once per file.
//!
//! Run with: `cargo test -p everruns-test-support --all-features --test integration`
//! Run one module: `cargo test -p everruns-test-support --all-features --test integration <module>::`

mod ask_user_test;
mod command_host_test;
mod in_memory_fixtures_test;
mod llmsim_migration_bridge;
mod mcp_harness_test;
mod message_metadata_test;
mod mid_turn_reasoning_effort_test;
mod prompt_budget_fixtures;
mod reason_atom_test;
mod seed_events_test;
