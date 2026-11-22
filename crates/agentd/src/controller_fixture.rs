//! A control plane server (`injected::CONTROLLER_SERVERS`) for tests: tools
//! that answer fixed text and record each call with the token's actor,
//! behind an egress that verifies the session token, and an executor whose
//! sessions' turns have the actors a test sets. Makes no host calls.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, Weak};

use anyhow::Result;
use async_trait::async_trait;
use everruns_contracts::dataplane::Connection;
use everruns_contracts::dataplane_seal::Key;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedCapabilityDefinition, SessionActor, injected_capability_id,
};
use everruns_contracts::tools::ToolExecutionResult;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::mcp::injected_server::{
    InjectedCapabilityService, VerifiedSession, handle_injected_request,
};
use everruns_core::mcp::session_token::{SessionTokenSigner, SessionTokenVerifier};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::mcp::OrgMcp;
use crate::providers::OrgMaterial;

const KEY: Key = [7u8; 32];
const ORG: &str = "org-a";

/// One tool call: the tool, its arguments, the token's actor.
pub(crate) type Call = (String, Value, Option<SessionActor>);

struct Echo {
    name: String,
    answer: Value,
    asked: Arc<Mutex<Vec<Call>>>,
    actor: Option<SessionActor>,
}

#[async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.asked
            .lock()
            .push((self.name.clone(), arguments, self.actor.clone()));
        ToolExecutionResult::success(Value::String(self.answer.to_string()))
    }
}

struct Service {
    definition: InjectedCapabilityDefinition,
    tools: Vec<(String, Value)>,
    asked: Arc<Mutex<Vec<Call>>>,
}

#[async_trait]
impl InjectedCapabilityService for Service {
    fn definition(&self) -> &InjectedCapabilityDefinition {
        &self.definition
    }
    async fn tools(&self, session: &VerifiedSession) -> Result<Vec<Box<dyn Tool>>> {
        Ok(self
            .tools
            .iter()
            .map(|(name, answer)| -> Box<dyn Tool> {
                Box::new(Echo {
                    name: name.clone(),
                    answer: answer.clone(),
                    asked: self.asked.clone(),
                    actor: session.claims.act.clone(),
                })
            })
            .collect())
    }
    async fn tool_context(&self, session: &VerifiedSession) -> Result<ToolContext> {
        Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
    }
}

/// The control plane's endpoint: a token that does not verify is a 401.
struct Endpoint {
    name: String,
    url: String,
    service: Service,
    verifier: SessionTokenVerifier,
}

#[async_trait]
impl EgressService for Endpoint {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        assert_eq!(request.url, self.url);
        let bearer = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .and_then(|(_, value)| value.strip_prefix("Bearer "))
            .unwrap_or_default()
            .to_string();
        let Ok(claims) =
            self.verifier
                .verify(&bearer, &self.url, &injected_capability_id(&self.name))
        else {
            return Ok(EgressResponse {
                status: 401,
                headers: BTreeMap::new(),
                body: Vec::new(),
            });
        };
        let session = VerifiedSession::from_claims(claims).unwrap();
        let message: Value = serde_json::from_slice(&request.body).unwrap();
        Ok(
            match handle_injected_request(&self.service, &session, message).await {
                Some(reply) => EgressResponse {
                    status: 200,
                    headers: [("content-type".to_string(), "application/json".to_string())].into(),
                    body: serde_json::to_vec(&reply).unwrap(),
                },
                None => EgressResponse {
                    status: 202,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                },
            },
        )
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

/// An executor running sessions whose create brought the server.
pub(crate) struct Executor {
    name: String,
    url: String,
    /// Each session's open turn's person.
    pub open: Mutex<HashMap<SessionId, SessionActor>>,
    /// Who sent each message.
    pub senders: Mutex<HashMap<MessageId, String>>,
    mcp: OnceLock<Arc<OrgMcp>>,
}

#[async_trait]
impl OrgMaterial for Executor {
    fn tenant(&self, _: SessionId) -> Option<String> {
        Some(ORG.into())
    }
    async fn connection(&self, _: &str, _: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }
    async fn secret(&self, _: &str, _: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }
    fn data_plane_key(&self) -> Option<Key> {
        Some(KEY)
    }
    fn turn_actor(&self, session: SessionId) -> Option<SessionActor> {
        self.open.lock().get(&session).cloned()
    }
    fn message_actor(&self, _: SessionId, message: MessageId) -> Option<SessionActor> {
        self.senders.lock().get(&message).map(|sub| SessionActor {
            sub: sub.clone(),
            turn: Some(message.uuid().to_string()),
        })
    }
    fn controller_url(&self, _: SessionId, name: &str) -> Option<String> {
        (name == self.name).then(|| self.url.clone())
    }
    fn mcp(&self) -> Option<Arc<OrgMcp>> {
        self.mcp.get().cloned()
    }
}

/// The server `name` at `url` with `tools` (name, answer as text JSON).
pub(crate) struct Fixture {
    pub executor: Arc<Executor>,
    pub material: Weak<dyn OrgMaterial>,
    pub asked: Arc<Mutex<Vec<Call>>>,
    _kept: Arc<dyn OrgMaterial>,
}

impl Fixture {
    pub(crate) fn new(name: &str, url: &str, tools: &[(&str, Value)]) -> Self {
        let signer = SessionTokenSigner::from_pkcs8(
            everruns_contracts::dataplane_seal::SESSION_TOKEN_ISSUER,
            &everruns_contracts::dataplane_seal::session_token_pkcs8(&KEY),
        )
        .unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let endpoint = Arc::new(Endpoint {
            name: name.to_owned(),
            url: url.to_owned(),
            service: Service {
                definition: serde_json::from_value(json!({
                    "name": name,
                    "description": name,
                    "server": { "url": url },
                }))
                .unwrap(),
                tools: tools
                    .iter()
                    .map(|(tool, answer)| ((*tool).to_owned(), answer.clone()))
                    .collect(),
                asked: asked.clone(),
            },
            verifier: SessionTokenVerifier::from_jwks(&signer.jwks(), vec![signer.issuer().into()])
                .unwrap(),
        });
        let executor = Arc::new(Executor {
            name: name.to_owned(),
            url: url.to_owned(),
            open: Mutex::default(),
            senders: Mutex::default(),
            mcp: OnceLock::new(),
        });
        let mcp = Arc::new(OrgMcp::new(endpoint));
        let kept: Arc<dyn OrgMaterial> = executor.clone();
        let material = Arc::downgrade(&kept);
        mcp.bind(material.clone());
        let _ = executor.mcp.set(mcp);
        Self {
            executor,
            material,
            asked,
            _kept: kept,
        }
    }
}
