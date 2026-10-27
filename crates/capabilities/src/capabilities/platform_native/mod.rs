//! Inventory-generated native functions over the authorized Platform dispatcher.

use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde::Deserialize;
use serde_json::Value;
use std::sync::LazyLock;

mod schema;

#[derive(Deserialize)]
struct Operation {
    name: String,
    description: String,
    read_only: bool,
    input_schema: Value,
}

static OPERATIONS: LazyLock<Vec<Operation>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("operations.json")).expect("generated platform operations")
});

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    OPERATIONS
        .iter()
        .map(|operation| {
            Box::new(PlatformNativeTool {
                operation,
                name: format!("platform_{}", operation.name),
            }) as Box<dyn Tool>
        })
        .collect()
}

struct PlatformNativeTool {
    operation: &'static Operation,
    name: String,
}

#[async_trait]
impl Tool for PlatformNativeTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.operation.description
    }

    fn parameters_schema(&self) -> Value {
        schema::parameters(&self.operation.input_schema)
    }

    fn hints(&self) -> ToolHints {
        // Read-only commands can still contact remote services. Replay remains
        // conservative unless a command explicitly owns a stronger contract.
        ToolHints {
            readonly: Some(self.operation.read_only),
            ..Default::default()
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Platform command context is required")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if !arguments.is_object() || arguments.get("organization_id").is_some() {
            return ToolExecutionResult::tool_error(
                "Expected command arguments scoped to the current session organization",
            );
        }
        let Some(store) = context.extension::<crate::platform_store::PlatformStoreExt>() else {
            return ToolExecutionResult::tool_error("Platform command service is unavailable");
        };
        // THREAT[TM-AGENT-017]: the same Run edge as the shell re-resolves the
        // session's current capability, caller, tenant and command policy.
        match store.0.platform_run(&self.operation.name, arguments).await {
            Ok(output) => ToolExecutionResult::success(output),
            Err(error) => ToolExecutionResult::tool_error(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
