// Microsoft Foundry decisions (Microsoft-Decision-1).
//
// Foundry serves Microsoft's decision models over the System One protocol
// (the TypeSafe/OpenRouter wire: `state`, a `questions` map of noul/choice/score,
// `answers` keyed by question id). It lives at the resource root, not under the
// OpenAI-compatible `/openai/v1` base the chat driver uses:
//
//   POST https://<resource>.services.ai.azure.com/providers/microsoft/v1/systemone
//
// `model` is the deployment name, not the catalog id: a deployment named
// `Decision-1` answers only to `Decision-1` and reports `microsoft-decision-1`
// back. Verified live on 2026-10-09 against a GlobalStandard deployment; the
// endpoint enforces 2-10 score levels and 2-255 choice options, the same
// limits System One declares. A project-scoped base URL
// (`.../api/projects/<name>`) is not used: that path demands an `api-version`
// the route does not document, while the resource root accepts the same key.

use async_trait::async_trait;
use everruns_contracts::ProviderEndpoint;
use everruns_contracts::decision_driver::{DecisionDriver, DecisionDriverCapabilities};
use everruns_contracts::decisions::{DecisionOutcome, DecisionRequest};
use everruns_contracts::error::{AgentLoopError, Result};

use crate::systemone::SystemOneDecisionDriver;

/// Path of the System One route under a Foundry resource root.
const SYSTEMONE_PATH: &str = "/providers/microsoft/v1/systemone";

/// The System One URL for a Foundry provider endpoint.
///
/// The endpoint's base URL is the chat driver's (`{resource}/openai/v1`,
/// possibly under a project). The decision route sits at the resource root,
/// so both suffixes are removed before the route is appended. Query
/// parameters are kept, as the chat driver keeps them.
///
/// ```
/// use everruns_drivers::mai::{MaiAuth, decisions_url, provider};
///
/// let service = provider(
///     "foundry",
///     "https://res.services.ai.azure.com/api/projects/dev",
///     MaiAuth::ApiKey("key".into()),
/// );
/// assert_eq!(
///     decisions_url(service.endpoint()).unwrap(),
///     "https://res.services.ai.azure.com/providers/microsoft/v1/systemone",
/// );
/// ```
pub fn decisions_url(endpoint: &ProviderEndpoint) -> Option<String> {
    let mut url = reqwest::Url::parse(endpoint.base_url()?).ok()?;
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/openai/v1").unwrap_or(path);
    let path = match path.rfind("/api/projects/") {
        Some(index) if !path[index + "/api/projects/".len()..].contains('/') => &path[..index],
        _ => path,
    };
    let path = format!("{path}{SYSTEMONE_PATH}");
    url.set_path(&path);
    Some(url.to_string())
}

/// Microsoft Foundry decision driver: System One at the Foundry route.
#[derive(Clone, Debug, Default)]
pub struct MaiDecisionDriver(SystemOneDecisionDriver);

impl MaiDecisionDriver {
    /// Create the driver. Authentication comes from the provider endpoint.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DecisionDriver for MaiDecisionDriver {
    fn id(&self) -> &str {
        "mai-systemone"
    }

    fn capabilities(&self) -> DecisionDriverCapabilities {
        self.0.capabilities()
    }

