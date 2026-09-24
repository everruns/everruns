//! Single integration-test binary for the "domain" CI shard.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns-server` crate from scratch (see `knowledge/project/ci-build-time.md`):
//! most of the shard's wall-clock was compiling, not testing. Merging these
//! modules into one target pays that link cost once instead of once per file.
//!
//! Run with: `cargo test -p everruns-server --test domain -- --test-threads=1`
//! Run one module: `cargo test -p everruns-server --test domain <module>:: -- --test-threads=1`
//!
//! A handful of `tests/*.rs` files stay as their own binaries and are not
//! listed here: some install process-global state (a metrics recorder, an
//! env var) that only one process may touch, and others are already carved
//! out for unrelated reasons (a dedicated S3 job, strict-mode env mutation).
//! See the "Run domain integration tests" step in `.github/workflows/ci.yml`
//! for the current list.

#[path = "../test_harness.rs"]
mod test_harness;

mod ag_ui_integration_test;
mod agent_endpoints_migration_test;
mod agent_trigger_invocation_integration_test;
mod app_a2a_ask_user_test;
mod app_a2a_integration_test;
mod app_api_integration_test;
mod app_invocation_channels_integration_test;
mod auth_integration_test;
mod cli_auth_no_org_test;
mod cli_auth_test;
mod client_side_tools_test;
mod command_policy_enforcement_test;
mod db_pool_isolation_test;
mod endpoint_attribution_test;
mod evals_integration_test;
mod fcp_integration_test;
mod guardrails_integration_test;
mod llm_model_default_test;
mod mcp_acts_as_grpc_test;
mod mcp_endpoint_test;
mod mcp_oauth_user_switch_test;
mod migration_history_test;
mod observers_integration_test;
mod openapi_coverage_test;
mod openapi_descriptions_test;
mod openapi_sdk_metadata_test;
mod org_creation_test;
mod org_invitations_test;
mod org_isolation_test;
mod org_lifecycle_test;
mod reporting_integration_test;
mod schedule_integration_test;
mod service_mcp_oauth_lifecycle_test;
mod session_git_integration_test;
mod session_tab_counts_integration_test;
mod session_workspace_attach_test;
mod skills_integration_test;
mod subagent_spawn_handles_test;
mod webhook_trigger_migration_test;
mod workspace_files_integration_test;
