//! EVE-1187: per-call binding of the payment authority to the turn input.

use super::*;
use crate::ToolExecutionResult;
use crate::engine::test_fixtures::NoopEventEmitter;
use crate::engine::tools::ToolRegistry;
use crate::engine::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use async_trait::async_trait;
use everruns_contracts::BuiltinTool;
use serde_json::json;

/// Payment authority that reports which input it was bound to (EVE-1187).
#[derive(Clone, Default)]
struct BindingProbePaymentAuthority {
    bound: Option<uuid::Uuid>,
}

#[async_trait]
impl crate::engine::tool_execution::PaymentAuthority for BindingProbePaymentAuthority {
    fn for_execution(
        &self,
        input_message_id: uuid::Uuid,
    ) -> Option<Arc<dyn crate::engine::tool_execution::PaymentAuthority>> {
        Some(Arc::new(Self {
            bound: Some(input_message_id),
        }))
    }

    async fn execute_machine_payment(
        &self,
        _session_id: SessionId,
        _request: crate::payment::MachinePaymentRequest,
    ) -> everruns_contracts::error::Result<crate::payment::MachinePaymentResponse> {
        Ok(crate::payment::MachinePaymentResponse {
            attempt_id: None,
            amount_usd: 0.0,
            rail: None,
            response: json!({ "bound": self.bound }),
            receipt: json!({}),
        })
    }
}

struct PaymentBindingProbeTool;

#[async_trait]
impl crate::engine::tools::Tool for PaymentBindingProbeTool {
    fn name(&self) -> &str {
        "payment_binding_probe"
    }

    fn description(&self) -> &str {
        "reports the input the payment authority is bound to"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("context required")
    }

    async fn execute_with_context(
        &self,
        _arguments: serde_json::Value,
        context: &crate::engine::tool_context::ToolContext,
    ) -> ToolExecutionResult {
        let Some(authority) = context.payment_authority.as_ref() else {
            return ToolExecutionResult::tool_error("no payment authority");
        };
        let response = authority
            .execute_machine_payment(
                context.session_id,
                crate::payment::MachinePaymentRequest {
                    capability: "probe".into(),
                    operation: "probe".into(),
                    method: crate::payment::PaymentMethod::Get,
                    url: "https://example.com".into(),
                    body: None,
                    max_amount_usd: 0.01,
                    rail_preference: vec![],
                    metadata: json!({}),
                },
            )
            .await
            .expect("probe authority");
        ToolExecutionResult::success(response.response)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// EVE-1187: the payment authority decides whether a human wallet is usable
/// from the input that caused the turn, so each tool call must see it bound
/// to that input rather than an unscoped session-wide authority.
#[tokio::test]
async fn test_act_atom_binds_payment_authority_to_the_turn_input() {
    let mut executor = ToolRegistry::with_defaults();
    executor.register(PaymentBindingProbeTool);
    let atom = ActAtom::new(executor, NoopEventEmitter)
        .with_payment_authority(Arc::new(BindingProbePaymentAuthority::default()));

    let input_message_id = MessageId::new();
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), input_message_id);
    let input = ActInput {
        org_id: Some(1),
        context,
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "payment_binding_probe".to_string(),
            arguments: json!({}),
        }],
        tool_definitions: vec![ToolDefinition::Builtin(BuiltinTool {
            name: "payment_binding_probe".to_string(),
            display_name: None,
            description: "reports payment binding".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
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

    assert_eq!(result.success_count, 1);
    let payload = result.results[0].result.result.as_ref().unwrap();
    assert_eq!(payload["bound"], json!(input_message_id.uuid()));
}
