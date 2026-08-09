// Bedrock credential parsing
//
// Credentials are declared as discrete typed fields (see the driver's
// `credential_schema`) and reach the driver as the typed `DriverConfig`
// credential map — `access_key_id`, `secret_access_key`, optional
// `session_token`, optional `region` (defaults to "us-east-1"). The driver no
// longer parses a JSON document out of `api_key`; the credential document is
// parsed into typed fields once, centrally, in `DriverConfig`.

use everruns_provider::driver_registry::DriverConfig;
use everruns_provider::error::{AgentLoopError, Result};

const DEFAULT_REGION: &str = "us-east-1";

#[derive(Clone)]
pub struct BedrockCredential {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
}

impl std::fmt::Debug for BedrockCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BedrockCredential")
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("region", &self.region)
            .finish()
    }
}

impl BedrockCredential {
    /// Construct credentials for a ready-made Bedrock provider.
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
            region: region.into(),
        }
    }

    /// Attach the session token used by temporary AWS credentials.
    pub fn with_session_token(mut self, session_token: impl Into<String>) -> Self {
        self.session_token = Some(session_token.into());
        self
    }

    /// Build a credential from the typed driver credential fields.
    ///
    /// `access_key_id` and `secret_access_key` are required; `region` defaults
    /// to `us-east-1`; `session_token` is only set for temporary credentials.
    pub fn from_driver_config(config: &DriverConfig) -> Result<Self> {
        let access_key_id = config.credential("access_key_id").ok_or_else(|| {
            AgentLoopError::llm(
                "Bedrock provider is missing the AWS access key ID. Configure access_key_id and \
                 secret_access_key in provider settings.",
            )
        })?;
        let secret_access_key = config.credential("secret_access_key").ok_or_else(|| {
            AgentLoopError::llm(
                "Bedrock provider is missing the AWS secret access key. Configure access_key_id \
                 and secret_access_key in provider settings.",
            )
        })?;

        Ok(Self {
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key.to_string(),
            session_token: config.credential("session_token").map(str::to_string),
            region: config
                .credential("region")
                .unwrap_or(DEFAULT_REGION)
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::credential_schema::parse_credential_document;
    use everruns_provider::driver_registry::DriverId;

    fn config_from_document(document: &str) -> DriverConfig {
        DriverConfig {
            provider: everruns_provider::ProviderKey::new("bedrock"),
            provider_type: DriverId::Bedrock,
            credentials: parse_credential_document(Some(document)),
            api_key: Some(document.to_string()),
            base_url: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn test_parse_full_credential() {
        let json = r#"{"access_key_id":"AKID","secret_access_key":"SECRET","session_token":"TOKEN","region":"eu-west-1"}"#;
        let cred = BedrockCredential::from_driver_config(&config_from_document(json)).unwrap();
        assert_eq!(cred.access_key_id, "AKID");
        assert_eq!(cred.secret_access_key, "SECRET");
        assert_eq!(cred.session_token.as_deref(), Some("TOKEN"));
        assert_eq!(cred.region, "eu-west-1");
        let debug = format!("{cred:?}");
        assert!(!debug.contains("AKID"));
        assert!(!debug.contains("SECRET"));
        assert!(!debug.contains("TOKEN"));
    }

    #[test]
    fn test_parse_minimal_credential_defaults_region() {
        let json = r#"{"access_key_id":"AKID","secret_access_key":"SECRET"}"#;
        let cred = BedrockCredential::from_driver_config(&config_from_document(json)).unwrap();
        assert_eq!(cred.region, "us-east-1");
        assert!(cred.session_token.is_none());
    }

    #[test]
    fn test_missing_required_fields_returns_error() {
        // A raw (non-JSON) credential parses to a lone `api_key` field, which is
        // not a valid Bedrock credential.
        assert!(BedrockCredential::from_driver_config(&config_from_document("not-json")).is_err());
        assert!(BedrockCredential::from_driver_config(&config_from_document("{}")).is_err());
    }
}
