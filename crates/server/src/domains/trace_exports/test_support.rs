//! Shared tracing test fixtures; independent of HTTP handlers.
use super::*;
use crate::domains::common::Ctx;
use crate::storage::{EncryptionService, StorageBackend};
use async_trait::async_trait;
use everruns_core::{
    Caller, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    OrgRole,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub(crate) fn config() -> DestinationConfig {
    DestinationConfig {
        name: "Customer collector".into(),
        provider: TraceProvider::Otlp,
        endpoint: "https://collector.example".into(),
        project_id: None,
        capture_content: false,
        capture_reasoning: false,
        enabled: true,
    }
}
pub(crate) fn encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    )
}
pub(crate) fn ctx(db: Arc<StorageBackend>, role: OrgRole) -> Ctx {
    Ctx::minimal_for_test(
        Caller {
            org_id: 1,
            org_public_id: "org_00000000000000000000000000000001".into(),
            user_id: None,
            role,
            is_platform_user: false,
            is_internal: false,
            read_only: false,
        },
        db,
        Some(encryption()),
    )
}
pub(crate) async fn destination(db: &StorageBackend, org: i64) -> Uuid {
    db.create_tracing_destination(
        org,
        &config(),
        encryption()
            .encrypt_string(&json!({"headers":{"Authorization":"Bearer CUSTOMER_KEY"}}).to_string())
            .unwrap(),
    )
    .await
    .unwrap()
}
#[derive(Default)]
pub(crate) struct Collector {
    pub(crate) requests: Mutex<Vec<EgressRequest>>,
}
#[async_trait]
impl EgressService for Collector {
    async fn send(&self, r: EgressRequest) -> EgressResult<EgressResponse> {
        self.requests.lock().unwrap().push(r);
        Ok(EgressResponse {
            status: 200,
            headers: Default::default(),
            body: vec![],
        })
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unreachable!()
    }
}
