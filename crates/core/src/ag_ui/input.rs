use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ag_ui::{Message, Metadata, PROTOCOL_VERSION};

/// The request body that starts a run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAgentInput {
    pub thread_id: String,
    pub run_id: String,
    /// The consumer's protocol version; absent from a pre-1.0 consumer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    pub messages: Vec<Message>,
    /// Frontend tools the consumer will execute. Absent and empty mean the
    /// same; both are always written because 0.x producers required them.
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub context: Vec<Context>,
    /// Any JSON, forwarded to the agent untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_props: Option<Value>,
    /// Answers to the interrupts of the run this one continues.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resume: Vec<ResumeEntry>,
}

impl RunAgentInput {
    /// Declares [`PROTOCOL_VERSION`] as the consumer's version.
    ///
    /// ```
    /// use everruns_core::ag_ui::RunAgentInput;
    ///
    /// let input = RunAgentInput::default().with_protocol_version();
    /// assert_eq!(input.protocol_version.as_deref(), Some("1.0"));
    /// ```
    pub fn with_protocol_version(mut self) -> Self {
        self.protocol_version = Some(PROTOCOL_VERSION.to_owned());
        self
    }
}

/// A frontend tool definition.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// A piece of context the consumer supplies to the run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub description: String,
    pub value: String,
}

/// Something a run stopped to ask for, carried by the interrupt outcome.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interrupt {
    /// Unique within the run; a [`ResumeEntry`] answers it by this id.
    pub id: String,
    /// Open string, e.g. `"tool_approval"`.
    pub reason: String,
    /// A human-readable prompt for whoever answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The tool call an approval concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// JSON Schema of the expected answer, carried opaquely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<serde_json::Map<String, Value>>,
    /// Conventionally an ISO 8601 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_run_id: Option<String>,
}

impl Interrupt {
    pub fn new(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            reason: reason.into(),
            ..Self::default()
        }
    }
}

/// The answer to one [`Interrupt`] on a resuming run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeEntry {
    pub interrupt_id: String,
    pub status: ResumeStatus,
    /// The answer the agent asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Envelope information about the response, not part of the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeStatus {
    /// The interrupt was answered.
    Resolved,
    /// The interrupt was abandoned.
    Cancelled,
}
