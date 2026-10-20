use super::{InternalCommandTransport, call, call_secret};
use crate::core::dreaming::{DreamMemoryScope, DreamStateRow, DreamingStore};
use async_trait::async_trait;
use everruns_contracts::{error::Result, typed_id::SessionId};
use serde_json::json;
use std::sync::Arc;
/// Scoped transport factory; due() uses only the internal worker authority.
pub struct CommandDreamingStore {
    pub transport: Arc<dyn Fn(i64) -> Arc<dyn InternalCommandTransport> + Send + Sync>,
}
#[async_trait]
impl DreamingStore for CommandDreamingStore {
    async fn get(&self, org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        call_secret(
            self.transport.as_ref()(org).as_ref(),
            "Read dreaming state",
            "worker_dreaming_state",
            json!({"operation":"get","key":key}),
        )
        .await
    }
    async fn compare_exchange(
        &self,
        org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        session_id: Option<SessionId>,
        due_at: Option<i64>,
    ) -> Result<bool> {
        call(self.transport.as_ref()(org).as_ref(),"Commit dreaming state","worker_dreaming_state",json!({"operation":"cas","key":key,"expected":expected,"value":value,"session_id":session_id,"due_at":due_at})).await
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        call(
            self.transport.as_ref()(1).as_ref(),
            "Find dreaming work",
            "worker_dreaming_state",
            json!({"operation":"due","now":now,"limit":limit}),
        )
        .await
    }
}
pub async fn memory_scope(
    transport: &dyn InternalCommandTransport,
    session: SessionId,
    mount: &str,
) -> Result<Option<DreamMemoryScope>> {
    call(
        transport,
        "Authorize dreaming Memory",
        "worker_dreaming_state",
        json!({"operation":"scope","session_id":session,"mount":mount}),
    )
    .await
}

pub async fn create_artifact(
    transport: &dyn InternalCommandTransport,
    session: SessionId,
    path: &str,
    content: &str,
) -> Result<bool> {
    call(
        transport,
        "Create dreaming artifact",
        "worker_dreaming_state",
        json!({"operation":"create_artifact","session_id":session,"key":path,"value":content}),
    )
    .await
}
