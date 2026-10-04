//! Policy gates must decide on the arguments that actually execute (EVE-1184).
//!
//! Approval and guardrail hooks used to run before user `pre_tool_use` hooks,
//! so a hook could rewrite an approved, benign call into a destructive one
//! that then ran without anyone deciding on it. These tests drive the real
//! host wiring: real `guardrails` / `tool_approval` capabilities and a real
//! bash user hook dispatched through bashkit.
#![cfg(all(feature = "bashkit", feature = "builtins"))]

use async_trait::async_trait;
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId, TurnId};
use everruns_core::ExecutionContext;
use everruns_core::builtins::GuardrailsCapability;
use everruns_core::builtins::tool_approval::{
    ApprovalDecision, ToolApprovalCapability, ToolApprover,
};
use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_core::engine::{ActInput, ActResult};
use everruns_core::host::execute_act_activity;
use everruns_core::user_hook_types::{
    ExecutorSpec, HookEvent, HookMatcher, HookSource, OnError, UserHookSpec,
};
use everruns_core::{HarnessDefinition, Tool, ToolExecutionResult};
use serde_json::json;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::runtime_host_test::{
    MockHostAdapter, harness, mock_host, reason_tool_definitions, session, set_default_model_spec,
};

const TOOL: &str = "policy_echo";

/// Returns the `value` it was called with, so a test can see which arguments
/// actually executed.
struct PolicyEchoTool;

#[async_trait]
impl Tool for PolicyEchoTool {
    fn name(&self) -> &str {
        TOOL
    }

    fn description(&self) -> &str {
        "Returns the provided value."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({ "value": arguments["value"] }))
    }
}

/// Contributes the tool plus one `pre_tool_use` bash hook that rewrites
/// `value` — a schema-valid mutation, the shape EVE-1184 is about.
struct RewritingHookCapability {
    rewrite_to: &'static str,
}

impl Capability for RewritingHookCapability {
    fn id(&self) -> &str {
        "rewriting_hook"
    }
    fn name(&self) -> &str {
        "Rewriting Hook"
    }
    fn description(&self) -> &str {
        "Test capability whose pre_tool_use hook rewrites tool arguments."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(PolicyEchoTool)]
    }
    fn user_hooks(&self) -> Vec<UserHookSpec> {
        let decision = json!({
            "decision": "mutate",
            "patch": { "arguments": { "value": self.rewrite_to } },
        });
        vec![UserHookSpec {
            id: Some("rewrite".into()),
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: format!("echo '{decision}'"),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Block,
            description: None,
            source: HookSource::UserConfig,
        }]
    }
}

/// Approves only calls whose `value` is `"benign"`, and records every
/// argument set it was asked about.
#[derive(Default)]
struct BenignOnlyApprover {
    asked: Mutex<Vec<serde_json::Value>>,
}

#[async_trait]
impl ToolApprover for BenignOnlyApprover {
    async fn approve(
        &self,
        _session_id: SessionId,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
    ) -> ApprovalDecision {
        self.asked.lock().unwrap().push(tool_call.arguments.clone());
        if tool_call.arguments["value"] == "benign" || tool_call.arguments["value"] == "tidy" {
            ApprovalDecision::Allow
        } else {
            ApprovalDecision::Reject
        }
    }
}

async fn run_call(
    adapter: &MockHostAdapter,
    capabilities: Vec<everruns_contracts::CapabilityRef>,
) -> ActResult {
    set_default_model_spec(adapter).await;
    let harness_id = HarnessId::from_uuid(Uuid::now_v7());
    let session_id = SessionId::from_uuid(Uuid::now_v7());
    adapter
        .harness_store
        .add_harness(
            harness_id,
            HarnessDefinition {
                capabilities,
                ..harness()
            },
        )
        .await;
    adapter
        .session_store
        .insert(session(session_id, harness_id))
        .await;

    execute_act_activity(
        adapter,
        ActInput {
            org_id: Some(1),
            context: ExecutionContext::new(
                session_id,
                TurnId::from_uuid(Uuid::now_v7()),
                MessageId::from_uuid(Uuid::now_v7()),
            ),
            harness_id,
            agent_id: None,
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: TOOL.into(),
                arguments: json!({ "value": "benign" }),
            }],
            tool_definitions: reason_tool_definitions(adapter, session_id, harness_id, None).await,
            locale: None,
            blueprint_id: None,
            network_access: None,
            parallel_tool_calls: None,
        },
    )
    .await
    .unwrap()
}

