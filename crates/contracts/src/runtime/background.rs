// Background tool execution contracts.
//
// Decision: foreground tool execution remains the default. Tools opt into
// detached execution with the `supports_background` hint plus a native
// `BackgroundExecutableTool` implementation.

use crate::runtime::error::Result;
use crate::runtime::tool_context::ToolContext;
use crate::runtime::tools::ToolExecutionResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Structured progress reported by background tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BackgroundProgress {
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub unit: Option<String>,
    pub label: Option<String>,
    /// Ordered checklist, for work that plans its steps up front (a thread's
    /// assignment). Empty for plain counters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ProgressStep>,
}

/// One checklist line in [`BackgroundProgress::steps`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProgressStep {
    pub title: String,
    pub status: ProgressStepStatus,
}

/// State of one checklist line.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProgressStepStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
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
