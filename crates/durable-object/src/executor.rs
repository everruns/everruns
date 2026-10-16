use std::sync::Arc;

use crate::HostFactory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::session_actor::SessionActorConfig;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{AuthorityClient, SessionBinding};

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
    tokio::spawn(async move {
        let _permit = permit;
        let _ = crate::activation::run_claimed(authority, claim, config, executor.factory).await;
    });
    StatusCode::ACCEPTED
}
