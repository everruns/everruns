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
    fn next_exec_preserves_lineage_and_workspace_and_changes_execution_id() {
        for workspace in [None, Some(WorkspaceId::from_seed(5))] {
            let mut context = ExecutionContext::new(
                SessionId::from_seed(1),
                TurnId::from_seed(2),
                MessageId::from_seed(3),
            );
            assert_eq!(context.workspace_id, None);
            if let Some(workspace) = workspace {
                context = context.with_workspace_id(workspace);
            }
            let next = context.next_exec();
            assert_eq!(next.session_id, SessionId::from_seed(1));
            assert_eq!(next.turn_id, TurnId::from_seed(2));
            assert_eq!(next.input_message_id, MessageId::from_seed(3));
            assert_eq!(next.workspace_id, workspace);
            assert_ne!(next.exec_id, context.exec_id);
            assert_ne!(next.next_exec().exec_id, next.exec_id);
        }
    }

    #[test]
    fn wire_shape_preserves_literal_identity_and_missing_workspace_default() {
        let wire = serde_json::json!({"session_id":"session_00000000000000000000000000000001","turn_id":"turn_00000000000000000000000000000002","input_message_id":"message_00000000000000000000000000000003","exec_id":"exec_00000000000000000000000000000004"});
        let parsed: ExecutionContext = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(parsed.workspace_id, None);
        assert_eq!(parsed.session_id, SessionId::from_seed(1));
        assert_eq!(parsed.turn_id, TurnId::from_seed(2));
        assert_eq!(parsed.input_message_id, MessageId::from_seed(3));
        assert_eq!(parsed.exec_id, ExecId::from_seed(4));
        let mut expected = wire;
        expected["workspace_id"] = serde_json::Value::Null;
        assert_eq!(serde_json::to_value(&parsed).unwrap(), expected);
        let attached = parsed
            .with_workspace_id(WorkspaceId::from_seed(5))
            .with_workspace_id(WorkspaceId::from_seed(6));
        expected["workspace_id"] = serde_json::json!("wsp_00000000000000000000000000000006");
        assert_eq!(serde_json::to_value(&attached).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<ExecutionContext>(expected)
                .unwrap()
                .workspace_id,
            Some(WorkspaceId::from_seed(6))
        );
    }
}
