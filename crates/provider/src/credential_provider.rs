// Pluggable provider credential source (knowledge/foundations/llm-drivers.md, knowledge/foundations/providers.md)
//
// Driver crates never read the process environment for credentials. Reading
// `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc. from a shared host environment is
// unsafe in the multitenant server: a platform-level key would silently fund
// tenant execution (the fail-closed Key Resolution Contract in
// `knowledge/foundations/llm-drivers.md`).
//
// Instead, credential loading is an explicit, injectable concern. A caller that
// wants env-based credentials — a CLI, a dev entrypoint, a standalone embedder —
// constructs an [`EnvCredentialProvider`] and passes it in. The server never
// constructs one; it resolves credentials from the encrypted database. This is
// the single, common seam across every driver, so adding a new driver does not
// add a new place that touches the environment.
//
// What a driver *does* own is the *names*: each declares its variables on its
// own credential schema (`FormField::env`, `CredentialFormSchema::base_url_env`)
// following its vendor's SDK convention. Declaring a name is inert — it reads
// nothing and is identical on the server, where the declaration is simply never
// consulted.

use std::collections::BTreeMap;

use crate::credential_schema::assemble_credential_document;
use crate::driver_registry::DriverDescriptor;

/// Credentials resolved for a single driver.
///
/// Carries the driver's declared credential fields, in the same field-map shape
/// the operator-entered form produces, plus an optional endpoint override.
/// Multi-field drivers (Bedrock's AWS keys, MAI's Entra OAuth block) are
/// therefore expressible; a single-key driver simply has one `api_key` field.
///
/// `Debug` redacts every field value (EVE-879): resolved credentials flow
/// through host wiring that logs liberally, so secrets must never reach output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderCredentials {
    fields: BTreeMap<String, String>,
    base_url: Option<String>,
}

impl ProviderCredentials {
    /// Assemble credentials from a resolved field map and optional endpoint.
    pub fn new(fields: BTreeMap<String, String>, base_url: Option<String>) -> Self {
        Self { fields, base_url }
    }

    /// The single-field API key, for the common one-key driver.
    pub fn api_key(&self) -> Option<&str> {
        self.field("api_key")
    }

    /// The endpoint override, when one was resolved.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    /// One declared credential field by name (`access_key_id`, `tenant_id`, …).
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// Every resolved field.
    pub fn fields(&self) -> &BTreeMap<String, String> {
        &self.fields
    }

    /// The credential document a driver config carries: the raw key for a
    /// single-key driver, a JSON object for a multi-field one. Identical to
    /// what the server stores, so a driver parses env-resolved and
    /// operator-entered credentials through the same path.
    pub fn document(&self) -> Option<String> {
        assemble_credential_document(&self.fields)
    }

    /// Whether any credential value is present.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.base_url.is_none()
    }
}

impl std::fmt::Debug for ProviderCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCredentials")
            .field(
                "fields",
                &self
                    .fields
                    .keys()
                    .map(|name| (name.as_str(), "[REDACTED]"))
                    .collect::<BTreeMap<_, _>>(),
            )
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// A source of provider credentials, injected by the caller.
///
/// Resolution is driven by the driver's own [`DriverDescriptor`], because only
/// the driver knows which fields it needs and what they are called. Drivers and
/// dev stores depend on this trait, not on the environment. The multitenant
/// server path resolves credentials from the encrypted database and does not
/// use a `CredentialProvider`; only explicit standalone/dev entrypoints
/// construct one (typically [`EnvCredentialProvider`]).
pub trait CredentialProvider: Send + Sync {
    /// Resolve credentials for the given driver, or `None` when this source has
    /// none for it.
    fn resolve(&self, driver: &DriverDescriptor) -> Option<ProviderCredentials>;
}

