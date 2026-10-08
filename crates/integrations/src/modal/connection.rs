//! Modal connection provider.
//!
//! Decision: Modal authenticates with a token pair (ID `ak-...`, secret
//! `as-...`), but a connection stores exactly one credential string (the
//! `api_key` field) and the leased-resource cleanup worker can only fetch that
//! string. So the form takes the pair in one field, `ak-...:as-...`;
//! `ModalCredentials::parse` also accepts the `modal token set --token-id ...
//! --token-secret ...` line Modal prints, so pasting that works too. Splitting
//! the ID into `provider_metadata` would leave cleanup without it.
//!
//! Decision: validation mints a control-plane auth token (`AuthTokenGet`). It
//! is cheap, has no side effects, and fails exactly when the pair is wrong.

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use super::MODAL_PROVIDER;
use super::client::{ModalClient, ModalCredentials};

/// Connection provider for Modal.
pub struct ModalConnector;

impl ModalConnector {
    async fn verify(credentials: ModalCredentials) -> Result<ConnectorValidation, String> {
        let client = ModalClient::new(credentials)?;
        client.verify_credentials().await?;
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }
}

#[async_trait]
impl Connector for ModalConnector {
    fn capabilities(&self) -> &'static [&'static str] {
        &["sandbox_provisioning"]
    }
    fn provider_id(&self) -> &str {
        MODAL_PROVIDER
    }

    fn display_name(&self) -> &str {
        "Modal"
    }

    fn description(&self) -> &str {
        "Modal sandboxes: full Linux VMs and gVisor containers for code execution"
    }

    fn icon(&self) -> &str {
        // A generic glyph the UI already renders; there is no Modal logo asset.
        "cloud"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "Token")
                    .required()
                    .with_placeholder("ak-...:as-...")
                    .with_help(
                        "Token ID and token secret joined by a colon, or the `modal token set ...` line Modal shows.",
                    ),
            ],
            instructions_markdown: "\
1. Open [Modal settings > API tokens](https://modal.com/settings/tokens) and create a new token\n\
2. Copy the token ID (`ak-...`) and token secret (`as-...`)\n\
3. Paste them below as `ak-...:as-...`"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        Self::verify(ModalCredentials::parse(credential)?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata() {
        let connector = ModalConnector;
        assert_eq!(connector.provider_id(), "modal");
        assert_eq!(connector.display_name(), "Modal");
        assert_eq!(connector.connection_type(), ConnectorType::ApiKey);
    }

    #[test]
    fn form_asks_for_the_pair_in_one_field() {
        let schema = ModalConnector.form_schema().unwrap();
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(
            schema
                .instructions_markdown
                .contains("modal.com/settings/tokens")
        );
    }

    #[tokio::test]
    async fn malformed_pairs_fail_before_any_network_call() {
        let err = ModalConnector
            .validate("as-only-a-secret")
            .await
            .unwrap_err();
        assert!(err.contains("token pair"), "{err}");
    }
}
