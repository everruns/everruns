//! Host-boundary fakes: assertions observe dispatch, atomic commit, and redaction.
use async_trait::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result},
    runtime::{credential_services::*, resource_access::*, *},
    typed_id::SessionId,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;
#[derive(Default)]
pub(crate) struct Credentials {
    pub values: Mutex<Vec<ResolvedCredential>>,
    pub consent: AtomicBool,
    pub fail_commit: AtomicBool,
    pub commits: Mutex<Vec<(Vec<CapturedCredential>, Value)>>,
    pub revoked: AtomicBool,
}
#[async_trait]
impl CredentialService for Credentials {
    async fn list(
        &self,
        _: CredentialInvocation,
        _: CredentialUsage,
    ) -> Result<Vec<CredentialInfo>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.info.clone())
            .collect())
    }
    async fn resolve(
        &self,
        _: CredentialInvocation,
        _: CredentialUsage,
        names: Vec<String>,
    ) -> Result<Vec<ResolvedCredential>> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(AgentLoopError::store("revoked"));
        }
        names
            .into_iter()
            .map(|n| {
                self.values
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|s| s.info.name == n)
                    .cloned()
                    .ok_or_else(|| AgentLoopError::store("unavailable"))
            })
            .collect()
    }
    async fn prepare_capture(
        &self,
        _: CredentialInvocation,
        plan: CapturePlan,
    ) -> Result<CapturePreparation> {
        Ok(if self.consent.load(Ordering::SeqCst) {
            CapturePreparation::ConsentRequired {
                ticket: Uuid::now_v7(),
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
                summary: serde_json::json!({"credentials":plan.captures}),
            }
        } else {
            CapturePreparation::Ready {
                ticket: Uuid::now_v7(),
            }
        })
    }
    async fn commit_capture(
        &self,
        _: CredentialInvocation,
        _: Uuid,
        values: Vec<CapturedCredential>,
        result: Value,
    ) -> Result<()> {
        if self.fail_commit.load(Ordering::SeqCst) {
            return Err(AgentLoopError::store("store unavailable"));
        }
        self.commits.lock().unwrap().push((values, result));
        Ok(())
    }
}
pub(crate) struct Store(pub Arc<Credentials>);
#[async_trait]
impl SessionStorageStore for Store {
    fn credential_service(&self) -> Option<Arc<dyn CredentialService>> {
        Some(self.0.clone())
    }
    async fn set_value(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
    async fn get_value(&self, _: SessionId, _: &str) -> Result<Option<String>> {
        unreachable!()
    }
    async fn delete_value(&self, _: SessionId, _: &str) -> Result<bool> {
        unreachable!()
    }
    async fn list_keys(&self, _: SessionId) -> Result<Vec<KeyInfo>> {
        unreachable!()
    }
    async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
        unreachable!()
    }
    async fn get_secret(&self, _: SessionId, _: &str) -> Result<Option<String>> {
        unreachable!()
    }
    async fn delete_secret(&self, _: SessionId, _: &str) -> Result<bool> {
        unreachable!()
    }
    async fn list_secrets(&self, _: SessionId) -> Result<Vec<SecretInfo>> {
        unreachable!()
    }
}
pub(crate) struct Http {
    pub seen: Mutex<Vec<EgressRequest>>,
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub chunks: Vec<Vec<u8>>,
    pub interrupted: bool,
}
impl Http {
    pub fn json(body: &str) -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            status: 200,
            headers: BTreeMap::from([("content-type".into(), "application/json".into())]),
            chunks: vec![body.as_bytes().to_vec()],
            interrupted: false,
        }
    }
}
#[async_trait]
impl EgressService for Http {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.seen.lock().unwrap().push(request);
        Ok(EgressResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.chunks.concat(),
        })
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        self.seen.lock().unwrap().push(request);
        let mut chunks: Vec<_> = self.chunks.iter().cloned().map(Ok).collect();
        if self.interrupted {
            chunks.push(Err(EgressError::InvalidRequest("interrupted".into())));
        }
        Ok(EgressStreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::pin(futures::stream::iter(chunks)),
        })
    }
}
pub(crate) fn context(service: Arc<Credentials>, http: Arc<Http>) -> ToolContext {
    let session = SessionId::new();
    let mut ctx = ToolContext::new(session)
        .with_storage_store_arc(Arc::new(Store(service)))
        .with_extension(Arc::new(CredentialInvocation {
            session_id: session,
            input_message_id: Uuid::now_v7(),
        }));
    ctx.egress_service = Some(http);
    ctx.tool_call_id = Some("secure-fetch".into());
    ctx
}
