// TypeSafe Connection Provider
//
// Decision: API-key connection (not OAuth); the user pastes a key from typesafe.ai.
// Decision: Validate by asking the cheapest possible real question — a one-noul
//   evaluation — because the API has no dedicated auth probe. 401 means invalid.

use async_trait::async_trait;
use everruns_platform::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use crate::client::{Error, Evaluation, Question, RetryPolicy, TypeSafeAIClient};

/// Connector catalog entry for TypeSafe.
pub struct TypeSafeAIConnector;

#[async_trait]
impl Connector for TypeSafeAIConnector {
    fn provider_id(&self) -> &str {
        crate::TYPESAFE_CONNECTION_PROVIDER
    }

    fn display_name(&self) -> &str {
        "TypeSafe"
    }

    fn description(&self) -> &str {
        "Typed judgments — probabilities, selections, and scores — for agents"
    }

    fn icon(&self) -> &str {
        "scale"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .with_placeholder("ts-..."),
            ],
            instructions_markdown: "\
1. Sign in at [typesafe.ai](https://typesafe.ai)\n\
2. Create an **API key** in the dashboard\n\
3. Paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        probe_key(
            TypeSafeAIClient::builder(credential)
                .retry(RetryPolicy::none())
                .build(),
        )
        .await
    }
}

/// The validation round trip, over a caller-supplied client.
///
/// Split from [`Connector::validate`] so the endpoint is injectable: `validate`
/// itself can only ever talk to the real API, and the status-to-message mapping
/// below is what the user reads when a key does not work.
async fn probe_key(client: TypeSafeAIClient) -> Result<ConnectorValidation, String> {
    // One noul over a two-word state: the smallest request the API accepts.
    let probe =
        Evaluation::new("connection check").ask("ok", Question::noul("Is this text in English?"));

    match client.evaluate(probe).await {
        Ok(_) => Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: None,
        }),
        Err(Error::Api { status: 401, .. }) | Err(Error::Api { status: 403, .. }) => {
            Err("Invalid API key. Check that the key is correct and active.".into())
        }
        Err(Error::Api { status: 429, .. }) => {
            Err("API key is valid but rate-limited. Try again in a moment.".into())
        }
        Err(error) => Err(format!("Could not verify the key: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CREDENTIAL: &str = "sentinel-connector-credential";

    fn client(server: &MockServer) -> TypeSafeAIClient {
        TypeSafeAIClient::builder(CREDENTIAL)
            .base_url(server.uri())
            .retry(RetryPolicy::none())
            .build()
    }

    async fn responding(status: u16, body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(&server)
            .await;
        server
    }

    #[test]
    fn connector_metadata_matches_the_capability() {
        let connector = TypeSafeAIConnector;
        assert_eq!(connector.provider_id(), crate::TYPESAFE_CONNECTION_PROVIDER);
        assert_eq!(connector.display_name(), "TypeSafe");
        assert_eq!(connector.connection_type(), ConnectorType::ApiKey);
    }

    #[test]
    fn form_asks_for_one_required_secret_field() {
        let schema = TypeSafeAIConnector.form_schema().expect("form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(schema.instructions_markdown.contains("typesafe.ai"));
    }

    // --- validation round trip ---

    /// The probe spends the cheapest request the API accepts, and it must carry
    /// the key being validated — not the one the process happens to hold.
    #[tokio::test]
    async fn a_working_key_is_accepted_after_one_minimal_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(header("authorization", format!("Bearer {CREDENTIAL}")))
            // The whole probe: one noul over a two-word state. If this body ever
            // grows, connecting an account starts costing more than it should.
            .and(body_json(json!({
                "state": "connection check",
                "model": "jev-latest",
                "questions": {
                    "ok": {"type": "noul", "instructions": "Is this text in English?"}
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "jev-1.13.0",
                "answers": {"ok": {"type": "noul", "noul": 0.99}},
                "usage": {"input_tokens": 12, "output_tokens": 2}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let validation = probe_key(client(&server)).await.expect("key accepted");
        assert!(validation.provider_username.is_none());
        assert!(validation.provider_metadata.is_none());
    }

    #[tokio::test]
    async fn a_rejected_key_says_the_key_is_wrong() {
        for status in [401, 403] {
            let server = responding(status, r#"{"error":{"message":"bad key"}}"#).await;
            let message = probe_key(client(&server)).await.unwrap_err();
            assert!(message.contains("Invalid API key"), "{status}: {message}");
        }
    }

    /// A rate limit means the key worked. Telling the user it is invalid would
    /// send them to reissue a perfectly good key.
    #[tokio::test]
    async fn a_rate_limit_is_reported_as_valid_but_throttled() {
        let server = responding(429, r#"{"error":{"message":"slow down"}}"#).await;
        let message = probe_key(client(&server)).await.unwrap_err();
        assert!(message.contains("rate-limited"), "{message}");
        assert!(!message.contains("Invalid API key"), "{message}");
    }

    #[tokio::test]
    async fn any_other_failure_is_inconclusive_rather_than_a_verdict() {
        let server = responding(500, "upstream exploded").await;
        let message = probe_key(client(&server)).await.unwrap_err();
        assert!(message.starts_with("Could not verify the key"), "{message}");
        assert!(!message.contains("Invalid API key"), "{message}");
    }

    /// The message goes to a form the user is staring at; echoing the key back
    /// would put it in screenshots and support tickets.
    #[tokio::test]
    async fn no_failure_message_echoes_the_credential() {
        for status in [401, 403, 429, 500] {
            let server = responding(status, &format!("rejected {CREDENTIAL}")).await;
            let message = probe_key(client(&server)).await.unwrap_err();
            assert!(!message.contains(CREDENTIAL), "{status}: {message}");
        }
    }
}
