//! Single integration-test binary for `everruns-host`.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns-host` crate from scratch (see `knowledge/project/ci-build-time.md`).
//! Merging these modules into one target pays that link cost once instead of
//! once per file. `mcp_runtime_test` and `native_containment` (each declared
//! via `[[test]]` in Cargo.toml, gated behind their own feature) and
//! `dependency_direction.rs` (a static-analysis guard, not a runtime suite)
//! stay separate binaries.
//!
//! Run with: `cargo test -p everruns-host --features lua,bashkit,host-shell --test integration -- --test-threads=1`
//! Run one module: `cargo test -p everruns-host --features lua,bashkit,host-shell --test integration <module>:: -- --test-threads=1`

mod engine_planned_turn_test;
mod event_log_contract;
mod execution_contract_guard;
mod file_store_grep_conformance_test;
mod filesystem_narration_paths_test;
mod in_process_runtime_test;
mod lua_code_mode_test;
mod mid_turn_wake_test;
mod model_change_event_test;
mod model_visible_path_identity_test;
mod native_async_http;
mod resolved_snapshot_test;
mod runtime_host_test;
mod tool_scheduler_e2e_test;
mod turn_recovery_test;
mod usage_limit_auto_continue_test;
mod workspace_paths_conformance_test;
