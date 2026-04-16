// Background tool execution contracts.
//
// Decision: foreground tool execution remains the default. Tools opt into
// detached execution with the `supports_background` hint plus a native
// `BackgroundExecutableTool` implementation.

use crate::error::Result;
use crate::tools::ToolExecutionResult;
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Structured progress reported by background tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackgroundProgress {
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub unit: Option<String>,
    pub label: Option<String>,
}

/// Final result from a completed background tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundOutcome {
    pub summary: String,
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
}

/// Sink for background status, output, and progress updates.
#[async_trait]
pub trait BackgroundEventSink: Send + Sync {
    async fn status(&self, message: &str) -> Result<()>;

    async fn output(&self, stream: &str, delta: &str) -> Result<()>;

    async fn progress(&self, progress: BackgroundProgress) -> Result<()>;
}

/// Trait implemented by tools that natively support detached execution.
#[async_trait]
pub trait BackgroundExecutableTool: Send + Sync {
    async fn execute_background(
        &self,
        arguments: Value,
        context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> std::result::Result<BackgroundOutcome, ToolExecutionResult>;
}
