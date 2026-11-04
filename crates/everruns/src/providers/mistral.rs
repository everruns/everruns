//! Mistral provider configuration (requires the `mistral` feature).
//!
//! [`Mistral`] is the value-first provider configuration for talking to
//! Mistral API. Pass it to
//! [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Completion::provider`](crate::llm::Completion::provider), and select the
//! provider-visible model id separately.
//!
//! The `everruns_drivers::mistral` driver is re-exported here
//! ([`MistralChatDriver`], [`register_driver`]) for embedders who need the
//! low-level driver directly.

use std::fmt;

use everruns_contracts::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns_drivers::mistral` driver for direct, low-level use.
pub use everruns_drivers::mistral::{MistralApi, MistralChatDriver, register_driver};

/// Why an [`Mistral`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MistralError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the Mistral driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for MistralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MistralError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no Mistral credentials in the environment; set {}",
                    vars.join(" or ")
                )
            }
        }
    }
}

impl std::error::Error for MistralError {}

/// Mistral provider configuration.
///
/// Build one with [`Mistral::new`] (explicit, deterministic — reads no
/// environment) or [`Mistral::from_env`] (reads `MISTRAL_API_KEY`, and `MISTRAL_BASE_URL` when set). Select the model
/// separately.
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct Mistral {
    api_key: String,
    base_url: Option<String>,
    api: MistralApi,
}

impl Mistral {
    /// Configure Mistral with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from the environment instead.
    ///
    /// ```
    /// use everruns::providers::mistral::Mistral;
    ///
    /// let provider = Mistral::new("your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
            api: MistralApi::default(),
        }
    }

    /// Configure Mistral, reading the API key (and optional base URL) from the
    /// environment.
    ///
    /// The variables are the driver's own declaration — `MISTRAL_API_KEY`, and
    /// the optional `MISTRAL_BASE_URL` — resolved through the shared
    /// [`EnvCredentialProvider`], so they cannot drift from what the driver
    /// reads. An unset or empty key returns [`MistralError::MissingEnvVar`]
    /// rather than panicking.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::mistral::Mistral};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `MISTRAL_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(Mistral::from_env()?)
    ///     .model("your-model-id")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, MistralError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, MistralError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let driver = everruns_drivers::mistral::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| MistralError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
            api: MistralApi::default(),
        })
    }

    /// Select the chat API for this provider connection.
    ///
    /// ```
    /// use everruns::{Mistral, MistralApi};
    /// let provider = Mistral::new("key").api(MistralApi::Responses);
    /// ```
    pub fn api(mut self, api: MistralApi) -> Self {
        self.api = api;
        self
    }

    /// Override the API base URL (a proxy or gateway that speaks the Mistral
    /// API).
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Consume the config into its provider assembly parts. Crate-internal so
    /// the public surface never leaks the raw key.
    fn into_parts(self) -> (String, Option<String>, MistralApi) {
        (self.api_key, self.base_url, self.api)
    }
}

impl fmt::Debug for Mistral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mistral")
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .field("api", &self.api)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<Mistral> for Provider {
    fn from(config: Mistral) -> Self {
        let (api_key, base_url, api) = config.into_parts();
        let mut provider = everruns_drivers::mistral::provider_with_api("mistral", api_key, api)
            .with_driver_id(crate::DriverId::Mistral);
        if let Some(base_url) = base_url {
            provider = provider.base_url(base_url);
        }
        provider
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_lookup_reads_key_and_optional_base_url() {
        let (api_key, base_url, _) = Mistral::from_lookup(|name| match name {
            "MISTRAL_API_KEY" => Some("mistral-env".to_string()),
            "MISTRAL_BASE_URL" => Some("https://proxy.example/v1beta".to_string()),
            _ => None,
        })
        .expect("key present")
        .into_parts();
        assert_eq!(api_key, "mistral-env");
        assert_eq!(base_url, Some("https://proxy.example/v1beta".to_string()));
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = Mistral::from_lookup(|_| None).unwrap_err();
        let MistralError::MissingEnvVar { vars } = err;
        assert!(vars.contains(&"MISTRAL_API_KEY".to_string()), "{vars:?}");
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!("{:?}", Mistral::new("mistral-secret"));
        assert!(!rendered.contains("mistral-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}
