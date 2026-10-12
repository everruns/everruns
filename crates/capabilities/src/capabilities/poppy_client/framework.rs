//! The Poppy personal agent for Framework apps: `Agent::builder()
//! .capability(PoppyPersonalAgent::new(..))`.
//!
//! Decision: the app is the personal agent. It owns the signing key and
//! publishes its client metadata and JWKS at the HTTPS base URL it names
//! ([`PoppyPersonalAgent::metadata`], [`PoppyPersonalAgent::jwks`]); companies
//! fetch them from there to check its assertions. One app speaks for one
//! person, so the person's identity at each company is shared by every
//! session, and each session holds its own conversations. State lives in
//! memory: a restart is a new person to every company.
//!
//! Decision: the tools are the hosted `poppy_client` tools (same names,
//! schemas and descriptions). A Framework call has no tool call id, so each
//! message gets a fresh id and a retried turn sends a new message.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::capability::definition::{self, Handler, schemars};
use everruns_contracts::capability::{CapabilitySpec, IntoCapability};
use everruns_core::EgressService;
use everruns_core::capabilities::Capability;
use everruns_core::tools::Tool;
use serde_json::Value;

use super::client::{
    InMemoryPoppyStore, PoppyAccount, PoppyClient, PoppyError, PoppyStore, StoredConversation,
};
use super::key::PoppyKey;
use super::protocol::{PoppyAgent, client_id, client_metadata};
use super::{PoppyClientCapability, PoppyTool};

/// A Framework app as a Poppy personal agent.
#[derive(Clone)]
pub struct PoppyPersonalAgent {
    inner: Arc<Inner>,
}

struct Inner {
    base_url: String,
    key: Arc<PoppyKey>,
    agent: Arc<PoppyAgent>,
    person: Arc<InMemoryPoppyStore>,
    sessions: parking_lot::Mutex<HashMap<String, Arc<PoppyClient>>>,
}

impl PoppyPersonalAgent {
    /// A personal agent whose metadata is served at `base_url` (HTTPS, the
    /// address companies fetch `/v1/poppy/agent.json` and
    /// `/v1/poppy/jwks.json` from), signing with `key`. Requests leave
    /// directly, refusing private and loopback addresses.
    pub fn new(base_url: impl Into<String>, key: PoppyKey) -> Self {
        Self::with_egress(
            base_url,
            key,
            Arc::new(everruns_core::host::DirectEgressService::new()),
        )
    }

