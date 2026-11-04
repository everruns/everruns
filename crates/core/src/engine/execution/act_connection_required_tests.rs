//! A call blocked on a missing connection is a failure, not a success.
//! Kept apart from `act_tests.rs`, which is at the file-size ceiling.

use crate::ToolExecutionResult;
use crate::engine::execution::{ActAtom, ActInput, ExecutionContext};
use crate::engine::test_fixtures::NoopEventEmitter;
use crate::engine::tool_types::{ConnectionRequired, ToolCall, ToolDefinition};
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use async_trait::async_trait;
use everruns_contracts::BuiltinTool;
use serde_json::json;

struct NeedsConnectionTool;

#[async_trait]
impl crate::engine::tools::Tool for NeedsConnectionTool {
    fn name(&self) -> &str {
        "needs_connection"
    }

    fn description(&self) -> &str {
        "Needs a provider connection"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::connection_required("daytona")
    }
}

/// A call blocked on a missing connection did not run. It must be reported as
/// a failure, never as a successful empty result, while still carrying
/// `connection_required` so the connect card can be shown.
#[tokio::test]
async fn test_act_atom_reports_connection_required_as_failure() {
    let mut executor = ToolRegistry::with_defaults();
    executor.register(NeedsConnectionTool);
    let atom = ActAtom::new(executor, NoopEventEmitter);

    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "needs_connection".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
            name: "needs_connection".to_string(),
            display_name: None,
            description: "needs a connection".to_string(),
            parameters: json!({"type": "object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::engine::tool_types::ToolHints::default(),
            full_parameters: None,
        })],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    let call = &result.results[0];
    assert!(!call.success);
    assert_eq!(call.status, "error");
    assert_eq!(
        call.connection_required,
        Some(ConnectionRequired::provider_only("daytona"))
    );
    assert!(
        call.result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("did not run"))
    );
    assert_eq!(result.success_count, 0);
}
