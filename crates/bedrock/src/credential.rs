// Bedrock credential parsing
//
// Credentials are encoded as a JSON object in ProviderConfig.api_key:
//   {"access_key_id":"...","secret_access_key":"...","session_token":"...","region":"..."}
// session_token is optional. region defaults to "us-east-1" when omitted.

use everruns_core::error::{AgentLoopError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockCredential {
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub session_token: Option<String>,
    #[serde(default = "default_region")]
    pub region: String,
}

fn default_region() -> String {
    "us-east-1".to_string()
}

impl BedrockCredential {
    pub fn from_api_key(api_key: &str) -> Result<Self> {
        serde_json::from_str(api_key).map_err(|e| {
            AgentLoopError::llm(format!(
                "Bedrock credentials must be JSON {{access_key_id,secret_access_key}} (region and session_token are optional): {e}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_credential() {
        let json = r#"{"access_key_id":"AKID","secret_access_key":"SECRET","session_token":"TOKEN","region":"eu-west-1"}"#;
        let cred = BedrockCredential::from_api_key(json).unwrap();
        assert_eq!(cred.access_key_id, "AKID");
        assert_eq!(cred.secret_access_key, "SECRET");
        assert_eq!(cred.session_token.as_deref(), Some("TOKEN"));
        assert_eq!(cred.region, "eu-west-1");
    }

    #[test]
    fn test_parse_minimal_credential_defaults_region() {
        let json = r#"{"access_key_id":"AKID","secret_access_key":"SECRET"}"#;
        let cred = BedrockCredential::from_api_key(json).unwrap();
        assert_eq!(cred.region, "us-east-1");
        assert!(cred.session_token.is_none());
    }

    #[test]
    fn test_parse_invalid_json_returns_error() {
        assert!(BedrockCredential::from_api_key("not-json").is_err());
        assert!(BedrockCredential::from_api_key("{}").is_err()); // missing required fields
    }
}
