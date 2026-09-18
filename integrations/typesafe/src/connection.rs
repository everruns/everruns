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
        "TypeSafeAI"
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
        // One noul over a two-word state: the smallest request the API accepts.
        let client = TypeSafeAIClient::builder(credential)
            .retry(RetryPolicy::none())
            .build();
        let probe = Evaluation::new("connection check")
            .ask("ok", Question::noul("Is this text in English?"));

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_metadata_matches_the_capability() {
        let connector = TypeSafeAIConnector;
        assert_eq!(connector.provider_id(), crate::TYPESAFE_CONNECTION_PROVIDER);
        assert_eq!(connector.display_name(), "TypeSafeAI");
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
}
