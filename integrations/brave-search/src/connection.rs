// Brave Search Connection Provider
//
// Decision: API-key-based connection (not OAuth). User enters key from brave.com/search/api.
// Decision: Validate key by calling GET /web/search?q=test — 200 means valid, 401/403 means invalid.
// Decision: All Brave Search connection config (form schema, instructions, validation)
//   lives in this crate, not in the server crate.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};

use crate::BRAVE_SEARCH_API_BASE;

pub struct BraveSearchConnectionProvider;

#[async_trait]
impl ConnectionProvider for BraveSearchConnectionProvider {
    fn provider_id(&self) -> &str {
        "brave_search"
    }

    fn display_name(&self) -> &str {
        "Brave Search"
    }

    fn description(&self) -> &str {
        "Web search for agents via Brave Search API"
    }

    fn icon(&self) -> &str {
        "search"
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
                placeholder: Some("BSA...".to_string()),
                help_text: None,
            }],
            instructions_markdown: "\
1. Go to [Brave Search API](https://brave.com/search/api/)\n\
2. Sign up for a **free** plan (2,000 queries/month)\n\
3. Copy your **API Key** from the dashboard\n\
4. Paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{BRAVE_SEARCH_API_BASE}/web/search?q=test&count=1"))
            .header("X-Subscription-Token", credential)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Failed to reach Brave Search API: {e}"))?;

        match response.status().as_u16() {
            200 => Ok(ConnectionValidation {
                provider_username: None,
                provider_metadata: None,
            }),
            401 | 403 => Err("Invalid API key. Check that the key is correct and active.".into()),
            429 => Err("API key is valid but rate-limited. Try again in a moment.".into()),
            status => Err(format!(
                "Unexpected response from Brave Search API (HTTP {status})"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = BraveSearchConnectionProvider;
        assert_eq!(p.provider_id(), "brave_search");
        assert_eq!(p.display_name(), "Brave Search");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "search");
    }

    #[test]
    fn test_form_schema() {
        let p = BraveSearchConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(schema.instructions_markdown.contains("brave.com"));
    }
}
