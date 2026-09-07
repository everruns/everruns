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
    fn typed_credentials_preserve_values_defaults_and_redacted_debug() {
        for (document, region, token) in [
            (
                r#"{"access_key_id":"access-marker","secret_access_key":"secret-marker","session_token":"token-marker","region":"eu-west-1"}"#,
                "eu-west-1",
                Some("token-marker"),
            ),
            (
                r#"{"access_key_id":"access-marker","secret_access_key":"secret-marker"}"#,
                "us-east-1",
                None,
            ),
            (
                r#"{"access_key_id":"access-marker","secret_access_key":"secret-marker","session_token":"","region":""}"#,
                "us-east-1",
                None,
            ),
        ] {
            let credential =
                BedrockCredential::from_driver_config(&config_from_document(document)).unwrap();
            assert_eq!(
                (
                    &*credential.access_key_id,
                    &*credential.secret_access_key,
                    credential.session_token.as_deref(),
                    &*credential.region
                ),
                ("access-marker", "secret-marker", token, region)
            );
            let debug = format!("{credential:?}");
            assert_eq!(
                debug,
                format!(
                    "BedrockCredential {{ access_key_id: \"[REDACTED]\", secret_access_key: \"[REDACTED]\", session_token: {:?}, region: {region:?} }}",
                    token.map(|_| "[REDACTED]")
                )
            );
        }
    }
    #[test]
    fn incomplete_typed_credentials_report_the_missing_field_without_secrets() {
        for (document, field) in [
            ("not-json", "access key ID"),
            ("{}", "access key ID"),
            (r#"{"secret_access_key":"secret-marker"}"#, "access key ID"),
            (
                r#"{"access_key_id":"","secret_access_key":"secret-marker"}"#,
                "access key ID",
            ),
            (r#"{"access_key_id":"access-marker"}"#, "secret access key"),
            (
                r#"{"access_key_id":"access-marker","secret_access_key":""}"#,
                "secret access key",
            ),
        ] {
            let error =
                BedrockCredential::from_driver_config(&config_from_document(document)).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "LLM error: Bedrock provider is missing the AWS {field}. Configure access_key_id and secret_access_key in provider settings."
                )
            );
        }
    }
}
