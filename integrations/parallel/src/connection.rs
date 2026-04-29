// Parallel Connection Provider
//
// Decision: API-key connection. Parallel's MCP accepts the key as a bearer
// token on both /mcp and /mcp-oauth, and the free mode remains usable without
// storing any credential.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};
use everruns_core::{McpToolsListRequest, McpToolsListResponse};

use crate::{PARALLEL_MCP_URL, PARALLEL_PROVIDER_ID};

pub struct ParallelConnectionProvider;

#[async_trait]
impl ConnectionProvider for ParallelConnectionProvider {
    fn provider_id(&self) -> &str {
        PARALLEL_PROVIDER_ID
    }

    fn display_name(&self) -> &str {
        "Parallel"
    }

    fn description(&self) -> &str {
        "Optional Parallel API key for higher Search MCP limits and authenticated MCP endpoints"
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
                placeholder: Some("parallel_api_key".to_string()),
                help_text: Some(
                    "Parallel is free without a key; add one for authenticated usage.".to_string(),
                ),
            }],
            instructions_markdown: "\
Parallel MCP works without a key by default.\n\n\
To use authenticated mode:\n\
1. Go to [Parallel Platform](https://platform.parallel.ai)\n\
2. Create or copy your API key\n\
3. Paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let response = reqwest::Client::new()
            .post(PARALLEL_MCP_URL)
            .bearer_auth(credential)
            .json(&McpToolsListRequest::default())
            .send()
            .await
            .map_err(|e| format!("Failed to reach Parallel MCP: {e}"))?;

        match response.status().as_u16() {
            200 => {
                let mcp_response: McpToolsListResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Invalid Parallel MCP response: {e}"))?;
                if let Some(error) = mcp_response.error {
                    return Err(format!("Parallel MCP error: {}", error.message));
                }
                Ok(ConnectionValidation {
                    provider_username: None,
                    provider_metadata: None,
                })
            }
            401 | 403 => Err("Invalid Parallel API key.".to_string()),
            429 => Err("Parallel API key is valid but rate-limited. Try again later.".to_string()),
            status => Err(format!(
                "Unexpected response from Parallel MCP (HTTP {status})"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata() {
        let p = ParallelConnectionProvider;
        assert_eq!(p.provider_id(), "parallel");
        assert_eq!(p.display_name(), "Parallel");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "search");
    }

    #[test]
    fn form_schema_describes_optional_key() {
        let p = ParallelConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.instructions_markdown.contains("without a key"));
        assert!(
            schema
                .instructions_markdown
                .contains("platform.parallel.ai")
        );
    }
}
