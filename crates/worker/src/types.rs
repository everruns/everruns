// Workflow and activity type definitions
//
// These types define the inputs/outputs for workflows in the
// durable execution model. All types must be serializable for
// persistence.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Result from processing a workflow activation
#[derive(Debug)]
pub enum WorkflowAction {
    /// Schedule an activity
    ScheduleActivity {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    },
    /// Complete the workflow successfully
    CompleteWorkflow { result: Option<serde_json::Value> },
    /// Fail the workflow
    FailWorkflow { reason: String },
    /// No action needed (waiting for activity result)
    None,
}

/// Input for the Session workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWorkflowInput {
    pub session_id: Uuid,
    pub agent_id: Uuid,
}

/// Output for the Session workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWorkflowOutput {
    pub session_id: Uuid,
    pub status: String,
    pub iterations: u32,
}

/// Constants for workflow names
pub mod workflow_names {
    pub const TURN_WORKFLOW: &str = "turn_workflow";
}

/// Maximum number of tool calling iterations
pub const MAX_TOOL_ITERATIONS: u32 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_workflow_input_serialization() {
        let input = SessionWorkflowInput {
            session_id: Uuid::nil(),
            agent_id: Uuid::nil(),
        };

        let json = serde_json::to_string(&input).unwrap();
        let parsed: SessionWorkflowInput = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, input.session_id);
        assert_eq!(parsed.agent_id, input.agent_id);
    }

    #[test]
    fn test_session_workflow_output_serialization() {
        let output = SessionWorkflowOutput {
            session_id: Uuid::nil(),
            status: "completed".to_string(),
            iterations: 3,
        };

        let json = serde_json::to_string(&output).unwrap();
        let parsed: SessionWorkflowOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, output.session_id);
        assert_eq!(parsed.status, "completed");
        assert_eq!(parsed.iterations, 3);
    }
}
