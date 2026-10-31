//! Model routing: turn `"provider/model"` strings into runnable models.
//!
//! Decision: apps name models, hosts own credentials. Resolution order:
//!
//! 1. `"sim"` → the agent's offline script (or an echo simulator).
//! 2. `bedrock/<model-id>` with a region (`AWS_REGION` or `AWS_DEFAULT_REGION`)
//!    → Amazon Bedrock on the AWS default credential chain, given the Bedrock
//!    model id or inference profile after the first `/`. Checked before the
//!    API-key routes because it names a provider the host reaches with its own IAM
//!    role (an AgentCore Runtime execution role, an ECS task role, an instance
//!    profile), not a credential an app supplies. Needs the `bedrock` feature;
//!    without it a `bedrock/` model falls through to the rules below.
//! 3. `openai/<model>` with `OPENAI_BASE_URL` and `OPENAI_API_KEY` → the
//!    configured OpenAI-compatible Responses API, given `<model>`.
//! 4. `OPENROUTER_API_KEY` → OpenRouter, which accepts `provider/model` ids.
//! 5. `openai/<model>` with `OPENAI_API_KEY` → OpenAI directly.
//! 6. Otherwise, in `dev` and evals, the offline script; in `start`, an error.

#[cfg(feature = "bedrock")]
use everruns::providers::bedrock::Bedrock;
use everruns::providers::{openai::OpenAI, openrouter::OpenRouter};
use everruns::{LlmSimConfig, Model};

/// How a model string was resolved, for the dev console and agent card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Simulator,
    Bedrock,
    OpenRouter,
    OpenAI,
}

impl Route {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Route::Simulator => "simulator",
            Route::Bedrock => "bedrock",
            Route::OpenRouter => "openrouter",
            Route::OpenAI => "openai",
        }
    }
}

/// Where model routing looks for credentials. Read once per resolution so tests
/// can supply their own.
pub(crate) struct Env {
    pub openai_base_url: Option<String>,
    pub openrouter_key: Option<String>,
    pub openai_key: Option<String>,
    /// `AWS_REGION`, else `AWS_DEFAULT_REGION`: the signal that this host runs
    /// with AWS credentials for `bedrock/…` models.
    pub aws_region: Option<String>,
}

impl Env {
    pub(crate) fn from_process() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        Self {
            openai_base_url: var("OPENAI_BASE_URL"),
            openrouter_key: var("OPENROUTER_API_KEY"),
            openai_key: var("OPENAI_API_KEY"),
            aws_region: var("AWS_REGION").or_else(|| var("AWS_DEFAULT_REGION")),
        }
    }
}

/// Decide the route for `model` without building anything.
pub(crate) fn route(model: &str, env: &Env) -> Option<Route> {
    if model == "sim" || model.starts_with("sim/") {
        return Some(Route::Simulator);
    }
    if cfg!(feature = "bedrock") && bedrock_model_id(model).is_some() && env.aws_region.is_some() {
        return Some(Route::Bedrock);
    }
    if openai_model_id(model).is_some() && env.openai_base_url.is_some() && env.openai_key.is_some()
    {
        return Some(Route::OpenAI);
    }
    if env.openrouter_key.is_some() {
        return Some(Route::OpenRouter);
    }
    if openai_model_id(model).is_some() && env.openai_key.is_some() {
        return Some(Route::OpenAI);
    }
    None
}

/// Strip only the routing prefix; endpoint model ids may themselves contain `/`.
fn openai_model_id(model: &str) -> Option<&str> {
    model
        .strip_prefix("openai/")
        .filter(|id| !id.trim().is_empty())
}

/// The Bedrock model id or inference profile in `bedrock/<model-id>`.
fn bedrock_model_id(model: &str) -> Option<&str> {
    model
        .strip_prefix("bedrock/")
        .filter(|id| !id.trim().is_empty())
}

