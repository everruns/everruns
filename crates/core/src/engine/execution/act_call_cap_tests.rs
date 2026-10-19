//! ActAtom-level coverage for the per-act tool call cap (TM-AGENT-007).
//! Kept apart from `act_tests.rs`, which is at the file-size ceiling.

use crate::engine::execution::{ActAtom, ActInput, ExecutionContext};
use crate::engine::test_fixtures::NoopEventEmitter;
use crate::engine::tool_types::ToolCall;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use serde_json::json;

/// THREAT[TM-AGENT-007]: a response with more calls than the per-act cap runs
/// only the first `DEFAULT_MAX_TOOL_CALLS_PER_ACT`; the rest fail unexecuted.
#[tokio::test]
async fn test_act_atom_rejects_tool_calls_past_the_per_act_cap() {
    let cap = super::DEFAULT_MAX_TOOL_CALLS_PER_ACT;
    let atom = ActAtom::new(ToolRegistry::with_defaults(), NoopEventEmitter);
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: (0..cap + 2)
            .map(|i| ToolCall {
                id: format!("call_{i}"),
                name: "nonexistent_tool".to_string(),
                arguments: json!({}),
            })
            .collect(),
        tool_definitions: vec![],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    };

    let result = atom.execute(input).await.unwrap();

    assert_eq!(result.results.len(), cap + 2);
    assert_eq!(result.error_count as usize, cap + 2);
    let not_executed = result
        .results
        .iter()
        .filter(|r| {
            r.result
                .error
                .as_deref()
                .is_some_and(|e| e.starts_with("Not executed"))
        })
        .map(|r| r.tool_call.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        not_executed,
        [format!("call_{cap}"), format!("call_{}", cap + 1)]
    );
}
