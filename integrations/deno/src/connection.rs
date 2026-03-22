// Deno Deploy connection provider.
//
// Decision: this generic connection uses a single organization access token
// (`ddo_...`). Personal tokens (`ddp_...`) that require org metadata are not
// supported here yet.

use crate::client::DenoClient;
use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};

pub struct DenoConnectionProvider;

#[async_trait]
impl ConnectionProvider for DenoConnectionProvider {
    fn provider_id(&self) -> &str {
        "deno"
    }

    fn display_name(&self) -> &str {
        "Deno Deploy"
    }

    fn description(&self) -> &str {
        "Deno Sandboxes for code execution"
    }

    fn icon(&self) -> &str {
        "server"
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectionFormSchema> {
        Some(ConnectionFormSchema {
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "Access Token".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: Some("ddo_...".to_string()),
                help_text: Some("Use an organization token. Personal tokens require org metadata that the generic API-key flow does not store yet.".to_string()),
            }],
            instructions_markdown: "1. Go to [Deno Deploy Sandbox](https://console.deno.com)\n2. Create an organization access token (`ddo_...`)\n3. Paste the token below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        if credential.starts_with("ddp_") {
            return Err(
                "Personal Deno tokens are not supported in the generic connection flow yet. Use an organization token (ddo_...).".to_string(),
            );
        }

        let client = DenoClient::new(credential.to_string(), None);
        client
            .list_sandboxes(&std::collections::BTreeMap::new())
            .await?;
        Ok(ConnectionValidation {
            provider_username: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let provider = DenoConnectionProvider;
        assert_eq!(provider.provider_id(), "deno");
        assert_eq!(provider.display_name(), "Deno Deploy");
        assert_eq!(provider.connection_type(), ConnectionType::ApiKey);
        assert_eq!(provider.icon(), "server");
    }

    #[test]
    fn test_form_schema() {
        let provider = DenoConnectionProvider;
        let schema = provider.form_schema().expect("schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.instructions_markdown.contains("console.deno.com"));
    }
}
