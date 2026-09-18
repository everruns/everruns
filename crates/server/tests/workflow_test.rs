//! Workflow tests for Everruns API
//!
//! These tests require a RUNNING API server and Worker process.
//! They test end-to-end workflows including LLM execution.
//!
//! For API endpoint testing without a running server, use:
//! - `api_integration_test.rs` (in-process testing with PostgreSQL)
//! - `repository_integration_test.rs` (direct repository testing)
//!
//! Run with: cargo test -p everruns-server --test workflow_test -- --test-threads=1
//!
//! Requirements:
//! - API server running at localhost:9000
//! - Worker process running
//! - PostgreSQL with migrations applied
//! - Uses LlmSim for workflow tests, no real API keys needed

// #[macro_use]: support defines skip_on_provider_account_block!, and
// macro_rules is textually scoped — the modules below only see it if
// support is declared first.
#[macro_use]
#[path = "workflow_test/support.rs"]
mod support;
#[path = "workflow_test/basics.rs"]
mod basics;
#[path = "workflow_test/capabilities.rs"]
mod capabilities;
#[path = "workflow_test/filesystem.rs"]
mod filesystem;
#[path = "workflow_test/messages.rs"]
mod messages;
#[path = "workflow_test/sessions.rs"]
mod sessions;
#[path = "workflow_test/streaming.rs"]
mod streaming;
#[path = "workflow_test/thinking.rs"]
mod thinking;
#[path = "workflow_test/tool_calls.rs"]
mod tool_calls;
