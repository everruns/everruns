//! CPU observation stays attached to the task that actually invokes the tool.
use super::*;
use crate::engine::{
    events::EventData,
    typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId},
};
use crate::tool_context::ToolContext;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Clock(AtomicU64);
impl crate::engine::ThreadCpuClock for Clock {
    fn thread_cpu_time_ns(&self) -> Option<u64> {
        Some(self.0.fetch_add(7, Ordering::Relaxed))
    }
}
struct Executor(u8);
#[async_trait]
impl ToolExecutor for Executor {
    async fn execute(
        &self,
        call: &ToolCall,
        _: &ToolDefinition,
    ) -> crate::engine::error::Result<ToolResult> {
        tokio::task::yield_now().await;
        if self.0 == 1 {
            return Err(crate::engine::error::AgentLoopError::tool(
                "fixture failure",
            ));
        }
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            result: Some(json!({})),
            error: (self.0 == 2).then(|| "tool error".to_string()),
            details: None,
            images: None,
            connection_required: None,
            raw_output: None,
        })
    }
}
struct Block;
#[async_trait]
impl act_hooks::PreToolUseHook for Block {
    async fn before_exec(
        &self,
        call: ToolCall,
        _: &ToolDefinition,
        _: &ToolContext,
    ) -> act_hooks::PreToolUseDecision {
        act_hooks::PreToolUseDecision::Block {
            tool_call: call,
            reason: "fixture block".into(),
            user_message: None,
        }
    }
}

async fn completed(cpu_bound: bool, outcome: u8, observe: bool, block: bool) -> ToolCompletedData {
    let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
    let mut atom = ActAtom::new(Executor(outcome), emitter.clone());
    if observe {
        atom = atom.with_thread_cpu_clock(Some(Arc::new(Clock(AtomicU64::new(100)))));
    }
    if block {
        atom = atom.with_pre_tool_hooks(vec![Arc::new(Block)]);
    }
    atom.execute(ActInput {
        org_id: Some(1),
        context: ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "cpu_fixture".into(),
            name: "fixture".into(),
            arguments: json!({}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(everruns_contracts::BuiltinTool {
            name: "fixture".into(),
            display_name: None,
            description: "finite fixture".into(),
            parameters: json!({"type":"object"}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: crate::engine::tool_types::ToolHints::default().with_cpu_bound(cpu_bound),
            full_parameters: None,
        })],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    })
    .await
    .unwrap();
    emitter
        .events()
        .await
        .into_iter()
        .find_map(|e| match e.data {
            EventData::ToolCompleted(data) => Some(data),
            _ => None,
        })
        .expect("tool completion")
}

#[tokio::test]
async fn ordinary_and_spawned_tools_record_cpu_on_success_and_failure() {
    for cpu_bound in [false, true] {
        for outcome in 0..3 {
            let data = completed(cpu_bound, outcome, true, false).await;
            assert_eq!(data.worker_thread_cpu_ns, Some(14));
            assert!(data.duration_ms.is_some(), "wall duration remains separate");
            assert_eq!(data.success, outcome == 0);
        }
    }
}

#[tokio::test]
async fn unobserved_or_uninvoked_execution_does_not_invent_zero_cpu() {
    assert_eq!(
        completed(false, 0, false, false).await.worker_thread_cpu_ns,
        None
    );
    assert_eq!(
        completed(true, 0, true, true).await.worker_thread_cpu_ns,
        None
    );
}
