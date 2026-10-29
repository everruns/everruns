use super::*;
use crate::http_transport::{HttpError, HttpRequest, HttpResponse, HttpTransport};
use crate::runtime_provider::{ProviderAuth, ProviderAuthRequest};
use bytes::Bytes;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

mod refusals;

fn policy() -> TextDispatchPolicy {
    TextDispatchPolicy {
        profile_id: "local-fixture-only".into(),
        model: "openai/fixture-model".into(),
        context_window_tokens: 4096,
        max_output_tokens: 128,
        prompt_usd_per_million: 1,
        completion_usd_per_million: 3,
        max_request_bytes: 4096,
        max_response_bytes: 4096,
        timeout: crate::rt::Duration::from_secs(1),
    }
}
fn request() -> TextRequest {
    TextRequest {
        messages: vec![TextMessage {
            role: TextRole::User,
            content: "private prompt".into(),
        }],
        max_output_tokens: 32,
    }
}
fn response() -> Value {
    json!({"id":"gen-fixture", "model":"openai/fixture-model", "service_tier":"default",
        "openrouter_metadata":{"attempt":1,"requested":"openai/fixture-model","is_byok":false,
            "endpoints":{"available":[{"provider":"OpenAI","model":"openai/fixture-model","selected":true}]}},
        "usage":{"prompt_tokens":10,"completion_tokens":5,"cost":0.00001},
        "choices":[{"message":{"role":"assistant","content":"private answer"},"finish_reason":"stop"}]})
}
#[derive(Default)]
struct State {
    hash: Option<[u8; 32]>,
    started: bool,
    held: bool,
    revoked: bool,
    revoke_after_reserve: bool,
    fail_record: bool,
    records: Vec<(
        Uuid,
        String,
        String,
        String,
        UsageObservation,
        Option<ReceiptReview>,
    )>,
}
#[derive(Clone)]
struct Host {
    state: Arc<Mutex<State>>,
    generation: Uuid,
    auth_calls: Arc<AtomicUsize>,
    endpoint: String,
}
impl Host {
    fn new() -> Self {
        Self {
            state: Arc::default(),
            generation: Uuid::new_v4(),
            auth_calls: Arc::default(),
            endpoint: OPENROUTER_API.into(),
        }
    }
}
struct Auth(Arc<AtomicUsize>);
#[async_trait]
impl ProviderAuth for Auth {
    async fn headers(
        &self,
        _: ProviderAuthRequest<'_>,
    ) -> crate::error::Result<Vec<(String, String)>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(vec![(
            "authorization".into(),
            format!("Bearer {}", Uuid::new_v4()),
        )])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
#[async_trait]
impl DispatchHost for Host {
    type Permit = Self;
    async fn reserve(&self, plan: &DispatchPlan) -> Result<Self, DispatchError> {
        let mut state = self.state.lock().unwrap();
        if state.hash.is_some_and(|hash| hash != plan.request_hash()) {
            return Err(DispatchError::Refused);
        }
        if state.hash.is_none() {
            state.hash = Some(plan.request_hash());
            state.held = true;
        }
        state.revoked |= state.revoke_after_reserve;
        Ok(self.clone())
    }
}
#[async_trait]
impl DispatchPermit for Host {
    async fn start(
        &mut self,
        plan: &DispatchPlan,
    ) -> Result<(DispatchIdentity, ProviderEndpoint), DispatchError> {
        let mut state = self.state.lock().unwrap();
        if state.started || state.revoked || state.hash != Some(plan.request_hash()) {
            return Err(DispatchError::Refused);
        }
        state.started = true;
        Ok((
            DispatchIdentity {
                generation_id: self.generation,
                organization: "host-org".into(),
                session_id: None,
                supplier_account: "host-account".into(),
                provider_id: crate::typed_id::ProviderId::from_uuid(self.generation),
                started_at: crate::rt::utc_now(),
            },
            ProviderEndpoint::from_parts(&self.endpoint, Auth(self.auth_calls.clone())),
        ))
    }
    async fn record(&mut self, receipt: &SupplierReceipt) -> Result<(), DispatchError> {
        let mut state = self.state.lock().unwrap();
        assert_eq!(state.hash, Some(receipt.request_hash()));
        assert_eq!(receipt.identity().generation_id, self.generation);
        if state.fail_record {
            return Err(DispatchError::Refused);
        }
        state.records.push((
            receipt.identity().generation_id,
            receipt.identity().organization.clone(),
            receipt.identity().supplier_account.clone(),
            receipt.supplier_response_id().into(),
            receipt.usage().clone(),
            receipt.review(),
        ));
        state.held = receipt.review().is_some();
        Ok(())
    }
}
enum Reply {
    Json(Value),
    Status(u16),
    Failure,
    Pending,
    Oversized,
}
struct Transport {
    reply: Reply,
    requests: Mutex<Vec<HttpRequest>>,
    dropped: Arc<AtomicBool>,
    sent: tokio::sync::Notify,
}
impl Transport {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            requests: Mutex::default(),
            dropped: Arc::default(),
            sent: tokio::sync::Notify::new(),
        })
    }
    fn client(self: &Arc<Self>) -> DispatchTransport {
        DispatchTransport::injected(self.clone())
    }
}
struct PendingBody(Arc<AtomicBool>);
impl futures::Stream for PendingBody {
    type Item = Result<Bytes, HttpError>;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}
