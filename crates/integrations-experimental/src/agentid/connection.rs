//! AgentMail connection for the agent's service account.
//!
//! Decision: validation checks format only and makes no network call. The
//! only AgentMail call this integration makes is the authorize request, so a
//! saved connection never touches AgentMail until an agent signs in somewhere.
//! A wrong key or inbox surfaces as the tool's error on first use.

use std::collections::HashMap;

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};

use super::AGENTMAIL_PROVIDER;

/// Longest inbox id accepted (an email address).
const MAX_INBOX_ID_LEN: usize = 254;
/// Longest API key accepted.
const MAX_API_KEY_LEN: usize = 512;

/// AgentMail API key plus the inbox the agent signs in with.
pub struct AgentMailConnector;

#[async_trait]
impl Connector for AgentMailConnector {
    fn provider_id(&self) -> &str {
        AGENTMAIL_PROVIDER
    }

    fn display_name(&self) -> &str {
        "AgentMail"
    }

    fn description(&self) -> &str {
        "AgentMail inbox an agent uses to sign in to apps with AgentID"
    }

    fn icon(&self) -> &str {
        "agentmail"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .with_placeholder("am_..."),
                FormField::text("inbox_id", "Inbox")
                    .required()
                    .with_placeholder("research@acme.agentmail.to")
                    .with_help("The inbox registered with AgentID that this agent signs in as."),
            ],
            instructions_markdown: "\
1. Create an API key in the AgentMail console\n\
2. Enter the inbox this agent signs in with (its AgentID email)\n\
3. Connect it to the agent's service account. End users never get this connection."
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        check_api_key(credential)?;
        Err("Enter the AgentMail inbox as well as the API key.".into())
    }

    async fn validate_fields(
        &self,
        fields: &HashMap<String, String>,
    ) -> Result<ConnectorValidation, String> {
        let api_key = fields.get("api_key").map(|s| s.trim()).unwrap_or("");
        check_api_key(api_key)?;
        let inbox_id = fields.get("inbox_id").map(|s| s.trim()).unwrap_or("");
        check_inbox_id(inbox_id)?;
        Ok(ConnectorValidation {
            provider_username: Some(inbox_id.to_string()),
            provider_metadata: Some(serde_json::json!({ "inbox_id": inbox_id })),
        })
    }
}

fn check_api_key(api_key: &str) -> Result<(), String> {
    if api_key.is_empty() {
        return Err("Enter an AgentMail API key.".into());
    }
    if api_key.len() > MAX_API_KEY_LEN
        || api_key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("That does not look like an AgentMail API key.".into());
    }
    Ok(())
}

pub(crate) fn check_inbox_id(inbox_id: &str) -> Result<(), String> {
    let valid = !inbox_id.is_empty()
        && inbox_id.len() <= MAX_INBOX_ID_LEN
        && inbox_id.contains('@')
        && !inbox_id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '#' | '\\'));
    if valid {
        Ok(())
    } else {
        Err("Enter the inbox as an email address, for example research@acme.agentmail.to.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(api_key: &str, inbox_id: &str) -> HashMap<String, String> {
        HashMap::from([
            ("api_key".to_string(), api_key.to_string()),
            ("inbox_id".to_string(), inbox_id.to_string()),
        ])
    }

    #[tokio::test]
    async fn a_key_and_inbox_save_the_inbox_as_metadata() {
        let saved = AgentMailConnector
            .validate_fields(&fields("am_key", " research@acme.agentmail.to "))
            .await
            .unwrap();
        assert_eq!(
            saved.provider_metadata,
            Some(serde_json::json!({ "inbox_id": "research@acme.agentmail.to" }))
        );
    }

    #[tokio::test]
    async fn a_missing_or_path_like_inbox_is_refused() {
        let connector = AgentMailConnector;
        assert!(
            connector
                .validate_fields(&fields("am_key", ""))
                .await
                .is_err()
        );
        assert!(
            connector
                .validate_fields(&fields("am_key", "a@b/../../admin"))
                .await
                .is_err()
        );
        assert!(
            connector
                .validate_fields(&fields("", "a@b.to"))
                .await
                .is_err()
        );
        assert!(connector.validate("am_key").await.is_err());
    }

    #[test]
    fn the_form_asks_for_key_and_inbox() {
        let schema = AgentMailConnector.form_schema().unwrap();
        let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["api_key", "inbox_id"]);
    }
}
