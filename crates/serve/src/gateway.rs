//! Model gateway: turn `"provider/model"` strings into runnable models.
//!
//! Decision: apps name models, hosts own credentials. Resolution order:
//!
//! 1. `"sim"` → the agent's offline script (or an echo simulator).
//! 2. `bedrock/<model-id>` with a region (`AWS_REGION` or `AWS_DEFAULT_REGION`)
//!    → Amazon Bedrock on the AWS default credential chain, given the Bedrock
//!    model id or inference profile after the first `/`. Checked before the
//!    gateway because it names a provider the host reaches with its own IAM
//!    role (an AgentCore Runtime execution role, an ECS task role, an instance
//!    profile), not a credential an app supplies. Needs the `bedrock` feature;
//!    without it a `bedrock/` model falls through to the rules below.
//! 3. `SERVE_GATEWAY_URL` (+ `SERVE_GATEWAY_KEY`) → an OpenAI-compatible
//!    gateway, given the full `provider/model` id. This is what a host sets.
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
    Gateway,
    OpenRouter,
    OpenAI,
}

impl Route {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Route::Simulator => "simulator",
            Route::Bedrock => "bedrock",
            Route::Gateway => "gateway",
            Route::OpenRouter => "openrouter",
            Route::OpenAI => "openai",
        }
    }
}

/// Where the gateway looks for credentials. Read once per resolution so tests
/// can supply their own.
pub(crate) struct Env {
    pub gateway_url: Option<String>,
    pub gateway_key: Option<String>,
    pub openrouter_key: Option<String>,
    pub openai_key: Option<String>,
    /// `AWS_REGION`, else `AWS_DEFAULT_REGION`: the signal that this host runs
    /// with AWS credentials for `bedrock/…` models.
    pub aws_region: Option<String>,
}

impl Env {
    pub(crate) fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            gateway_url: var("SERVE_GATEWAY_URL"),
            gateway_key: var("SERVE_GATEWAY_KEY"),
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
    if env.gateway_url.is_some() {
        return Some(Route::Gateway);
    }
    if env.openrouter_key.is_some() {
        return Some(Route::OpenRouter);
    }
    if model.starts_with("openai/") && env.openai_key.is_some() {
        return Some(Route::OpenAI);
    }
    None
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
        Some(Route::Gateway) => {
            let url = env.gateway_url.clone().unwrap_or_default();
            let key = env.gateway_key.clone().unwrap_or_default();
            Ok((
                Model::new(model, OpenAI::new(key).base_url(url)),
                Route::Gateway,
            ))
        }
        Some(Route::OpenRouter) => {
            let key = env.openrouter_key.clone().unwrap_or_default();
            Ok((Model::new(model, OpenRouter::new(key)), Route::OpenRouter))
        }
        Some(Route::OpenAI) => {
            let key = env.openai_key.clone().unwrap_or_default();
            let id = model.trim_start_matches("openai/");
            Ok((Model::new(id, OpenAI::new(key)), Route::OpenAI))
        }
        None if allow_offline => Ok(simulated()),
        None => anyhow::bail!(
            "no route for model `{model}`: set SERVE_GATEWAY_URL (hosted), OPENROUTER_API_KEY, \
             OPENAI_API_KEY for `openai/…` models, or AWS_REGION for `bedrock/…` models \
             (with the `bedrock` feature and AWS credentials or an IAM role)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            gateway_url: None,
            gateway_key: None,
            openrouter_key: None,
            openai_key: None,
            aws_region: None,
        }
    }

    #[test]
    fn sim_always_simulates() {
        let mut env = env();
        env.openrouter_key = Some("k".into());
        assert_eq!(route("sim", &env), Some(Route::Simulator));
    }

    #[test]
    fn host_gateway_wins_over_developer_keys() {
        let mut env = env();
        env.gateway_url = Some("https://gw.test/v1".into());
        env.openrouter_key = Some("k".into());
        assert_eq!(
            route("anthropic/claude-sonnet-5", &env),
            Some(Route::Gateway)
        );
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
        assert!(err.contains("SERVE_GATEWAY_URL"), "{err}");
        let (_, route) = resolve("anthropic/claude-sonnet-5", None, true, &env).unwrap();
        assert_eq!(route, Route::Simulator);
    }

    #[test]
    fn bedrock_models_need_a_region() {
        let mut env = env();
        env.gateway_url = Some("https://gw.test/v1".into());
        // No region: not a Bedrock route, so the host gateway takes it.
        assert_eq!(
            route("bedrock/us.anthropic.claude-sonnet-4-6", &env),
            Some(Route::Gateway)
        );
        env.gateway_url = None;
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
    fn bedrock_with_a_region_wins_over_the_gateway() {
        let mut env = env();
        env.aws_region = Some("us-west-2".into());
        env.gateway_url = Some("https://gw.test/v1".into());
        env.openrouter_key = Some("k".into());
        assert_eq!(
            route("bedrock/us.anthropic.claude-sonnet-4-6", &env),
            Some(Route::Bedrock)
        );
        // Other models still go to the gateway.
        assert_eq!(
            route("anthropic/claude-sonnet-5", &env),
            Some(Route::Gateway)
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
