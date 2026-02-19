// Daytona Connection Provider
//
// Decision: API-key-based connection (not OAuth). User enters key from Daytona dashboard.
// Decision: Validate key by calling GET /sandbox — 200 means valid, 401 means invalid.
// Decision: All Daytona-specific connection config (form schema, instructions, validation)
//   lives in this crate, not in the server crate.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};

use crate::DAYTONA_API_BASE;

pub struct DaytonaConnectionProvider;

#[async_trait]
impl ConnectionProvider for DaytonaConnectionProvider {
    fn provider_id(&self) -> &str {
        "daytona"
    }

    fn display_name(&self) -> &str {
        "Daytona"
    }

    fn description(&self) -> &str {
        "Cloud sandbox environments for code execution"
    }

    fn icon(&self) -> &str {
        "cloud"
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectionFormSchema> {
        Some(ConnectionFormSchema {
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: Some("your-daytona-api-key".to_string()),
                help_text: None,
            }],
            instructions_markdown: "\
1. Go to [Daytona Dashboard](https://app.daytona.io)\n\
2. Navigate to **API Keys** in your account settings\n\
3. Click **Create New API Key**\n\
4. Copy the key and paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{DAYTONA_API_BASE}/sandbox"))
            .bearer_auth(credential)
            .send()
            .await
            .map_err(|e| format!("Failed to reach Daytona API: {e}"))?;

        match response.status().as_u16() {
            200 => Ok(ConnectionValidation {
                provider_username: None,
            }),
            401 | 403 => Err("Invalid API key. Check that the key is correct and active.".into()),
            status => Err(format!(
                "Unexpected response from Daytona API (HTTP {status})"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = DaytonaConnectionProvider;
        assert_eq!(p.provider_id(), "daytona");
        assert_eq!(p.display_name(), "Daytona");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "cloud");
    }

    #[test]
    fn test_form_schema() {
        let p = DaytonaConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(schema.instructions_markdown.contains("app.daytona.io"));
    }
}