    async fn evaluate(
        &self,
        endpoint: &ProviderEndpoint,
        request: DecisionRequest,
    ) -> Result<DecisionOutcome> {
        let url = decisions_url(endpoint)
            .ok_or_else(|| AgentLoopError::llm("Decision provider endpoint is required"))?;
        self.0.evaluate_at(endpoint, url, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mai::{MaiAuth, provider};
    use everruns_contracts::decisions::{DecisionAnswer, DecisionQuestion};
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn route_sits_at_the_resource_root() {
        for (base, expected) in [
            (
                "https://res.services.ai.azure.com",
                "https://res.services.ai.azure.com/providers/microsoft/v1/systemone",
            ),
            (
                "https://res.services.ai.azure.com/openai/v1/",
                "https://res.services.ai.azure.com/providers/microsoft/v1/systemone",
            ),
            (
                "https://res.services.ai.azure.com/api/projects/dev",
                "https://res.services.ai.azure.com/providers/microsoft/v1/systemone",
            ),
            (
                "https://res.services.ai.azure.com/api/projects/dev/openai/v1/chat/completions",
                "https://res.services.ai.azure.com/providers/microsoft/v1/systemone",
            ),
            (
                "https://proxy.example/foundry?route=a",
                "https://proxy.example/foundry/providers/microsoft/v1/systemone?route=a",
            ),
        ] {
            let service = provider("foundry", base, MaiAuth::ApiKey("key".into()));
            assert_eq!(
                decisions_url(service.endpoint()).as_deref(),
                Some(expected),
                "{base}"
            );
        }
        assert!(decisions_url(&ProviderEndpoint::default()).is_none());
    }

    /// The live response from 2026-10-09, trimmed to one answer per primitive.
    fn live_response() -> Value {
        json!({
            "model": "microsoft-decision-1",
            "answers": {
                "priority": {
                    "type": "score",
                    "score": 1.591248736299162,
                    "confidence": 0.3882084721160395,
                    "legend": {"0": "Routine", "1": "Important", "2": "Urgent"},
                    "probabilities": {"0": 0.0008902451115308707, "1": 0.4069707734777761, "2": 0.592138981410693}
                },
                "team": {
                    "type": "choice",
                    "choice": "billing",
                    "confidence": 0.9983918284983826,
                    "probabilities": {"billing": 0.9991959142491913, "technical": 0.0008040857508087322}
                },
                "urgency": {"type": "noul", "noul": 0.9971990694457676}
            },
            "usage": {"input_tokens": 115, "output_tokens": 3}
        })
    }

    #[tokio::test]
    async fn speaks_system_one_with_the_foundry_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/providers/microsoft/v1/systemone"))
            .and(header("api-key", "foundry-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(live_response()))
            .expect(1)
            .mount(&server)
            .await;
        let service = provider(
            "foundry",
            format!("{}/api/projects/dev", server.uri()),
            MaiAuth::ApiKey("foundry-key".into()),
        );
        let request = DecisionRequest::new("My payment failed twice today.")
            .model("Decision-1")
            .ask("urgency", DecisionQuestion::noul("Is it urgent?"))
            .ask(
                "team",
                DecisionQuestion::Choice {
                    instructions: "Which team?".into(),
                    options: vec![
                        ("billing".into(), Some("Payments".into())),
                        ("technical".into(), None),
                    ],
                },
            )
            .ask(
                "priority",
                DecisionQuestion::score("Priority?", ["Routine", "Important", "Urgent"]),
            );
        let outcome = MaiDecisionDriver::new()
            .evaluate(service.endpoint(), request)
            .await
            .unwrap();
        assert_eq!(outcome.model, "microsoft-decision-1");
        assert!(outcome.calibrated);
        assert_eq!(outcome.usage.input_tokens, 115);
        assert!(outcome.get("urgency").unwrap().probability_yes().unwrap() > 0.99);
        let DecisionAnswer::Choice { selected, .. } = outcome.get("team").unwrap() else {
            panic!("expected a choice");
        };
        assert_eq!(selected, "billing");
        let DecisionAnswer::Score { score, .. } = outcome.get("priority").unwrap() else {
            panic!("expected a score");
        };
        assert!((score - 1.5912).abs() < 1e-3);

        let requests = server.received_requests().await.unwrap();
        let body: Value = requests[0].body_json().unwrap();
        assert_eq!(body["model"], "Decision-1");
        assert_eq!(body["questions"]["urgency"]["type"], "noul");
        assert_eq!(body["questions"]["team"]["criteria"]["billing"], "Payments");
        assert!(requests[0].headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn surfaces_foundry_errors_without_their_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": {"code": "DeploymentNotFound", "message": "The API deployment x does not exist."}
            })))
            .mount(&server)
            .await;
        let service = provider("foundry", server.uri(), MaiAuth::ApiKey("key".into()));
        let error = MaiDecisionDriver::new()
            .evaluate(
                service.endpoint(),
                DecisionRequest::new("state")
                    .model("missing")
                    .ask("q", DecisionQuestion::noul("Is it?")),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 404"), "{error}");
    }
}
