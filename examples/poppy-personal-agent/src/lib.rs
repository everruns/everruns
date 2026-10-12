//! A personal agent on the Everruns Framework that talks to companies over
//! the Personal Agent Protocol ("Poppy").
//!
//! The person asks a question about a company; the agent finds the company by
//! its domain, messages the company's agent, and reports what it said. The
//! app is the personal agent: it signs with its own key and publishes its
//! client metadata and key ([`identity_routes`]) at a public HTTPS address,
//! where companies check them.
//!
//! The example runs on a real model (`gpt-6.1-sol`).

use everruns::poppy::{AGENT_PATH, JWKS_PATH, PoppyPersonalAgent};
use everruns::providers::openai::{OpenAI, OpenAIError};
use everruns::{Agent, BuildError, LocalConfig, Model};

/// The model the example runs on.
pub const MODEL: &str = "gpt-6.1-sol";

/// The name companies see for this personal agent.
pub const CLIENT_NAME: &str = "Everruns Poppy example";

pub const INSTRUCTIONS: &str = "\
You are a personal assistant. When the person asks about their business with a \
company, talk to the company's agent with the Poppy tools and tell the person \
what the company said, in its words. If the company needs the person to sign \
in, say so.";

/// Build the personal agent. Session state lives in `state_dir`.
pub fn personal_agent(
    model: Model,
    poppy: PoppyPersonalAgent,
    state_dir: &std::path::Path,
) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("personal-agent")
        .instructions(INSTRUCTIONS)
        .model(model)
        .capability(poppy)
        .max_iterations(8)
        .local(LocalConfig::new(state_dir))
        .build()
}

/// OpenAI's `gpt-6.1-sol`, keyed by `OPENAI_API_KEY`.
pub fn openai_model() -> Result<Model, OpenAIError> {
    Ok(Model::new(MODEL, OpenAI::from_env()?))
}

/// The two documents companies fetch: the client metadata at the agent's
/// `client_id` and its JWK Set.
pub fn identity_routes(poppy: &PoppyPersonalAgent) -> axum::Router {
    let metadata = poppy.metadata(CLIENT_NAME);
    let jwks = poppy.jwks();
    axum::Router::new()
        .route(
            AGENT_PATH,
            axum::routing::get(move || async move { axum::Json(metadata) }),
        )
        .route(
            JWKS_PATH,
            axum::routing::get(move || async move { axum::Json(jwks) }),
        )
}

/// The Poppy tools the agent called in `session`, with their arguments.
pub async fn poppy_calls(
    session: &everruns::Session,
) -> Result<Vec<(String, serde_json::Value)>, everruns::HistoryError> {
    Ok(session
        .history()
        .page()
        .await?
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|part| match part {
            everruns::ContentPart::ToolCall(call) if call.name.starts_with("poppy_") => {
                Some((call.name.clone(), call.arguments.clone()))
            }
            _ => None,
        })
        .collect())
}
