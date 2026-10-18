use std::sync::{Arc, RwLock};
use std::time::Duration;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::events::{
    Event, EventContext, EventRequest, InputMessageData, ToolCompletedData,
};
use everruns_core::host::AcceptedTurnInput;
use everruns_core::host::session_actor::store::{ActorFence, LeasedActor, actor_lease_lost};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Debug)]
pub(crate) struct AuthorityRejection(pub u16);
impl std::fmt::Display for AuthorityRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "authority rejected operation ({})", self.0)
    }
}
impl std::error::Error for AuthorityRejection {}
pub(crate) fn rejected(status: u16) -> AgentLoopError {
    AgentLoopError::Internal(anyhow::Error::new(AuthorityRejection(status)))
}

/// Authority RPC supplied by a native HTTP client or an isolate-local object bridge.
#[async_trait::async_trait]
pub trait AuthorityTransport: Send + Sync + 'static {
    async fn invoke(
        &self,
        tenant: &str,
        session: SessionId,
        operation: &str,
        body: Value,
    ) -> Result<Value>;
    /// Used by native executors to reject reuse of the authority credential.
    fn credential_matches(&self, _token: &str) -> bool {
        false
    }
}

/// Client configuration for a single tenant's authority service.
#[derive(Clone)]
pub struct AuthorityClient {
    tenant: String,
    transport: Arc<dyn AuthorityTransport>,
}

impl AuthorityClient {
    /// HTTPS authority transport; loopback HTTP is available for local development.
    #[cfg(feature = "native")]
    pub fn new(base: &str, tenant: &str, token: &str) -> Result<Self> {
        Self::with_transport(
            tenant,
            Arc::new(crate::http_transport::HttpAuthorityTransport::new(
                base, token,
            )?),
        )
    }

    /// Inject an authority transport without selecting a native HTTP stack.
    pub fn with_transport(tenant: &str, transport: Arc<dyn AuthorityTransport>) -> Result<Self> {
        if tenant.is_empty()
            || tenant.len() > 128
            || !tenant
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(AgentLoopError::config("invalid authority tenant"));
        }
        Ok(Self {
            tenant: tenant.to_owned(),
            transport,
        })
    }

    /// Bind requests to one typed session identity.
    pub fn session(&self, session_id: SessionId) -> Authority {
        Authority {
            client: self.clone(),
            session_id,
            fence: Arc::new(RwLock::new(None)),
            proactive: Arc::default(),
        }
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    #[cfg(feature = "native")]
    pub(crate) fn credential_matches(&self, token: &str) -> bool {
        self.transport.credential_matches(token)
    }
}

/// Immutable executable behavior identity retained with a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionBinding {
    /// Registered agent factory key.
    pub agent: String,
    /// Build containing the executable agent behavior.
    pub build_id: String,
}

/// One claimed activation; the authority, not the executor, allocates its epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub actor: LeasedActor,
    pub binding: SessionBinding,
}

/// Per-session authority. Create a separate fenced view for every activation.
#[derive(Clone)]
pub struct Authority {
    pub(crate) client: AuthorityClient,
    pub(crate) session_id: SessionId,
    fence: Arc<RwLock<Option<ActorFence>>>,
    pub(crate) proactive: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<(String, String), everruns_core::ProactiveCompactionAttempt>,
        >,
    >,
}

impl Authority {
    /// Bind session creation idempotently. Changing its executable identity fails.
    pub async fn bind(&self, binding: &SessionBinding) -> Result<()> {
        self.call::<Value>("bind", json!({ "binding": binding }))
            .await?;
        Ok(())
    }

    /// Persist input and a durable wake before returning its stable message ID.
    /// Retry an uncertain response using the same AcceptedTurnInput value.
    pub async fn send(&self, input: AcceptedTurnInput) -> Result<MessageId> {
        self.submit(input.into_message()).await
    }

    /// Submit a caller-retained message identity, including retries after the
    /// caller restarts. Persist the message before sending when that is needed.
    pub async fn submit(&self, message: everruns_core::RuntimeMessage) -> Result<MessageId> {
        let id = message.id;
        let event = EventRequest::new(
            self.session_id,
            EventContext::empty(),
            InputMessageData::new(message),
        );
        self.call::<Value>("input", json!({ "event": event }))
            .await?;
        Ok(id)
    }

    /// Resolve pending client calls atomically and reactivate the same open turn.
    pub async fn resolve(
        &self,
        resolution_id: uuid::Uuid,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        self.call::<Value>(
            "resolve",
            json!({ "resolution_id": resolution_id, "results": results }),
        )
        .await?;
        Ok(())
    }

    /// Persist a cancellation targeting the current turn/input generation.
    pub async fn cancel(&self, command_id: uuid::Uuid) -> Result<()> {
        self.call::<Value>("cancel", json!({ "command_id": command_id }))
            .await?;
        Ok(())
    }

    /// Retry an explicitly suspended activation after repairing its host/build.
    pub async fn resume(&self) -> Result<()> {
        self.call::<Value>("resume", json!({})).await?;
        Ok(())
    }

    /// Read bounded canonical history. `through` pins a stable snapshot.
    pub async fn events(
        &self,
        after: i32,
        through: Option<i32>,
        limit: usize,
    ) -> Result<EventBatch> {
        self.call(
            "events",
            json!({ "after": after, "through": through, "limit": limit }),
        )
        .await
    }

    /// Claim one known session; this does not scan other objects.
    pub async fn claim(&self, holder: &str, ttl: Duration) -> Result<Option<Claim>> {
        self.call(
            "claim",
            json!({ "holder": holder, "ttl_ms": ttl.as_millis() }),
        )
        .await
    }

    /// Bind a new client view to a fixed ownership epoch. It cannot be updated
    /// to another generation while old executor work is still in flight.
    pub fn fenced(&self, holder: &str, fence: u64) -> Self {
        Self {
            client: self.client.clone(),
            session_id: self.session_id,
            fence: Arc::new(RwLock::new(Some(ActorFence::new(
                self.session_id,
                holder,
                fence,
            )))),
            proactive: Arc::default(),
        }
    }

    pub(crate) fn observe_fence(&self, fence: &ActorFence) {
        let mut current = self
            .fence
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current
            .as_ref()
            .is_some_and(|value| value.holder == fence.holder && value.fence == fence.fence)
        {
            *current = Some(fence.clone());
        }
    }

    pub(crate) fn check_session(&self, session_id: SessionId) -> Result<()> {
        if session_id != self.session_id {
            return Err(AgentLoopError::config(
                "authority belongs to another session",
            ));
        }
        Ok(())
    }

    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        operation: &str,
        mut body: Value,
    ) -> Result<T> {
        let fence = self
            .fence
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(fence) = &fence {
            if fence.is_lost() {
                return Err(actor_lease_lost(self.session_id, "ownership already lost"));
            }
            body["lease"] = json!({ "holder": fence.holder, "fence": fence.fence });
        }
        let result = self
            .client
            .transport
            .invoke(&self.client.tenant, self.session_id, operation, body)
            .await;
        if let Some(fence) = &fence {
            fence.observe(&result);
        }
        serde_json::from_value(result?)
            .map_err(|_| AgentLoopError::store("invalid authority response"))
    }
}

/// A bounded snapshot of canonical events.
#[derive(Debug, Deserialize)]
pub struct EventBatch {
    pub events: Vec<Event>,
    pub high_watermark: i32,
    pub has_more: bool,
}
