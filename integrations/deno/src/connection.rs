// Deno Deploy connection provider.
//
// Supports both organization tokens (`ddo_...`) and personal tokens (`ddp_...`).
// Personal tokens require an org slug, stored as provider_metadata.

use crate::client::DenoClient;
use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};
use std::collections::HashMap;

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
            fields: vec![
                FormField {
                    name: "api_key".to_string(),
                    label: "Access Token".to_string(),
                    field_type: FieldType::Password,
                    required: true,
                    placeholder: Some("ddo_... or ddp_...".to_string()),
                    help_text: Some(
                        "Organization token (ddo_...) or personal token (ddp_...). Personal tokens require the organization slug below."
                            .to_string(),
                    ),
                },
                FormField {
                    name: "org_slug".to_string(),
                    label: "Organization Slug".to_string(),
                    field_type: FieldType::Text,
                    required: false,
                    placeholder: Some("my-org".to_string()),
                    help_text: Some(
                        "Required for personal tokens (ddp_...). Find it in your Deno Deploy dashboard URL."
                            .to_string(),
                    ),
                },
            ],
            instructions_markdown:
                "1. Go to [Deno Deploy Sandbox](https://console.deno.com)\n2. Create an access token (`ddo_...` or `ddp_...`)\n3. Paste the token below"
                    .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        // Single-field validation: only org tokens work without extra fields.
        if credential.starts_with("ddp_") {
            return Err(
                "Personal Deno tokens require an organization slug. Use the full form with the org slug field, or use an organization token (ddo_...).".to_string(),
            );
        }

        let client = DenoClient::new(credential.to_string(), None);
        client
            .list_sandboxes(&std::collections::BTreeMap::new())
            .await?;
        Ok(ConnectionValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }

    async fn validate_fields(
        &self,
        fields: &HashMap<String, String>,
    ) -> Result<ConnectionValidation, String> {
        let api_key = fields.get("api_key").map(|s| s.as_str()).unwrap_or("");
        if api_key.is_empty() {
            return Err("Access token is required.".to_string());
        }

        let org_slug = fields
            .get("org_slug")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if api_key.starts_with("ddp_") && org_slug.is_none() {
            return Err("Personal Deno tokens (ddp_...) require an organization slug.".to_string());
        }

        let org_for_client = if api_key.starts_with("ddp_") {
            org_slug.clone()
        } else {
            None
        };

        let client = DenoClient::new(api_key.to_string(), org_for_client);
        client
            .list_sandboxes(&std::collections::BTreeMap::new())
            .await?;

        let provider_metadata = org_slug.map(|slug| serde_json::json!({ "org": slug }));

        Ok(ConnectionValidation {
            provider_username: None,
            provider_metadata,
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
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "api_key");
        assert_eq!(schema.fields[1].name, "org_slug");
        assert!(schema.instructions_markdown.contains("console.deno.com"));
    }

    #[tokio::test]
    async fn test_validate_rejects_personal_token_without_org() {
        let provider = DenoConnectionProvider;
        let err = provider.validate("ddp_test_token").await.unwrap_err();
        assert!(err.contains("organization slug"));
    }

    #[tokio::test]
    async fn test_validate_fields_rejects_personal_token_without_org() {
        let provider = DenoConnectionProvider;
        let mut fields = HashMap::new();
        fields.insert("api_key".to_string(), "ddp_test_token".to_string());
        let err = provider.validate_fields(&fields).await.unwrap_err();
        assert!(err.contains("organization slug"));
    }
}
