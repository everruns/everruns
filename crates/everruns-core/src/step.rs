// Loop step representation
//
// LoopStep represents a single iteration of the agentic loop.
// This abstraction allows the loop to be decomposed into discrete steps
// that can be executed independently (e.g., as Temporal activities).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_contracts::tools::{ToolCall, ToolResult};

use crate::message::Message;

/// The kind of step being executed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepKind {
    /// Initial setup step
    Setup,
    /// LLM inference step
    LlmCall,
    /// Tool execution step
    ToolExecution,
    /// Finalization step
    Finalize,
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepKind::Setup => write!(f, "setup"),
            StepKind::LlmCall => write!(f, "llm_call"),
            StepKind::ToolExecution => write!(f, "tool_execution"),
            StepKind::Finalize => write!(f, "finalize"),
        }
    }
}

/// A single step in the agentic loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopStep {
    /// Unique step ID
    pub id: Uuid,

    /// Session this step belongs to
    pub session_id: Uuid,

    /// Iteration number (1-indexed)
    pub iteration: usize,

    /// Kind of step
    pub kind: StepKind,

    /// When the step started
    pub started_at: DateTime<Utc>,

    /// When the step completed (None if still running)
    pub completed_at: Option<DateTime<Utc>>,

    /// Step result (None if still running or failed)
    pub result: Option<StepResult>,
}

/// Result of a step execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepResult {
    /// Setup completed successfully
    SetupComplete { message_count: usize },

    /// LLM call completed
    LlmCallComplete {
        /// Assistant's text response
        response_text: String,
        /// Tool calls requested (empty if none)
        tool_calls: Vec<ToolCall>,
        /// Whether to continue the loop
        continue_loop: bool,
    },

    /// Tool execution completed
    ToolExecutionComplete {
        /// Results for each tool call
        results: Vec<ToolResult>,
    },

    /// Finalization completed
    FinalizeComplete {
        /// Final assistant response
        final_response: Option<String>,
    },
}

impl LoopStep {
    /// Create a new setup step
    pub fn setup(session_id: Uuid) -> Self {
        Self {
            id: Uuid::now_v7(),
            session_id,
            iteration: 0,
            kind: StepKind::Setup,
            started_at: Utc::now(),
            completed_at: None,
            result: None,
        }
    }

    /// Create a new LLM call step
    pub fn llm_call(session_id: Uuid, iteration: usize) -> Self {
        Self {
            id: Uuid::now_v7(),
            session_id,
            iteration,
            kind: StepKind::LlmCall,
            started_at: Utc::now(),
            completed_at: None,
            result: None,
        }
    }

    /// Create a new tool execution step
    pub fn tool_execution(session_id: Uuid, iteration: usize) -> Self {
        Self {
            id: Uuid::now_v7(),
            session_id,
            iteration,
            kind: StepKind::ToolExecution,
            started_at: Utc::now(),
            completed_at: None,
            result: None,
        }
    }

    /// Create a new finalize step
    pub fn finalize(session_id: Uuid, iteration: usize) -> Self {
        Self {
            id: Uuid::now_v7(),
            session_id,
            iteration,
            kind: StepKind::Finalize,
            started_at: Utc::now(),
            completed_at: None,
            result: None,
        }
    }

    /// Mark the step as completed with a result
    pub fn complete(mut self, result: StepResult) -> Self {
        self.completed_at = Some(Utc::now());
        self.result = Some(result);
        self
    }

    /// Check if the step is completed
    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }

    /// Get duration in milliseconds if completed
    pub fn duration_ms(&self) -> Option<i64> {
        self.completed_at
            .map(|end| (end - self.started_at).num_milliseconds())
    }
}

/// Input for executing a step (used for decomposed execution)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepInput {
    /// Session ID
    pub session_id: Uuid,

    /// Current iteration
    pub iteration: usize,

    /// Current messages in the conversation
    pub messages: Vec<Message>,

    /// Pending tool calls to execute (for tool execution steps)
    pub pending_tool_calls: Vec<ToolCall>,
}

impl StepInput {
    /// Create input for a new loop
    pub fn new(session_id: Uuid, messages: Vec<Message>) -> Self {
        Self {
            session_id,
            iteration: 0,
            messages,
            pending_tool_calls: Vec::new(),
        }
    }

    /// Create input for a tool execution step
    pub fn for_tool_execution(
        session_id: Uuid,
        iteration: usize,
        messages: Vec<Message>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            session_id,
            iteration,
            messages,
            pending_tool_calls: tool_calls,
        }
    }

    /// Advance to next iteration
    pub fn next_iteration(mut self) -> Self {
        self.iteration += 1;
        self.pending_tool_calls.clear();
        self
    }
}

/// Output from executing a step (used for decomposed execution)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutput {
    /// The completed step
    pub step: LoopStep,

    /// Updated messages (including any new messages from this step)
    pub messages: Vec<Message>,

    /// Whether the loop should continue
    pub continue_loop: bool,

    /// Pending tool calls for next step (if any)
    pub pending_tool_calls: Vec<ToolCall>,
}

impl StepOutput {
    /// Create output indicating the loop should continue
    pub fn continue_with(
        step: LoopStep,
        messages: Vec<Message>,
        pending_tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            step,
            messages,
            continue_loop: true,
            pending_tool_calls,
        }
    }

    /// Create output indicating the loop is complete
    pub fn complete(step: LoopStep, messages: Vec<Message>) -> Self {
        Self {
            step,
            messages,
            continue_loop: false,
            pending_tool_calls: Vec::new(),
        }
    }
}
