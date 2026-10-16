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

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Client configuration for a single tenant's authority service.
#[derive(Clone)]
pub struct AuthorityClient {
    base: reqwest::Url,
    tenant: String,
    token: reqwest::header::HeaderValue,
    http: reqwest::Client,
}

impl AuthorityClient {
    /// Use HTTPS, or an explicit loopback HTTP endpoint for local development.
    /// Redirects are disabled so service credentials never follow a redirect.
    pub fn new(base: &str, tenant: &str, token: &str) -> Result<Self> {
        let base = reqwest::Url::parse(base)
            .map_err(|_| AgentLoopError::config("invalid authority URL"))?;
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && loopback)
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(AgentLoopError::config(
                "authority URL must be an HTTPS origin (loopback HTTP is allowed)",
            ));
        }
        if tenant.is_empty()
            || tenant.len() > 128
            || !tenant
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || token.len() < 32
        {
            return Err(AgentLoopError::config(
                "authority requires a valid tenant and a service token of at least 32 bytes",
            ));
        }
        let mut token = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AgentLoopError::config("invalid authority credential"))?;
        token.set_sensitive(true);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| AgentLoopError::config("cannot build authority transport"))?;
        Ok(Self {
            base,
            tenant: tenant.to_owned(),
            token,
            http,
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

    pub(crate) fn tenant(&self) -> &str {
        &self.tenant
    }

    pub(crate) fn credential_matches(&self, token: &str) -> bool {
        self.token.as_bytes() == format!("Bearer {token}").as_bytes()
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
        let path = format!(
            "/tenants/{}/sessions/{}/{}",
            self.client.tenant, self.session_id, operation
        );
        let url = self
            .client
            .base
            .join(&path)
            .map_err(|_| AgentLoopError::config("invalid authority path"))?;
        let mut response = self
            .client
            .http
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.client.token.clone())
            .json(&body)
            .send()
            .await
            .map_err(|_| AgentLoopError::store("authority transport unavailable"))?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            let error = actor_lease_lost(self.session_id, "authority rejected stale ownership");
            if let Some(fence) = &fence {
                fence.observe::<()>(&Err(actor_lease_lost(self.session_id, "stale ownership")));
            }
            return Err(error);
        }
        if !response.status().is_success() {
            return Err(AgentLoopError::store(format!(
                "authority {operation} rejected ({})",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AgentLoopError::store("authority response interrupted"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AgentLoopError::store("authority response exceeds limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
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