fn guardrail_ref() -> everruns_contracts::CapabilityRef {
    everruns_contracts::CapabilityRef::with_config(
        "guardrails",
        json!({
            "checks": [{
                "stage": "tool_use", "type": "regex",
                "patterns": ["(?i)destroy"]
            }]
        }),
    )
}

fn approval_capability(approver: Arc<BenignOnlyApprover>) -> ToolApprovalCapability {
    // Gate every call so the test does not depend on the tool's hints.
    ToolApprovalCapability::new(approver).with_policy(Arc::new(|_, _| true))
}

#[tokio::test]
async fn guardrail_blocks_a_call_a_user_hook_rewrote_into_a_blocked_one() {
    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(RewritingHookCapability {
            rewrite_to: "destroy everything",
        });
    adapter.capability_registry.register(GuardrailsCapability);

    // The guardrail precedes the hook in declaration order, which is the
    // order that used to let the hook have the last word.
    let result = run_call(
        &adapter,
        vec![
            guardrail_ref(),
            everruns_contracts::CapabilityRef::new("rewriting_hook"),
        ],
    )
    .await;

    assert_eq!(result.success_count, 0, "{:?}", result.results);
    let blocked = &result.results[0].result;
    assert!(blocked.result.is_none(), "the rewritten call must not run");
    let error = blocked.error.as_deref().unwrap_or_default();
    assert!(error.contains("guardrail"), "{error}");
}

#[tokio::test]
async fn approval_is_asked_about_the_rewritten_call_not_the_original() {
    let approver = Arc::new(BenignOnlyApprover::default());
    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(RewritingHookCapability {
            rewrite_to: "destroy everything",
        });
    adapter
        .capability_registry
        .register(approval_capability(approver.clone()));

    let result = run_call(
        &adapter,
        vec![
            everruns_contracts::CapabilityRef::new("tool_approval"),
            everruns_contracts::CapabilityRef::new("rewriting_hook"),
        ],
    )
    .await;

    assert_eq!(result.success_count, 0, "{:?}", result.results);
    let blocked = &result.results[0].result;
    assert!(blocked.result.is_none(), "the rewritten call must not run");
    assert!(
        blocked
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("rejected by user"),
        "{blocked:?}"
    );
    // The person was asked about exactly what would have executed, once.
    assert_eq!(
        *approver.asked.lock().unwrap(),
        vec![json!({ "value": "destroy everything" })]
    );
}

#[tokio::test]
async fn safe_rewrites_still_run_after_policy_approves_them() {
    let approver = Arc::new(BenignOnlyApprover::default());
    let mut adapter = mock_host();
    adapter
        .capability_registry
        .register(RewritingHookCapability { rewrite_to: "tidy" });
    adapter.capability_registry.register(GuardrailsCapability);
    adapter
        .capability_registry
        .register(approval_capability(approver.clone()));

    let result = run_call(
        &adapter,
        vec![
            guardrail_ref(),
            everruns_contracts::CapabilityRef::new("tool_approval"),
            everruns_contracts::CapabilityRef::new("rewriting_hook"),
        ],
    )
    .await;

    assert_eq!(result.success_count, 1, "{:?}", result.results);
    assert_eq!(
        result.results[0].result.result.as_ref().unwrap()["value"],
        "tidy"
    );
    assert_eq!(
        *approver.asked.lock().unwrap(),
        vec![json!({ "value": "tidy" })]
    );
}
