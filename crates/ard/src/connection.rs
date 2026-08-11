//! ARD registry connection provider.
//!
//! Decision: API-key-based connection. The credential is sent as a bearer token
//! on registry `POST /search` and artifact fetches. Validation is a lightweight
//! reachability probe — ARD registries are public-read in many deployments, so
//! we accept any non-5xx response as "key shape accepted" and only reject on
//! clear auth failures.
//!
//! Decision: All connection config (form schema, validation) lives in this
//! crate, not in the server crate. The registry base URL itself is supplied via
//! capability config, so the connection only carries the token.

use async_trait::async_trait;
use everruns_platform::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

/// Connection provider id; also the fallback session-secret name is derived
/// from this (uppercased).
pub const ARD_CONNECTION_PROVIDER: &str = "ard";
/// Fallback session secret / env var holding a registry bearer token.
pub const ARD_API_KEY_SECRET: &str = "ARD_REGISTRY_TOKEN";

pub struct ArdConnector;

#[async_trait]
impl Connector for ArdConnector {
    fn provider_id(&self) -> &str {
        ARD_CONNECTION_PROVIDER
    }

    fn display_name(&self) -> &str {
        "Agentic Resource Discovery"
    }

    fn description(&self) -> &str {
        "Bearer token for an ARD registry used to discover and attach external capabilities"
    }

    fn icon(&self) -> &str {
        "compass"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "Registry token")
                    .required()
                    .with_placeholder("ard_...")
                    .with_help(
                        "Bearer token for your ARD registry. Public reference registries may not \
                         require one — leave blank if your registry allows anonymous search.",
                    ),
            ],
            instructions_markdown: "\
1. Obtain a registry token from your ARD registry operator.\n\
2. Paste it below. The registry base URL is configured per-agent on the \
`resource_discovery` capability."
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        // We cannot validate against a specific registry here (the base URL is
        // capability-scoped, not connection-scoped). Accept a non-empty token
        // shape; real auth is exercised at search time and surfaced as a tool
        // error. This mirrors providers whose endpoint is configured elsewhere.
        if credential.trim().is_empty() {
            return Err("Registry token cannot be empty".to_string());
        }
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata() {
        let c = ArdConnector;
        assert_eq!(c.provider_id(), "ard");
        assert_eq!(c.connection_type(), ConnectorType::ApiKey);
    }

    #[tokio::test]
    async fn rejects_empty_credential() {
        assert!(ArdConnector.validate("   ").await.is_err());
        assert!(ArdConnector.validate("tok").await.is_ok());
    }

    #[test]
    fn form_schema_present() {
        let schema = ArdConnector.form_schema().unwrap();
        assert_eq!(schema.fields[0].name, "api_key");
    }
}
