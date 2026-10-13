//! Transport-neutral context shared by execution phases and emitted events.

use crate::typed_id::{ExecId, MessageId, SessionId, TurnId, WorkspaceId};
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
    /// The agent message whose tool calls this phase runs (an act phase
    /// only); it keys each call's idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls_message_id: Option<MessageId>,
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
            tool_calls_message_id: None,
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
            tool_calls_message_id: None,
        }
    }

    /// Attach the agent message whose tool calls this act phase runs.
    pub fn with_tool_calls_message(mut self, message_id: Option<MessageId>) -> Self {
        self.tool_calls_message_id = message_id;
        self
    }

    /// The idempotency key for one tool call of this act phase, passed to
    /// tools that can honor one (MCP `_meta`); `None` when the phase does not
    /// know the message holding its calls.
    ///
    /// Decision: a UUIDv5 over the session, that message and the call id.
    /// Provider call ids alone repeat (Gemini numbers them `call_0`, `call_1`
    /// afresh every step), so a gateway keyed on them would replay an earlier
    /// call's result for a new one. The message id makes the key unique per
    /// call, and resume rebuilds the same message id from the log, so a call
    /// run again after a crash carries the key its first attempt carried.
    pub fn tool_call_idempotency_key(&self, tool_call_id: &str) -> Option<String> {
        let message = self.tool_calls_message_id?;
        let name = format!("{}/{}", message.uuid(), tool_call_id);
        Some(uuid::Uuid::new_v5(&self.session_id.uuid(), name.as_bytes()).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_idempotency_keys_are_stable_and_unique_per_message_and_call() {
        let context = |message: Option<u128>| {
            ExecutionContext::new(
                SessionId::from_seed(1),
                TurnId::from_seed(2),
                MessageId::from_seed(3),
            )
            .with_tool_calls_message(message.map(MessageId::from_seed))
        };
        let key = |message, call| context(message).tool_call_idempotency_key(call);
        assert_eq!(key(None, "call_0"), None);
        // Stable across executions (a resumed act gets a new exec id).
        assert_eq!(key(Some(4), "call_0"), key(Some(4), "call_0"));
        // A repeated provider call id in a later step gets a new key.
        assert_ne!(key(Some(4), "call_0"), key(Some(5), "call_0"));
        assert_ne!(key(Some(4), "call_0"), key(Some(4), "call_1"));
        // The next phase does not inherit the act's message.
        assert_eq!(context(Some(4)).next_exec().tool_calls_message_id, None);
    }

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
