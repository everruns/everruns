// Parallel Connection Provider
//
// Decision: API-key connection. Parallel's MCP accepts the key as a bearer
// token on both /mcp and /mcp-oauth, and the free mode remains usable without
// storing any credential.

use async_trait::async_trait;
use everruns_core::{McpToolsListRequest, McpToolsListResponse};
use everruns_platform::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use crate::{PARALLEL_MCP_URL, PARALLEL_PROVIDER_ID};

pub struct ParallelConnector;

#[async_trait]
impl Connector for ParallelConnector {
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

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .with_placeholder("parallel_api_key")
                    .with_help("Parallel is free without a key; add one for authenticated usage."),
            ],
            instructions_markdown: "\
Parallel MCP works without a key by default.\n\n\
To use authenticated mode:\n\
1. Go to [Parallel Platform](https://platform.parallel.ai)\n\
2. Create or copy your API key\n\
3. Paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
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
                Ok(ConnectorValidation {
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
        let p = ParallelConnector;
        assert_eq!(p.provider_id(), "parallel");
        assert_eq!(p.display_name(), "Parallel");
        assert_eq!(p.connection_type(), ConnectorType::ApiKey);
        assert_eq!(p.icon(), "search");
    }

    #[test]
    fn form_schema_describes_optional_key() {
        let p = ParallelConnector;
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
