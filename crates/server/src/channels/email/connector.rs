//! Separate from AgentID's service-account connection: this account grants
//! organization inbox transport and does not change AgentID credential scope.
use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};
pub struct AgentMailInboxConnector;
#[async_trait]
impl Connector for AgentMailInboxConnector {
    fn provider_id(&self) -> &str {
        "agentmail_inbox"
    }
    fn display_name(&self) -> &str {
        "AgentMail inboxes"
    }
    fn description(&self) -> &str {
        "Organization email inbox provider account"
    }
    fn icon(&self) -> &str {
        "mail"
    }
    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }
    fn capabilities(&self) -> &'static [&'static str] {
        &["email_inbox"]
    }
    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema { fields: vec![FormField::password("api_key","API key").required().with_placeholder("am_...")], instructions_markdown: "Connect an AgentMail API key, then register its inboxes under Email. A saved key has not yet passed a delivery test.".into() })
    }
    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        if credential.is_empty()
            || credential.len() > 512
            || credential
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err("Enter an AgentMail API key".into());
        }
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: Some(serde_json::json!({"verification":"format_only"})),
        })
    }
}
