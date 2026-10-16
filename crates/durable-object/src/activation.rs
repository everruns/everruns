//! One activation path for native and isolate placements.
use crate::{Authority, AuthorityClient, Claim, SessionBinding};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::session_actor::store::SessionActorStore;
use everruns_core::host::session_actor::{
    ActorHosts, HostTurn, SessionActorConfig, SessionActorDriver, TurnSignals,
};
use everruns_core::host::{HostBackends, InProcessRuntime};
use std::{future::Future, sync::Arc};

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

/// Activate a known session on the current host after validating its retained build.
pub async fn activate(
    client: AuthorityClient,
    binding: &SessionBinding,
    session: SessionId,
    factory: Arc<dyn HostFactory>,
) -> Result<()> {
    let config = SessionActorConfig::new(uuid::Uuid::new_v4().to_string());
    let authority = client.session(session);
    let Some(claim) = authority.claim(&config.holder, config.lease_ttl).await? else {
        return Ok(());
    };
    if claim.binding != *binding || claim.actor.session_id != session {
        return Err(everruns_contracts::error::AgentLoopError::config(
            "incompatible retained agent build",
        ));
    }
    run_claimed(authority, claim, config, factory).await
}

pub(crate) async fn run_claimed(
    authority: Authority,
    claim: Claim,
    config: SessionActorConfig,
    factory: Arc<dyn HostFactory>,
) -> Result<()> {
    let fenced = authority.fenced(&config.holder, claim.actor.fence);
    let runtime = match everruns_contracts::rt::timeout(
        config.lease_ttl,
        factory.build(claim.actor.session_id, fenced.backends()),
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
            return Err(error);
        }
        Err(_) => {
            return Err(everruns_contracts::error::AgentLoopError::store(
                "host construction timed out",
            ));
        }
    };
    let mut actor = claim.actor;
    let Some(renewal) = fenced
        .renew_actor_lease(
            actor.session_id,
            &config.holder,
            actor.fence,
            config.lease_ttl,
            actor.consumed_seq,
        )
        .await?
    else {
        return Ok(());
    };
    actor.inbox_seq = renewal.inbox_seq;
    actor.cancel_inbox_seq = renewal.cancel_inbox_seq;
    let hosts = RuntimeHosts {
        runtime,
        authority: fenced.clone(),
    };
    SessionActorDriver::new(Arc::new(hosts), Arc::new(fenced), config)
        .run_actor(actor)
        .await;
    Ok(())
}

struct RuntimeHosts {
    runtime: InProcessRuntime,
    authority: Authority,
}

#[async_trait]
impl ActorHosts for RuntimeHosts {
    type Host = InProcessRuntime;
    fn host(&self, _turn: HostTurn, signals: &TurnSignals) -> InProcessRuntime {
        if let Some(fence) = &signals.fence {
            self.authority.observe_fence(fence);
        }
        self.runtime.clone()
    }
}
