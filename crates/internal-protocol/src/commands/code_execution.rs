//! Private, session-scoped code records over the trusted worker transport.
use super::{InternalCommandTransport, call_secret};
use async_trait::async_trait;
use everruns_contracts::{
    error::Result,
    runtime::durability::{DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult},
    typed_id::SessionId,
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

pub struct CommandCodeExecutionStore {
    pub transport: Arc<dyn InternalCommandTransport>,
    pub session: SessionId,
}

impl CommandCodeExecutionStore {
    async fn operation<R: serde::de::DeserializeOwned>(&self, mut params: Value) -> Result<R> {
        params["session_id"] = json!(self.session);
        call_secret(
            self.transport.as_ref(),
            "Private code journal",
            "worker_code_execution",
            params,
        )
        .await
    }
}

#[async_trait]
impl DurableToolResultStore for CommandCodeExecutionStore {
    async fn try_claim_tool_call(
        &self,
        turn: &str,
        call: &str,
        name: &str,
        fingerprint: &str,
    ) -> Result<ToolCallClaimResult> {
        self.operation(json!({"operation":"claim", "namespace":turn, "call_id":call, "tool_name":name, "fingerprint":fingerprint})).await
    }
    async fn settle_tool_call(
        &self,
        turn: &str,
        call: &str,
        value: Value,
        status: &str,
        token: Uuid,
    ) -> Result<bool> {
        self.operation(json!({"operation":"settle", "namespace":turn, "call_id":call, "value":value, "status":status, "token":token})).await
    }
    async fn get_tool_call_status(
        &self,
        turn: &str,
        call: &str,
    ) -> Result<Option<DurableToolCallStatus>> {
        self.operation(json!({"operation":"get", "namespace":turn, "call_id":call}))
            .await
    }
}