/// A [`CredentialProvider`] that reads a driver's declared variables from the
/// process environment.
///
/// This is the shared library implementation for env-based credentials and the
/// sanctioned pattern for any caller that wants them: driver/library code never
/// reads credential env vars itself, so new env-credential logic belongs here.
///
/// It is intended for standalone/CLI/dev use and MUST NOT be constructed on
/// org-scoped server execution paths — doing so would reopen the env fallback
/// the Key Resolution Contract forbids. This type is the only place in the
/// workspace that pairs a driver's declared names with a real `std::env::var`
/// lookup, so keeping it out of server wiring is sufficient to keep the
/// environment out of server credential resolution.
///
/// Which variables exist is the driver's decision, not this type's: each
/// declares them on its credential schema following its vendor's own SDK
/// (`ANTHROPIC_API_KEY`, `AWS_ACCESS_KEY_ID`, `AZURE_TENANT_ID`, …). A driver
/// that declares none resolves to `None` here, however its id is spelled.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCredentialProvider;

impl EnvCredentialProvider {
    /// Construct the env-backed credential provider.
    pub fn new() -> Self {
        Self
    }

    /// Resolve a driver's declared variables using an injectable lookup
    /// (testable without touching the real process environment).
    pub fn resolve_with<F>(driver: &DriverDescriptor, lookup: F) -> Option<ProviderCredentials>
    where
        F: Fn(&str) -> Option<String>,
    {
        let credentials = ProviderCredentials::new(
            driver.credential_schema.resolve_from_env(&lookup),
            driver.base_url_from_env(&lookup),
        );
        (!credentials.is_empty()).then_some(credentials)
    }
}

impl CredentialProvider for EnvCredentialProvider {
    fn resolve(&self, driver: &DriverDescriptor) -> Option<ProviderCredentials> {
        Self::resolve_with(driver, |name| std::env::var(name).ok())
    }
}

/// Why a driver could not be configured from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvCredentialError {
    /// None of the driver's declared variables carried a usable credential.
    ///
    /// `variables` is what the driver declares, so the message can name exactly
    /// what to set — possible only because the names belong to the driver
    /// rather than to a central derivation.
    Missing {
        /// The driver that was asked for.
        driver: String,
        /// Every variable the driver declares, most preferred first.
        variables: Vec<String>,
    },
    /// The driver declares no chat service to build a provider from.
    NoChatService {
        /// The driver that was asked for.
        driver: String,
    },
}

impl std::fmt::Display for EnvCredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvCredentialError::Missing { driver, variables } if variables.is_empty() => {
                write!(f, "driver {driver} declares no environment variables")
            }
            EnvCredentialError::Missing { driver, variables } => write!(
                f,
                "no credentials for {driver} in the environment; set {}",
                variables.join(" or ")
            ),
            EnvCredentialError::NoChatService { driver } => {
                write!(f, "driver {driver} does not implement the chat service")
            }
        }
    }
}

impl std::error::Error for EnvCredentialError {}

/// Build a chat [`Provider`](crate::runtime_provider::Provider) for a driver
/// from the variables that driver declares.
///
/// The shared implementation behind every driver crate's `from_env`. Credential
/// values reach the driver through its own registered factory, so an
/// env-configured provider is assembled exactly like an operator-configured
/// one.
///
/// Standalone/CLI/dev only, for the same reason [`EnvCredentialProvider`] is:
/// server paths resolve credentials from storage and must never call this.
pub fn provider_from_env(
    driver: &DriverDescriptor,
    id: impl Into<crate::runtime_provider::ProviderKey>,
) -> Result<crate::runtime_provider::Provider, EnvCredentialError> {
    provider_from_env_with(driver, id, |name| std::env::var(name).ok())
}

