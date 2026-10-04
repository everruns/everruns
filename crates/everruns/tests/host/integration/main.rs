//! Runtime integration modules in the facade's single `host` test binary.
//!
//! Combining these modules with MCP, policy, and dependency-direction tests
//! pays the host link cost once (see `knowledge/project/ci-build-time.md`).
//!
//! Run with: `cargo test -p everruns --features lua,bashkit,host-shell --test host -- --test-threads=1`
//! Run one module: `cargo test -p everruns --features lua,bashkit,host-shell --test host integration::<module>:: -- --test-threads=1`

mod durable_ask_user_pause_test;
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
mod runtime_artifact_store_test;
mod runtime_host_test;
mod tool_scheduler_e2e_test;
mod turn_recovery_test;
mod usage_limit_auto_continue_test;
mod workspace_paths_conformance_test;
