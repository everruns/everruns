// Loop events for streaming
//
// LoopEvent wraps AG-UI events with additional loop-specific context.
// This allows the loop to emit standard AG-UI events while also providing
// higher-level loop status events.

use crate::ag_ui::AgUiEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Events emitted during loop execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LoopEvent {
    /// AG-UI protocol event (for SSE streaming compatibility)
    AgUi(AgUiEvent),

    /// Loop started
    LoopStarted {
        session_id: String,
        timestamp: DateTime<Utc>,
    },

    /// Loop iteration started
    IterationStarted {
        session_id: String,
        iteration: usize,
        timestamp: DateTime<Utc>,
    },

    /// LLM call started
    LlmCallStarted {
        session_id: String,
        iteration: usize,
        timestamp: DateTime<Utc>,
    },

    /// LLM streaming text delta
    TextDelta {
        session_id: String,
        message_id: String,
        delta: String,
        timestamp: DateTime<Utc>,
    },

    /// LLM call completed
    LlmCallCompleted {
        session_id: String,
        iteration: usize,
        has_tool_calls: bool,
        timestamp: DateTime<Utc>,
    },

    /// Tool execution started
    ToolExecutionStarted {
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        timestamp: DateTime<Utc>,
    },

    /// Tool execution completed
    ToolExecutionCompleted {
        session_id: String,
        tool_call_id: String,
        success: bool,
        timestamp: DateTime<Utc>,
    },

    /// Loop iteration completed
    IterationCompleted {
        session_id: String,
        iteration: usize,
        continue_loop: bool,
        timestamp: DateTime<Utc>,
    },

    /// Loop completed successfully
    LoopCompleted {
        session_id: String,
        total_iterations: usize,
        timestamp: DateTime<Utc>,
    },

    /// Loop failed with error
    LoopError {
        session_id: String,
        error: String,
        timestamp: DateTime<Utc>,
    },
}

impl LoopEvent {
    /// Create a loop started event
    pub fn loop_started(session_id: impl Into<String>) -> Self {
        LoopEvent::LoopStarted {
            session_id: session_id.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create an iteration started event
    pub fn iteration_started(session_id: impl Into<String>, iteration: usize) -> Self {
        LoopEvent::IterationStarted {
            session_id: session_id.into(),
            iteration,
            timestamp: Utc::now(),
        }
    }

    /// Create an LLM call started event
    pub fn llm_call_started(session_id: impl Into<String>, iteration: usize) -> Self {
        LoopEvent::LlmCallStarted {
            session_id: session_id.into(),
            iteration,
            timestamp: Utc::now(),
        }
    }

    /// Create a text delta event
    pub fn text_delta(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        LoopEvent::TextDelta {
            session_id: session_id.into(),
            message_id: message_id.into(),
            delta: delta.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create an LLM call completed event
    pub fn llm_call_completed(
        session_id: impl Into<String>,
        iteration: usize,
        has_tool_calls: bool,
    ) -> Self {
        LoopEvent::LlmCallCompleted {
            session_id: session_id.into(),
            iteration,
            has_tool_calls,
            timestamp: Utc::now(),
        }
    }

    /// Create a tool execution started event
    pub fn tool_started(
        session_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
    ) -> Self {
        LoopEvent::ToolExecutionStarted {
            session_id: session_id.into(),
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a tool execution completed event
    pub fn tool_completed(
        session_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        success: bool,
    ) -> Self {
        LoopEvent::ToolExecutionCompleted {
            session_id: session_id.into(),
            tool_call_id: tool_call_id.into(),
            success,
            timestamp: Utc::now(),
        }
    }

    /// Create an iteration completed event
    pub fn iteration_completed(
        session_id: impl Into<String>,
        iteration: usize,
        continue_loop: bool,
    ) -> Self {
        LoopEvent::IterationCompleted {
            session_id: session_id.into(),
            iteration,
            continue_loop,
            timestamp: Utc::now(),
        }
    }

    /// Create a loop completed event
    pub fn loop_completed(session_id: impl Into<String>, total_iterations: usize) -> Self {
        LoopEvent::LoopCompleted {
            session_id: session_id.into(),
            total_iterations,
            timestamp: Utc::now(),
        }
    }

    /// Create a loop error event
    pub fn loop_error(session_id: impl Into<String>, error: impl Into<String>) -> Self {
        LoopEvent::LoopError {
            session_id: session_id.into(),
            error: error.into(),
            timestamp: Utc::now(),
        }
    }

    /// Wrap an AG-UI event
    pub fn ag_ui(event: AgUiEvent) -> Self {
        LoopEvent::AgUi(event)
    }

    /// Get the session ID for this event
    pub fn session_id(&self) -> &str {
        match self {
            LoopEvent::AgUi(e) => match e {
                AgUiEvent::RunStarted(e) => &e.thread_id,
                AgUiEvent::RunFinished(e) => &e.thread_id,
                AgUiEvent::RunError(_) => "",
                AgUiEvent::StepStarted(_) => "",
                AgUiEvent::StepFinished(_) => "",
                AgUiEvent::TextMessageStart(e) => &e.message_id,
                AgUiEvent::TextMessageContent(e) => &e.message_id,
                AgUiEvent::TextMessageEnd(e) => &e.message_id,
                AgUiEvent::ToolCallStart(e) => &e.tool_call_id,
                AgUiEvent::ToolCallArgs(e) => &e.tool_call_id,
                AgUiEvent::ToolCallEnd(e) => &e.tool_call_id,
                AgUiEvent::ToolCallResult(e) => &e.tool_call_id,
                AgUiEvent::StateSnapshot(_) => "",
                AgUiEvent::StateDelta(_) => "",
                AgUiEvent::MessagesSnapshot(_) => "",
                AgUiEvent::Custom(_) => "",
            },
            LoopEvent::LoopStarted { session_id, .. } => session_id,
            LoopEvent::IterationStarted { session_id, .. } => session_id,
            LoopEvent::LlmCallStarted { session_id, .. } => session_id,
            LoopEvent::TextDelta { session_id, .. } => session_id,
            LoopEvent::LlmCallCompleted { session_id, .. } => session_id,
            LoopEvent::ToolExecutionStarted { session_id, .. } => session_id,
            LoopEvent::ToolExecutionCompleted { session_id, .. } => session_id,
            LoopEvent::IterationCompleted { session_id, .. } => session_id,
            LoopEvent::LoopCompleted { session_id, .. } => session_id,
            LoopEvent::LoopError { session_id, .. } => session_id,
        }
    }
}
