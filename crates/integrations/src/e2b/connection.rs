// E2B Connection Provider
//
// Decision: API-key-based connection. User obtains key from E2B dashboard.
// Decision: Validate key by calling GET /sandboxes — 200 means valid, 401/403 means invalid.
// Decision: All E2B-specific connection config lives in this crate, not in the server crate.

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use crate::e2b::E2B_API_BASE;

pub struct E2BConnector;

#[async_trait]
impl Connector for E2BConnector {
    fn capabilities(&self) -> &'static [&'static str] {
        &["sandbox_provisioning"]
    }
    fn provider_id(&self) -> &str {
        "e2b"
    }

    fn display_name(&self) -> &str {
        "E2B"
    }

    fn description(&self) -> &str {
        "Cloud sandbox environments powered by E2B"
    }

    fn icon(&self) -> &str {
        "cloud"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .with_placeholder("e2b_..."),
            ],
            instructions_markdown: "\
1. Go to [E2B Dashboard](https://e2b.dev/dashboard)\n\
2. Navigate to **API Keys** in your account settings\n\
3. Click **Create API Key**\n\
4. Copy the key and paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{E2B_API_BASE}/sandboxes"))
            .header("X-API-KEY", credential)
            .send()
            .await
            .map_err(|e| format!("Failed to reach E2B API: {e}"))?;

        match response.status().as_u16() {
            200..=299 => Ok(ConnectorValidation {
                provider_username: None,
                provider_metadata: None,
            }),
            401 | 403 => {
                Err("Invalid API key. Check that the key is correct and active.".to_string())
            }
            status => Err(format!("Unexpected response from E2B API (HTTP {status})")),
        }
    }
}
