use super::*;

impl GrpcClient {
    pub async fn invoke_agent_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
        invocation_id: uuid::Uuid,
    ) -> Result<serde_json::Value> {
        let mut client = self.inner.client();
        let response = client
            .invoke_agent_trigger(proto::InvokeAgentTriggerRequest {
                org_id,
                agent_id: agent_id.to_string(),
                trigger_id: trigger_id.to_string(),
                invocation_id: invocation_id.to_string(),
            })
            .await
            .map_err(grpc_status_to_error)?;

        let response = response.into_inner();
        Ok(serde_json::json!({
            "session_id": response.session_id,
            "created_session": response.created_session,
        }))
    }
}
