//! Transport-neutral context shared by execution phases and emitted events.

use everruns_provider::typed_id::{ExecId, MessageId, SessionId, TurnId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Correlation and resource identity for one execution phase within a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Session that owns the turn.
    pub session_id: SessionId,
    /// Turn containing this execution phase.
    pub turn_id: TurnId,
    /// Input message that triggered the turn.
    pub input_message_id: MessageId,
    /// Unique identifier for this execution phase.
    pub exec_id: ExecId,
    /// Workspace used for virtual file operations, when explicitly attached.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
}

impl ExecutionContext {
    /// Create a context for the first execution phase in a turn.
    pub fn new(session_id: SessionId, turn_id: TurnId, input_message_id: MessageId) -> Self {
        Self {
            session_id,
            turn_id,
            input_message_id,
            exec_id: ExecId::new(),
            workspace_id: None,
        }
    }

    /// Attach the workspace addressed by this execution.
    pub fn with_workspace_id(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Create a context for the next phase while preserving turn lineage.
    pub fn next_exec(&self) -> Self {
        Self {
            session_id: self.session_id,
            turn_id: self.turn_id,
            input_message_id: self.input_message_id,
            exec_id: ExecId::new(),
            workspace_id: self.workspace_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_exec_preserves_lineage_and_changes_execution_id() {
        let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let next = context.next_exec();

        assert_eq!(next.session_id, context.session_id);
        assert_eq!(next.turn_id, context.turn_id);
        assert_eq!(next.input_message_id, context.input_message_id);
        assert_ne!(next.exec_id, context.exec_id);
    }

    #[test]
    fn serialization_round_trips() {
        let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
        let json = serde_json::to_string(&context).unwrap();
        let parsed: ExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, context.session_id);
        assert_eq!(parsed.turn_id, context.turn_id);
        assert_eq!(parsed.input_message_id, context.input_message_id);
        assert_eq!(parsed.exec_id, context.exec_id);
    }
}