/// [`provider_from_env`] against an injectable lookup, so tests never mutate the
/// process environment.
pub fn provider_from_env_with<F>(
    driver: &DriverDescriptor,
    id: impl Into<crate::runtime_provider::ProviderKey>,
    lookup: F,
) -> Result<crate::runtime_provider::Provider, EnvCredentialError>
where
    F: Fn(&str) -> Option<String>,
{
    let factory = driver
        .chat
        .as_ref()
        .ok_or_else(|| EnvCredentialError::NoChatService {
            driver: driver.id.to_string(),
        })?;
    let missing = || EnvCredentialError::Missing {
        driver: driver.id.to_string(),
        variables: driver.declared_env_vars(),
    };
    // An endpoint override alone is not a credential: building a provider from
    // it would defer the real failure to a 401 at the first request, instead of
    // saying here which variable to set. Keyless drivers are not built this way.
    let document = EnvCredentialProvider::resolve_with(driver, lookup)
        .and_then(|credentials| {
            credentials
                .document()
                .map(|document| (document, credentials.base_url().map(str::to_owned)))
        })
        .ok_or_else(missing)?;
    let (document, base_url) = document;

    // The caller's key is the runtime provider identity; the descriptor's id is
    // the driver kind. They are independent, and a model spec selects by the
    // former.
    let id = id.into();
    let config =
        crate::driver_registry::ProviderConfig::for_provider(id.clone(), driver.id.clone())
            .with_api_key(document);
    let config = match base_url {
        Some(base_url) => config.with_base_url(base_url),
        None => config,
    };
    // Through the same typed-field view every other driver-creation path uses,
    // so a multi-field driver reads its credentials exactly as it does on the
    // server.
    let config = crate::driver_registry::DriverConfig::from_provider_config(&config);
    Ok(crate::runtime_provider::Provider::from_driver(
        id,
        factory(&config).into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_schema::{CredentialFormSchema, FormField};
    use crate::driver_registry::BoxedChatDriver;
    use crate::provider::DriverId;

    /// A descriptor carrying only what credential resolution reads.
    fn driver(
        id: &'static str,
        fields: Vec<FormField>,
        base_url_env: Option<&str>,
    ) -> DriverDescriptor {
        DriverDescriptor {
            credential_schema: CredentialFormSchema {
                fields,
                instructions_markdown: String::new(),
            },
            base_url_env: base_url_env.map(str::to_owned),
            ..DriverDescriptor::chat_only(DriverId::external(id), |_| -> BoxedChatDriver {
                unreachable!("credential resolution never constructs the driver")
            })
        }
    }

    fn resolve(driver: &DriverDescriptor, entries: &[(&str, &str)]) -> Option<ProviderCredentials> {
        EnvCredentialProvider::resolve_with(driver, |name| {
            entries
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn a_driver_resolves_only_the_variables_it_declares() {
        let anthropic = driver(
            "anthropic",
            vec![
                FormField::password("api_key", "API Key")
                    .required()
                    .env("ANTHROPIC_API_KEY"),
            ],
            Some("ANTHROPIC_BASE_URL"),
        );
        let resolved = resolve(
            &anthropic,
            &[
                ("ANTHROPIC_API_KEY", "key"),
                ("ANTHROPIC_BASE_URL", "https://proxy.example"),
                ("OPENAI_API_KEY", "someone else's"),
            ],
        )
        .expect("declared variables are set");
        assert_eq!(resolved.api_key(), Some("key"));
        assert_eq!(resolved.base_url(), Some("https://proxy.example"));
        assert_eq!(resolved.document(), Some("key".to_string()));

        // Another vendor's variable is not this driver's credential.
        assert_eq!(resolve(&anthropic, &[("OPENAI_API_KEY", "other")]), None);
        assert_eq!(resolve(&anthropic, &[("ANTHROPIC_API_KEY", "")]), None);
        assert_eq!(resolve(&anthropic, &[]), None);
    }

    #[test]
    fn a_multi_field_driver_resolves_every_declared_field() {
        let bedrock = driver(
            "bedrock",
            vec![
                FormField::password("access_key_id", "Access Key ID")
                    .required()
                    .env("AWS_ACCESS_KEY_ID"),
                FormField::password("secret_access_key", "Secret Access Key")
                    .required()
                    .env("AWS_SECRET_ACCESS_KEY"),
                FormField::text("region", "Region")
                    .env("AWS_REGION")
                    .env_fallback("AWS_DEFAULT_REGION"),
            ],
            None,
        );
        let resolved = resolve(
            &bedrock,
            &[
                ("AWS_ACCESS_KEY_ID", "AKIA"),
                ("AWS_SECRET_ACCESS_KEY", "secret"),
                ("AWS_DEFAULT_REGION", "eu-west-1"),
            ],
        )
        .expect("AWS variables are set");
        assert_eq!(resolved.field("access_key_id"), Some("AKIA"));
        assert_eq!(resolved.field("region"), Some("eu-west-1"));
        // The document is the multi-field shape the driver already parses.
        assert_eq!(
            resolved.document(),
            Some(
                r#"{"access_key_id":"AKIA","region":"eu-west-1","secret_access_key":"secret"}"#
                    .to_string()
            )
        );
    }

    #[test]
    fn a_driver_that_declares_nothing_never_resolves_from_the_environment() {
        // The registry's default schema declares no variable, so a driver that
        // has not opted in cannot be configured by a stray environment value.
        let undeclared = driver(
            "custom-driver.v2",
            vec![FormField::password("api_key", "API Key").required()],
            None,
        );
        assert_eq!(
            resolve(
                &undeclared,
                &[
                    ("API_KEY", "set"),
                    ("CUSTOM_DRIVER_V2_API_KEY", "set"),
                    ("OPENAI_API_KEY", "set"),
                ]
            ),
            None
        );
        assert_eq!(resolve(&driver("llmsim", vec![], None), &[]), None);
    }

    #[test]
    fn provider_from_env_builds_through_the_drivers_own_factory() {
        use crate::driver_registry::{BoxedChatDriver, DriverConfig};

        // A factory that records what reached it, so the test proves the
        // resolved credential travels the same path an operator-entered one
        // does.
        fn factory(config: &DriverConfig) -> BoxedChatDriver {
            assert_eq!(
                config.credentials.get("api_key").map(String::as_str),
                Some("key")
            );
            assert_eq!(config.base_url.as_deref(), Some("https://proxy.example"));
            assert_eq!(config.provider.as_str(), "my-openai");
            crate::runtime_provider::Provider::new(config.provider.clone(), NoopDriver)
                .into_boxed_driver()
        }

        let descriptor = DriverDescriptor {
            credential_schema: CredentialFormSchema::api_key("VENDOR_API_KEY", ""),
            base_url_env: Some("VENDOR_BASE_URL".into()),
            ..DriverDescriptor::chat_only(DriverId::external("vendor"), factory)
        };
        let provider = provider_from_env_with(&descriptor, "my-openai", |name| match name {
            "VENDOR_API_KEY" => Some("key".to_string()),
            "VENDOR_BASE_URL" => Some("https://proxy.example".to_string()),
            _ => None,
        })
        .expect("declared variables are set");
        assert_eq!(provider.id().as_str(), "my-openai");
    }

    #[test]
    fn a_missing_credential_names_the_variables_the_driver_declares() {
        let descriptor = DriverDescriptor {
            credential_schema: CredentialFormSchema::api_key("VENDOR_API_KEY", ""),
            base_url_env: Some("VENDOR_BASE_URL".into()),
            ..DriverDescriptor::chat_only(DriverId::external("vendor"), |_| -> BoxedChatDriver {
                unreachable!("no credential, no driver")
            })
        };
        let error = provider_from_env_with(&descriptor, "vendor", |_| None).unwrap_err();
        assert_eq!(
            error,
            EnvCredentialError::Missing {
                driver: "vendor".to_string(),
                variables: vec!["VENDOR_API_KEY".to_string(), "VENDOR_BASE_URL".to_string()],
            }
        );
        assert_eq!(
            error.to_string(),
            "no credentials for vendor in the environment; set VENDOR_API_KEY or VENDOR_BASE_URL"
        );
    }

    struct NoopDriver;

    #[async_trait::async_trait]
    impl crate::driver_registry::ChatDriver for NoopDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &crate::runtime_provider::ProviderEndpoint,
            _messages: Vec<crate::driver_registry::LlmMessage>,
            _config: &crate::driver_registry::LlmCallConfig,
        ) -> crate::error::Result<crate::driver_registry::LlmResponseStream> {
            unreachable!("credential wiring never sends a request")
        }
    }

    #[test]
    fn debug_output_redacts_every_field_value() {
        let credentials = ProviderCredentials::new(
            BTreeMap::from([
                ("api_key".to_string(), "sk-super-secret".to_string()),
                ("client_secret".to_string(), "also-secret".to_string()),
            ]),
            Some("https://proxy.example".into()),
        );
        assert_eq!(
            format!("{credentials:?}"),
            "ProviderCredentials { fields: {\"api_key\": \"[REDACTED]\", \"client_secret\": \"[REDACTED]\"}, base_url: Some(\"https://proxy.example\") }"
        );
    }
}
