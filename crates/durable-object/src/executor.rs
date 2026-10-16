use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::session_actor::store::ActorLeaseStore;
use everruns_core::host::session_actor::{
    ActorHosts, HostTurn, SessionActorConfig, SessionActorDriver, TurnSignals,
};
use everruns_core::host::{HostBackends, InProcessRuntime};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{Authority, AuthorityClient, SessionBinding};

/// Reconstruct executable agent behavior for a known session and coherent stores.
#[async_trait]
pub trait HostFactory: Send + Sync + 'static {
    async fn build(
        &self,
        session_id: SessionId,
        backends: HostBackends,
    ) -> Result<InProcessRuntime>;
}

#[async_trait]
impl<F, Fut> HostFactory for F
where
    F: Fn(SessionId, HostBackends) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<InProcessRuntime>> + Send,
{
    async fn build(
        &self,
        session_id: SessionId,
        backends: HostBackends,
    ) -> Result<InProcessRuntime> {
        self(session_id, backends).await
    }
}

/// Authenticated, capacity-bounded native activation endpoint for one agent build.
#[derive(Clone)]
pub struct Executor {
    client: AuthorityClient,
    binding: SessionBinding,
    credential_hash: [u8; 32],
    factory: Arc<dyn HostFactory>,
    slots: Arc<tokio::sync::Semaphore>,
}

#[derive(Deserialize)]
struct Activation {
    tenant: String,
    session_id: SessionId,
    binding: SessionBinding,
}

impl Executor {
    /// The executor secret is separate from the authority service secret.
    pub fn new(
        client: AuthorityClient,
        binding: SessionBinding,
        token: &str,
        factory: impl HostFactory,
        concurrency: usize,
    ) -> Result<Self> {
        if token.len() < 32
            || client.credential_matches(token)
            || concurrency == 0
            || concurrency > 1024
            || binding.agent.is_empty()
            || binding.build_id.is_empty()
        {
            return Err(AgentLoopError::config(
                "executor requires an agent/build, a distinct 32-byte secret and concurrency in 1..=1024",
            ));
        }
        Ok(Self {
            client,
            binding,
            credential_hash: Sha256::digest(format!("Bearer {token}").as_bytes()).into(),
            factory: Arc::new(factory),
            slots: Arc::new(tokio::sync::Semaphore::new(concurrency)),
        })
    }

    /// Merge the `/execute` route into the executor's HTTP server.
    pub fn router(self) -> Router {
        Router::new()
            .route("/execute", post(activate))
            .layer(DefaultBodyLimit::max(4096))
            .with_state(self)
    }
}

async fn activate(
    State(executor): State<Executor>,
    headers: HeaderMap,
    Json(activation): Json<Activation>,
) -> StatusCode {
    // THREAT[TM-DURABLE-012]: activation authorizes a retained executable identity.
    let received: [u8; 32] = Sha256::digest(
        headers
            .get("authorization")
            .map_or(&[][..], |h| h.as_bytes()),
    )
    .into();
    if !bool::from(received.ct_eq(&executor.credential_hash)) {
        return StatusCode::UNAUTHORIZED;
    }
    if activation.tenant != executor.client.tenant() || activation.binding != executor.binding {
        return StatusCode::CONFLICT;
    }
    let Ok(permit) = executor.slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS;
    };
    let authority = executor.client.session(activation.session_id);
    let holder = uuid::Uuid::new_v4().to_string();
    let config = SessionActorConfig::new(&holder);
    let claim = match authority.claim(&holder, config.lease_ttl).await {
        Ok(Some(claim)) => claim,
        Ok(None) => return StatusCode::NO_CONTENT,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };
    if claim.binding != executor.binding || claim.actor.session_id != activation.session_id {
        return StatusCode::CONFLICT;
    }
    let fenced = authority.fenced(&holder, claim.actor.fence);
    // The command and alarm already live at the authority. Losing this task
    // expires its lease and causes another activation, including during setup.
    tokio::spawn(async move {
        let _permit = permit;
        let runtime = match tokio::time::timeout(
            config.lease_ttl,
            executor
                .factory
                .build(activation.session_id, fenced.backends()),
        )
        .await
        {
            Ok(Ok(runtime)) => runtime,
            Ok(Err(error)) => {
                if error.is_non_retryable() || claim.actor.attempts > config.poison_attempts {
                    let _ = fenced
                        .call::<serde_json::Value>("suspend", serde_json::json!({}))
                        .await;
                }
                return;
            }
            Err(_) => return,
        };
        // Construction may use most of the original lease. Confirm ownership
        // and a fresh TTL before executing any agent code.
        let mut actor = claim.actor;
        let Ok(Some(renewal)) = fenced
            .renew_actor_lease(
                actor.session_id,
                &holder,
                actor.fence,
                config.lease_ttl,
                actor.consumed_seq,
            )
            .await
        else {
            return;
        };
        actor.inbox_seq = renewal.inbox_seq;
        actor.cancel_inbox_seq = renewal.cancel_inbox_seq;
        let hosts = NativeHosts {
            runtime,
            authority: fenced.clone(),
        };
        SessionActorDriver::new(Arc::new(hosts), Arc::new(fenced), config)
            .run_actor(actor)
            .await;
    });
    StatusCode::ACCEPTED
}

struct NativeHosts {
    runtime: InProcessRuntime,
    authority: Authority,
}

#[async_trait]
impl ActorHosts for NativeHosts {
    type Host = InProcessRuntime;
    fn host(&self, _turn: HostTurn, signals: &TurnSignals) -> InProcessRuntime {
        if let Some(fence) = &signals.fence {
            self.authority.observe_fence(fence);
        }
        self.runtime.clone()
    }
}