    /// As [`new`](Self::new), with requests sent through `egress`.
    pub fn with_egress(
        base_url: impl Into<String>,
        key: PoppyKey,
        egress: Arc<dyn EgressService>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let key = Arc::new(key);
        let agent = Arc::new(PoppyAgent::new(egress, key.clone(), client_id(&base_url)));
        Self {
            inner: Arc::new(Inner {
                base_url,
                key,
                agent,
                person: Arc::new(InMemoryPoppyStore::new()),
                sessions: parking_lot::Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The agent's `client_id`.
    pub fn client_id(&self) -> String {
        client_id(&self.inner.base_url)
    }

    /// The client metadata document to serve at `client_id`, naming the
    /// agent `client_name` to companies.
    pub fn metadata(&self, client_name: &str) -> Value {
        client_metadata(&self.inner.base_url, client_name)
    }

    /// The JWK Set to serve at the metadata's `jwks_uri`.
    pub fn jwks(&self) -> Value {
        self.inner.key.jwks()
    }

    fn client(&self, session_id: &str) -> Arc<PoppyClient> {
        self.inner
            .sessions
            .lock()
            .entry(session_id.to_string())
            .or_insert_with(|| {
                Arc::new(PoppyClient::new(
                    self.inner.agent.clone(),
                    Arc::new(SessionStore {
                        person: self.inner.person.clone(),
                        session: session_id.to_string(),
                    }),
                ))
            })
            .clone()
    }
}

/// The person's identities, with one session's conversations.
struct SessionStore {
    person: Arc<InMemoryPoppyStore>,
    session: String,
}

impl SessionStore {
    fn key(&self, domain: &str) -> String {
        format!("{}\n{domain}", self.session)
    }
}

#[async_trait]
impl PoppyStore for SessionStore {
    async fn account(&self, issuer: &str) -> Result<PoppyAccount, String> {
        self.person.account(issuer).await
    }

    async fn conversation(&self, domain: &str) -> Result<Option<StoredConversation>, String> {
        let stored = self.person.conversation(&self.key(domain)).await?;
        Ok(stored.map(|conversation| StoredConversation {
            domain: domain.to_string(),
            ..conversation
        }))
    }

    async fn save_conversation(&self, conversation: &StoredConversation) -> Result<(), String> {
        self.person
            .save_conversation(&StoredConversation {
                domain: self.key(&conversation.domain),
                ..conversation.clone()
            })
            .await
    }
}

impl IntoCapability for PoppyPersonalAgent {
    fn into_capability(self) -> CapabilitySpec {
        let capability = PoppyClientCapability;
        definition::Definition::new(capability.id(), capability.name(), capability.description())
            .instructions(super::PROMPT)
            .tool(PoppyHandler::<0>(self.clone()))
            .tool(PoppyHandler::<1>(self.clone()))
            .tool(PoppyHandler::<2>(self.clone()))
            .tool(PoppyHandler::<3>(self.clone()))
            .tool(PoppyHandler::<4>(self))
            .into()
    }
}

/// The tool behind each handler kind.
static TOOLS: [PoppyTool; 5] = [
    PoppyTool::Discover,
    PoppyTool::Send,
    PoppyTool::Read,
    PoppyTool::Handoff,
    PoppyTool::Close,
];

fn tool(kind: u8) -> &'static PoppyTool {
    &TOOLS[usize::from(kind)]
}

/// One tool's arguments, with the hosted tool's schema.
#[derive(serde::Deserialize)]
#[serde(transparent)]
struct Arguments<const KIND: u8>(Value);

impl<const KIND: u8> schemars::JsonSchema for Arguments<KIND> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("PoppyArguments{KIND}").into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        match tool(KIND).parameters_schema() {
            Value::Object(map) => map.into(),
            _ => serde_json::Map::new().into(),
        }
    }
}

struct PoppyHandler<const KIND: u8>(PoppyPersonalAgent);

#[definition::async_trait]
impl<const KIND: u8> Handler for PoppyHandler<KIND> {
    type Input = Arguments<KIND>;
    type Output = Value;
    type Error = definition::Error;

    fn name(&self) -> &str {
        Tool::name(tool(KIND))
    }

    fn description(&self) -> &str {
        Tool::description(tool(KIND))
    }

    fn display_name(&self) -> Option<&str> {
        Tool::display_name(tool(KIND))
    }

    fn hints(&self) -> definition::Hints {
        let readonly = matches!(tool(KIND), PoppyTool::Discover | PoppyTool::Read);
        definition::Hints::default()
            .readonly(readonly)
            .open_world(true)
    }

    async fn execute(
        &self,
        input: Arguments<KIND>,
        context: definition::Context,
    ) -> Result<Value, Self::Error> {
        let message_id = format!("msg_{}", uuid::Uuid::now_v7().simple());
        let call = (*tool(KIND))
            .call(&input.0, message_id)
            .map_err(|error| definition::Error::user("poppy_invalid_arguments", error))?;
        match self.0.client(context.session_id()).run(call).await {
            Ok(answer) => Ok(answer),
            Err(PoppyError::Refused(message)) => {
                Err(definition::Error::user("poppy_refused", message))
            }
            Err(PoppyError::Internal(message)) => {
                tracing::warn!(%message, "Poppy call failed");
                Err(definition::Error::user(
                    "poppy_unavailable",
                    "Poppy is unavailable right now.",
                ))
            }
        }
    }
}