/// Resolve to a model. `allow_offline` lets an unroutable model fall back to
/// the simulator (dev, evals); without it the caller gets an error naming what
/// to set.
pub(crate) fn resolve(
    model: &str,
    offline: Option<&LlmSimConfig>,
    allow_offline: bool,
    env: &Env,
) -> crate::Result<(Model, Route)> {
    let simulated = || {
        let config = offline.cloned().unwrap_or_else(LlmSimConfig::echo);
        (Model::simulated_with_config(config), Route::Simulator)
    };
    match route(model, env) {
        Some(Route::Simulator) => Ok(simulated()),
        #[cfg(feature = "bedrock")]
        Some(Route::Bedrock) => {
            let id = bedrock_model_id(model).unwrap_or_default();
            let region = env.aws_region.clone().unwrap_or_default();
            Ok((
                Model::new(id, Bedrock::default_chain().region(region)),
                Route::Bedrock,
            ))
        }
        #[cfg(not(feature = "bedrock"))]
        // `route` never picks Bedrock without the feature.
        Some(Route::Bedrock) => {
            anyhow::bail!("model `{model}` needs serve's `bedrock` feature")
        }
        Some(Route::OpenRouter) => {
            let key = env.openrouter_key.clone().unwrap_or_default();
            Ok((Model::new(model, OpenRouter::new(key)), Route::OpenRouter))
        }
        Some(Route::OpenAI) => {
            let key = env.openai_key.clone().unwrap_or_default();
            let id = openai_model_id(model).unwrap_or_default();
            let mut provider = OpenAI::new(key);
            if let Some(url) = &env.openai_base_url {
                provider = provider.base_url(url);
            }
            Ok((Model::new(id, provider), Route::OpenAI))
        }
        None if allow_offline => Ok(simulated()),
        None => anyhow::bail!(
            "no route for model `{model}`: set OPENROUTER_API_KEY, \
             OPENAI_API_KEY (optionally OPENAI_BASE_URL) for `openai/…` models, \
             or AWS_REGION for `bedrock/…` models \
             (with the `bedrock` feature and AWS credentials or an IAM role)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            openai_base_url: None,
            openrouter_key: None,
            openai_key: None,
            aws_region: None,
        }
    }

    #[test]
    fn reads_standard_openai_settings_and_ignores_empty_values() {
        let env = Env::from_lookup(|name| match name {
            "OPENAI_BASE_URL" => Some("https://models.example/v1".into()),
            "OPENAI_API_KEY" => Some("synthetic-key".into()),
            "OPENROUTER_API_KEY" | "AWS_REGION" => Some(String::new()),
            "AWS_DEFAULT_REGION" => Some("us-east-1".into()),
            _ => None,
        });
        assert_eq!(
            env.openai_base_url.as_deref(),
            Some("https://models.example/v1")
        );
        assert_eq!(env.openai_key.as_deref(), Some("synthetic-key"));
        assert!(env.openrouter_key.is_none());
        assert_eq!(env.aws_region.as_deref(), Some("us-east-1"));
    }

    #[tokio::test]
    async fn openai_endpoint_receives_the_key_and_unprefixed_model_id() {
        use axum::{Json, Router, http::StatusCode, routing::post};
        use serde_json::{Value, json};

        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let app = Router::new().route(
            "/v1/responses",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let tx = tx.clone();
                    async move {
                        tx.send((headers, body)).await.unwrap();
                        // Only the outbound request is under test; no model is simulated here.
                        (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": {"message": "captured"}})),
                        )
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let env = Env::from_lookup(|name| match name {
            "OPENAI_BASE_URL" => Some(url.clone()),
            "OPENAI_API_KEY" => Some("synthetic-key".into()),
            // An explicit OpenAI endpoint must not send this request to OpenRouter.
            "OPENROUTER_API_KEY" => Some("unused-key".into()),
            _ => None,
        });
        for (configured, expected) in [
            ("openai/gpt-6-luna", "gpt-6-luna"),
            ("openai/openai/custom-model", "openai/custom-model"),
        ] {
            let (model, route) = resolve(configured, None, false, &env).unwrap();
            assert_eq!(route, Route::OpenAI);
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                model.complete("Capture this request"),
            )
            .await
            .unwrap();
            assert!(result.is_err());
            let (headers, body) = rx
                .try_recv()
                .expect("request reached the configured endpoint");
            assert_eq!(headers["authorization"], "Bearer synthetic-key");
            assert_eq!(body["model"], expected);
        }
        server.abort();
    }

    #[test]
    fn sim_always_simulates() {
        let mut env = env();
        env.openrouter_key = Some("k".into());
        assert_eq!(route("sim", &env), Some(Route::Simulator));
    }

    #[test]
    fn explicit_openai_endpoint_wins_for_openai_models_only() {
        let mut env = env();
        env.openai_base_url = Some("https://models.example/v1".into());
        env.openai_key = Some("openai-key".into());
        env.openrouter_key = Some("k".into());
        assert_eq!(route("openai/gpt-6-luna", &env), Some(Route::OpenAI));
        assert_eq!(
            route("anthropic/claude-sonnet-5", &env),
            Some(Route::OpenRouter)
        );
        env.openrouter_key = None;
        assert_eq!(route("anthropic/claude-sonnet-5", &env), None);
        env.openai_key = None;
        assert_eq!(route("openai/gpt-6-luna", &env), None);
    }

    #[test]
    fn openai_key_only_routes_openai_models() {
        let mut env = env();
        env.openai_key = Some("k".into());
        assert_eq!(route("openai/gpt-5-mini", &env), Some(Route::OpenAI));
        assert_eq!(route("anthropic/claude-sonnet-5", &env), None);
    }

    #[test]
    fn unroutable_model_is_an_error_in_production_and_simulated_in_dev() {
        let env = env();
        let err = resolve("anthropic/claude-sonnet-5", None, false, &env)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("OPENAI_BASE_URL"), "{err}");
        let (_, route) = resolve("anthropic/claude-sonnet-5", None, true, &env).unwrap();
        assert_eq!(route, Route::Simulator);
    }

    #[test]
    fn bedrock_models_need_a_region() {
        let mut env = env();
        env.openrouter_key = Some("key".into());
        // No region: not a Bedrock route, so OpenRouter takes it.
        assert_eq!(
            route("bedrock/us.anthropic.claude-sonnet-4-6", &env),
            Some(Route::OpenRouter)
        );
        env.openrouter_key = None;
        assert_eq!(route("bedrock/us.anthropic.claude-sonnet-4-6", &env), None);
    }

    #[test]
    fn a_bare_bedrock_prefix_is_not_a_model() {
        let mut env = env();
        env.aws_region = Some("us-east-1".into());
        assert_eq!(route("bedrock/", &env), None);
        assert_eq!(route("bedrock", &env), None);
        assert_eq!(route("anthropic/claude-sonnet-5", &env), None);
    }

    #[cfg(feature = "bedrock")]
    #[test]
    fn bedrock_with_a_region_wins_over_api_key_routes() {
        let mut env = env();
        env.aws_region = Some("us-west-2".into());
        env.openai_base_url = Some("https://models.example/v1".into());
        env.openai_key = Some("openai-key".into());
        env.openrouter_key = Some("k".into());
        assert_eq!(
            route("bedrock/us.anthropic.claude-sonnet-4-6", &env),
            Some(Route::Bedrock)
        );
        // Other models still follow their provider routes.
        assert_eq!(
            route("anthropic/claude-sonnet-5", &env),
            Some(Route::OpenRouter)
        );
        let (_, route) =
            resolve("bedrock/us.anthropic.claude-sonnet-4-6", None, false, &env).unwrap();
        assert_eq!(route, Route::Bedrock);
        assert_eq!(route.label(), "bedrock");
    }

    #[cfg(not(feature = "bedrock"))]
    #[test]
    fn without_the_feature_bedrock_models_are_not_routed() {
        let mut env = env();
        env.aws_region = Some("us-west-2".into());
        assert_eq!(route("bedrock/us.anthropic.claude-sonnet-4-6", &env), None);
        let err = resolve("bedrock/us.anthropic.claude-sonnet-4-6", None, false, &env)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("AWS_REGION"), "{err}");
    }
}
