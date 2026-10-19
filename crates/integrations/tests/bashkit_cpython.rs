//! CPython smoke test: with the `cpython` feature the shell serves
//! `python3` from bashkit's default rootfs (no `root:` line means the host
//! /etc/passwd did not leak through).

#![cfg(feature = "bashkit")]

use std::sync::Arc;

use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::InMemorySessionFileStore;
use everruns_integrations::bashkit::BashTool;
use serde_json::json;

#[tokio::test]
async fn test_bash_python3_runs_cpython() {
    let store = Arc::new(InMemorySessionFileStore::new());
    let context = ToolContext::with_file_store(SessionId::new(), store);
    let tool = BashTool::default();

    let result = tool
        .execute_with_context(
            json!({"commands": "python3 -c \"import sys; print(sys.implementation.name)\""}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(
                output["stdout"], "cpython\n",
                "unexpected output: {:?}",
                output
            );
        }
        other => panic!("expected success, got: {:?}", other),
    }
}
