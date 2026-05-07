// Cursor Connection Provider
//
// Decision: API-key connection. Cursor Cloud Agents API keys are created from
// Cursor Dashboard > Cloud Agents > My Settings > API Keys.
// Decision: validate with GET /v0/me; this avoids launching an agent during
// connection setup.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};
use serde_json::json;

use crate::client::CursorClient;

pub struct CursorConnectionProvider;

#[async_trait]
impl ConnectionProvider for CursorConnectionProvider {
    fn provider_id(&self) -> &str {
        "cursor"
    }

    fn display_name(&self) -> &str {
        "Cursor"
    }

    fn description(&self) -> &str {
        "Manage Cursor Cloud Agents for GitHub repositories"
    }

    fn icon(&self) -> &str {
        "code"
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectionFormSchema> {
        Some(ConnectionFormSchema {
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "Cloud Agents API Key".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: Some("cur_...".to_string()),
                help_text: Some(
                    "Use a Cursor Cloud Agents API key, not a general Cursor dashboard key."
                        .to_string(),
                ),
            }],
            instructions_markdown: "\
1. Open [Cursor Dashboard](https://cursor.com/dashboard?tab=cloud-agents)\n\
2. Go to **Cloud Agents** > **My Settings** > **API Keys**\n\
3. Create a Cloud Agents API key\n\
4. Paste it below\n\n\
Cursor must also have GitHub access to any repositories you want agents to work on."
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let api_base = std::env::var(crate::CURSOR_API_BASE_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| crate::CURSOR_API_BASE.to_string());
        let client = CursorClient::with_base_url(credential.to_string(), api_base);
        let info = client.api_key_info().await.map_err(|e| {
            if e.contains("401") || e.contains("403") || e.contains("404") {
                "Invalid Cursor Cloud Agents API key. Create one from Cursor Dashboard > Cloud Agents > My Settings > API Keys.".to_string()
            } else {
                e
            }
        })?;

        Ok(ConnectionValidation {
            provider_username: info.user_email.clone(),
            provider_metadata: Some(json!({
                "api_key_name": info.api_key_name,
                "created_at": info.created_at,
                "user_email": info.user_email,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata() {
        let p = CursorConnectionProvider;
        assert_eq!(p.provider_id(), "cursor");
        assert_eq!(p.display_name(), "Cursor");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "code");
    }

    #[test]
    fn form_schema_mentions_cloud_agents_key() {
        let p = CursorConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(schema.instructions_markdown.contains("Cloud Agents"));
    }
}
