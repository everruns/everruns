// Browserless Connection Provider
//
// Decision: API-token-based connection (not OAuth). User enters token from Browserless dashboard.
// Decision: Validate token by calling GET /active — 200 means valid, 401 means invalid.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};

use crate::BROWSERLESS_API_BASE;

pub struct BrowserlessConnectionProvider;

#[async_trait]
impl ConnectionProvider for BrowserlessConnectionProvider {
    fn provider_id(&self) -> &str {
        "browserless"
    }

    fn display_name(&self) -> &str {
        "Browserless"
    }

    fn description(&self) -> &str {
        "Cloud browser automation for screenshots, scraping, and testing"
    }

    fn icon(&self) -> &str {
        "browserless"
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectionFormSchema> {
        Some(ConnectionFormSchema {
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "API Token".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: Some("your-browserless-api-token".to_string()),
                help_text: None,
            }],
            instructions_markdown: "\
1. Go to [Browserless Dashboard](https://cloud.browserless.io)\n\
2. Navigate to **API Keys** in your account settings\n\
3. Copy your API token and paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{BROWSERLESS_API_BASE}/active?token={credential}"))
            .send()
            .await
            .map_err(|e| format!("Failed to reach Browserless API: {e}"))?;

        match response.status().as_u16() {
            200 => Ok(ConnectionValidation {
                provider_username: None,
            }),
            401 | 403 => {
                Err("Invalid API token. Check that the token is correct and active.".into())
            }
            status => Err(format!(
                "Unexpected response from Browserless API (HTTP {status})"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = BrowserlessConnectionProvider;
        assert_eq!(p.provider_id(), "browserless");
        assert_eq!(p.display_name(), "Browserless");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "browserless");
    }

    #[test]
    fn test_form_schema() {
        let p = BrowserlessConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(
            schema
                .instructions_markdown
                .contains("cloud.browserless.io")
        );
    }
}
