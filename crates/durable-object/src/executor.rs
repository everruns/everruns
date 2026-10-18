use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{AuthorityClient, SessionBinding};

/// Authenticated, capacity-bounded native activation endpoint for one agent build.
#[derive(Clone)]
pub struct Executor {
    client: AuthorityClient,
    binding: Option<SessionBinding>,
    credential_hash: [u8; 32],
    engine: everruns::Engine,
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
        engine: everruns::Engine,
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
            binding: Some(binding),
            credential_hash: Sha256::digest(format!("Bearer {token}").as_bytes()).into(),
            engine,
            slots: Arc::new(tokio::sync::Semaphore::new(concurrency)),
        })
    }

    /// Serve the retained builds registered by an ordinary serve application.
    /// The authority binding and Engine registry, rather than an HTTP Agent
    /// payload, authorize executable behavior.
    pub fn registered(
        client: AuthorityClient,
        token: &str,
        engine: everruns::Engine,
        concurrency: usize,
    ) -> Result<Self> {
        let mut executor = Self::new(
            client,
            SessionBinding {
                agent: "registry".into(),
                build_id: "registry".into(),
            },
            token,
            engine,
            concurrency,
        )?;
        executor.binding = None;
        Ok(executor)
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
    if activation.tenant != executor.client.tenant()
        || executor
            .binding
            .as_ref()
            .is_some_and(|binding| binding != &activation.binding)
    {
        return StatusCode::CONFLICT;
    }
    let binding = match executor.engine.session_binding(activation.session_id).await {
        Ok(binding) => binding.behavior,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };
    if binding.agent != activation.binding.agent
        || binding.build_id != activation.binding.build_id
        || !executor.engine.has_agent_build(&binding)
    {
        return StatusCode::CONFLICT;
    }
    let Ok(permit) = executor.slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS;
    };
    tokio::spawn(async move {
        let _permit = permit;
        if let Err(error) = executor.engine.activate(activation.session_id).await {
            tracing::warn!(session = %activation.session_id, %error, "Native Engine activation failed");
        }
    });
    StatusCode::ACCEPTED
}
