// Sprites Connection Provider
//
// Decision: Bearer-token-based connection (not OAuth). User gets token from Sprites CLI or dashboard.
// Decision: Validate token by calling GET /v1/sprites — 200 means valid, 401 means invalid.

use async_trait::async_trait;
use everruns_platform::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use crate::SPRITES_API_BASE;

pub struct SpritesConnector;

#[async_trait]
impl Connector for SpritesConnector {
    fn provider_id(&self) -> &str {
        "sprites"
    }

    fn display_name(&self) -> &str {
        "Sprites"
    }

    fn description(&self) -> &str {
        "Persistent, hardware-isolated Linux microVMs for code execution"
    }

    fn icon(&self) -> &str {
        "sprites"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Token")
                    .required()
                    .with_placeholder("your-sprites-api-token"),
            ],
            instructions_markdown: "\
1. Install the Sprites CLI: `curl https://sprites.dev/install.sh | bash`\n\
2. Run `sprite login` to authenticate\n\
3. Copy your token from the CLI output or dashboard\n\
4. Paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{SPRITES_API_BASE}/sprites"))
            .bearer_auth(credential)
            .send()
            .await
            .map_err(|e| format!("Failed to reach Sprites API: {e}"))?;

        match response.status().as_u16() {
            200 => Ok(ConnectorValidation {
                provider_username: None,
                provider_metadata: None,
            }),
            401 | 403 => {
                Err("Invalid API token. Check that the token is correct and active.".into())
            }
            status => Err(format!(
                "Unexpected response from Sprites API (HTTP {status})"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = SpritesConnector;
        assert_eq!(p.provider_id(), "sprites");
        assert_eq!(p.display_name(), "Sprites");
        assert_eq!(p.connection_type(), ConnectorType::ApiKey);
        assert_eq!(p.icon(), "sprites");
    }

    #[test]
    fn test_form_schema() {
        let p = SpritesConnector;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(schema.instructions_markdown.contains("sprites.dev"));
    }
}