impl Drop for PendingBody {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl HttpTransport for Transport {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.requests.lock().unwrap().push(request);
        self.sent.notify_one();
        let (status, body) = match &self.reply {
            Reply::Failure => return Err(HttpError::new("fixture network failure", true)),
            Reply::Status(status) => (*status, Vec::new()),
            Reply::Pending => {
                return Ok(HttpResponse::new(
                    crate::http_transport::StatusCode::OK,
                    Default::default(),
                    Box::pin(PendingBody(self.dropped.clone())),
                ));
            }
            Reply::Oversized => (200, vec![b'x'; 4097]),
            Reply::Json(json) => (200, serde_json::to_vec(json).unwrap()),
        };
        Ok(HttpResponse::new(
            crate::http_transport::StatusCode::from_u16(status).unwrap(),
            Default::default(),
            Box::pin(futures::stream::once(async { Ok(Bytes::from(body)) })),
        ))
    }
}

#[test]
fn frozen_wire_controls_and_integer_envelope_cannot_be_replaced_by_request_fields() {
    let policy = policy();
    let plan = policy.compile(&request()).unwrap();
    assert_eq!(plan.maximum_upstream_nanos(), 4_192_000);
    assert_eq!(plan.maximum_input_tokens(), 4096);
    assert_eq!(plan.maximum_output_tokens(), 32);
    let body: Value = serde_json::from_slice(plan.body()).unwrap();
    assert_eq!(
        body["provider"],
        json!({"only":["openai"],"allow_fallbacks":false,"require_parameters":true,
        "max_price":{"prompt":1,"completion":3,"request":0,"image":0}})
    );
    assert_eq!(body["max_completion_tokens"], 32);
    assert_eq!(body["n"], 1);
    assert_eq!(body["stream"], false);
    assert_eq!(body["tools"], json!([]));
    assert_eq!(body["plugins"], json!([]));
    assert_eq!(body["reasoning"], json!({"enabled":false}));
    assert!(!format!("{plan:?}").contains("private prompt"));
    let mut changed = request();
    changed.messages[0].content.push('!');
    assert_ne!(
        plan.request_hash(),
        policy.compile(&changed).unwrap().request_hash()
    );
    let mut changed_policy = policy;
    changed_policy.context_window_tokens += 1;
    assert_ne!(
        plan.request_hash(),
        changed_policy.compile(&request()).unwrap().request_hash()
    );
}

#[tokio::test]
async fn host_identity_exact_body_and_canonical_receipt_flow_through_one_dispatch() {
    let host = Host::new();
    let transport = Transport::new(Reply::Json(response()));
    let client = transport.client();
    let result = dispatch_text(&host, &client, &policy(), &request())
        .await
        .unwrap();
    assert_eq!(result, "private answer");
    let state = host.state.lock().unwrap();
    assert!(!state.held);
    assert_eq!(state.records.len(), 1);
    assert_eq!(
        (
            &state.records[0].0,
            &state.records[0].1,
            &state.records[0].2,
            &state.records[0].3
        ),
        (
            &host.generation,
            &"host-org".into(),
            &"host-account".into(),
            &"gen-fixture".into()
        )
    );
    assert_eq!(state.records[0].4.cost_nanos, Some(10_000));
    assert_eq!(state.records[0].5, None);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri(), OPENROUTER_CHAT);
    assert_eq!(requests[0].method(), "POST");
    assert_eq!(
        requests[0].body(),
        policy().compile(&request()).unwrap().body()
    );
    assert_eq!(requests[0].headers().len(), 3);
    assert_eq!(requests[0].headers()["x-openrouter-metadata"], "enabled");
}

#[tokio::test]
async fn concurrent_or_replayed_operations_cannot_dispatch_twice() {
    let host = Host::new();
    let transport = Transport::new(Reply::Json(response()));
    let client = transport.client();
    let p = policy();
    let r = request();
    let (a, b) = tokio::join!(
        dispatch_text(&host, &client, &p, &r),
        dispatch_text(&host, &client, &p, &r)
    );
    assert!(a.is_ok() ^ b.is_ok());
    assert_eq!(a.err().or(b.err()), Some(DispatchError::Refused));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(host.auth_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        dispatch_text(&host, &client, &p, &r).await,
        Err(DispatchError::Refused)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
