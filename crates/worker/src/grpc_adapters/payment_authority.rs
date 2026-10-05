//! Machine payments forwarded to the control-plane payment authority.

use super::*;

#[async_trait]
impl crate::core::tool_execution::PaymentAuthority for GrpcPaymentAuthority {
    fn for_execution(
        &self,
        id: Uuid,
    ) -> Option<Arc<dyn crate::core::tool_execution::PaymentAuthority>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: crate::core::payment::MachinePaymentRequest,
    ) -> everruns_contracts::error::Result<crate::core::payment::MachinePaymentResponse> {
        let mut client = self.client.inner.client();
        let proto_request = proto::ExecuteMachinePaymentRequest {
            org_id: self.org_id,
            session_id: session_id.to_string(),
            agent_id: self.agent_id.clone(),
            capability: request.capability,
            operation: request.operation,
            method: request.method.to_string(),
            url: request.url,
            body: request
                .body
                .as_ref()
                .map(everruns_internal_protocol::json_to_proto_value),
            max_amount_usd: request.max_amount_usd,
            rail_preference: request
                .rail_preference
                .iter()
                .map(ToString::to_string)
                .collect(),
            metadata: Some(everruns_internal_protocol::json_to_proto_value(
                &request.metadata,
            )),
            input_message_id: self.input_message_id.map(uuid_to_proto),
        };
        let response = client
            .execute_machine_payment(proto_request)
            .await
            .map_err(grpc_status_to_error)?
            .into_inner();

        let attempt_id = response
            .attempt_id
            .as_deref()
            .map(everruns_contracts::typed_id::PaymentAttemptId::parse)
            .transpose()
            .map_err(|error| {
                AgentLoopError::store(format!("Invalid payment attempt id: {error}"))
            })?;
        let rail = response
            .rail
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(AgentLoopError::config)?;
        let body = response
            .response
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or(serde_json::Value::Null);
        let receipt = response
            .receipt
            .as_ref()
            .map(everruns_internal_protocol::proto_value_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        Ok(crate::core::payment::MachinePaymentResponse {
            attempt_id,
            amount_usd: response.amount_usd,
            rail,
            response: body,
            receipt,
        })
    }
}
