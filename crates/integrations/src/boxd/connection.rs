//! The boxd connection provider.
//!
//! Decisions:
//! - API key form (`bxd_...`), not OAuth: boxd issues keys per org from its
//!   console, and a key is fenced to that one org.
//! - Validation exchanges the key for a JWT and calls `Whoami`, the cheapest
//!   call that proves both the key and the gRPC endpoint work.

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use super::client::BoxdClient;

/// The `boxd` connector.
pub struct BoxdConnector;

#[async_trait]
impl Connector for BoxdConnector {
    fn capabilities(&self) -> &'static [&'static str] {
        &["sandbox_provisioning"]
    }

    fn provider_id(&self) -> &str {
        super::BOXD_PROVIDER
    }

    fn display_name(&self) -> &str {
        "boxd"
    }

    fn description(&self) -> &str {
        "Persistent, hardware-isolated Linux VMs for code execution"
    }

    fn icon(&self) -> &str {
        "boxd"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .with_placeholder("bxd_..."),
            ],
            instructions_markdown: "\
1. Go to [boxd API keys](https://app.boxd.sh/keys)\n\
2. Create a key in the organization the machines should bill to\n\
3. Copy the key (it is shown once) and paste it below\n\n\
Machines Everruns creates are isolated from your other boxd machines."
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        if !credential.trim().starts_with("bxd_") {
            return Err("A boxd API key starts with bxd_.".to_string());
        }
        BoxdClient::new(credential)?.verify().await?;
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keys_without_the_boxd_prefix_are_refused_before_any_request() {
        let error = BoxdConnector.validate("sk-not-boxd").await.unwrap_err();
        assert!(error.contains("bxd_"), "{error}");
    }

    #[test]
    fn the_form_asks_for_one_secret_key() {
        let form = BoxdConnector.form_schema().unwrap();
        assert_eq!(form.fields.len(), 1);
        assert!(form.instructions_markdown.contains("app.boxd.sh/keys"));
    }
}
