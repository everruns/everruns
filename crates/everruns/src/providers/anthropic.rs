//! Anthropic provider configuration (requires the `anthropic` feature).
//!
//! [`Anthropic`] is the value-first provider configuration for talking to
//! Anthropic's Messages API. Pass it to
//! [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Completion::provider`](crate::llm::Completion::provider), and select the
//! provider-visible model id separately.
//!
//! The `everruns_drivers::anthropic` driver is re-exported here
//! ([`AnthropicChatDriver`], [`register_driver`]) for embedders who need the
//! low-level driver directly.

use std::fmt;

use everruns_provider::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns_drivers::anthropic` driver for direct, low-level use.
pub use everruns_drivers::anthropic::{AnthropicChatDriver, register_driver};

/// Why an [`Anthropic`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnthropicError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the Anthropic driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for AnthropicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnthropicError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no Anthropic credentials in the environment; set {}",
                    vars.join(" or ")
                )
            }
        }
    }
}

impl std::error::Error for AnthropicError {}

/// Anthropic provider configuration.
///
/// Build one with [`Anthropic::new`] (explicit, deterministic — reads no
/// environment) or [`Anthropic::from_env`] (reads `ANTHROPIC_API_KEY`). Select
/// the model separately.
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct Anthropic {
    api_key: String,
    base_url: Option<String>,
}

impl Anthropic {
    /// Configure Anthropic with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from `ANTHROPIC_API_KEY` instead.
    ///
    /// ```
    /// use everruns::providers::anthropic::Anthropic;
    ///
    /// let provider = Anthropic::new("sk-ant-your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure Anthropic, reading the API key from the environment.
    ///
    /// The variable is the driver's own declaration — `ANTHROPIC_API_KEY`,
    /// matching the `anthropic` SDK — resolved through the shared
    /// [`EnvCredentialProvider`], so it cannot drift from what the driver
    /// reads. An unset or empty key returns [`AnthropicError::MissingEnvVar`]
    /// rather than panicking.
    ///
    /// `ANTHROPIC_BASE_URL` is deliberately not read: the SDK's value is the
    /// host root, while [`base_url`](Self::base_url) takes the versioned API
    /// root. Set a proxy explicitly.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::anthropic::Anthropic};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `ANTHROPIC_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(Anthropic::from_env()?)
    ///     .model("claude-sonnet-5")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, AnthropicError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, AnthropicError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let driver = everruns_drivers::anthropic::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| AnthropicError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }

    /// Override the API base URL with the versioned API root of a proxy or
    /// gateway that speaks the Messages API, such as `https://proxy.example/v1`.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Consume the config into its provider assembly parts. Crate-internal so
    /// the public surface never leaks the raw key.
    fn into_parts(self) -> (String, Option<String>) {
        (self.api_key, self.base_url)
    }
}

impl fmt::Debug for Anthropic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Anthropic")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<Anthropic> for Provider {
    fn from(config: Anthropic) -> Self {
        let (api_key, base_url) = config.into_parts();
        let mut provider = everruns_drivers::anthropic::provider("anthropic", api_key)
            .with_driver_id(crate::DriverId::Anthropic);
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
    fn new_is_environment_free_and_sets_no_base_url() {
        let (api_key, base_url) = Anthropic::new("sk-ant-explicit").into_parts();
        assert_eq!(api_key, "sk-ant-explicit");
        assert_eq!(base_url, None);
    }

    #[test]
    fn from_lookup_reads_the_key_and_ignores_the_sdk_base_url() {
        let (api_key, base_url) = Anthropic::from_lookup(|name| match name {
            "ANTHROPIC_API_KEY" => Some("sk-ant-env".to_string()),
            "ANTHROPIC_BASE_URL" => Some("https://proxy.example".to_string()),
            _ => None,
        })
        .expect("key present")
        .into_parts();
        assert_eq!(api_key, "sk-ant-env");
        assert_eq!(base_url, None, "the SDK's host-root value is not imported");
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = Anthropic::from_lookup(|_| None).unwrap_err();
        assert_eq!(
            err,
            AnthropicError::MissingEnvVar {
                vars: vec!["ANTHROPIC_API_KEY".to_string()],
            }
        );
    }

    #[test]
    fn another_vendors_variable_does_not_configure_anthropic() {
        for other in ["OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
            assert!(
                Anthropic::from_lookup(|name| (name == other).then(|| "key".to_string())).is_err(),
                "{other} must not configure Anthropic"
            );
        }
    }

    #[test]
    fn the_provider_carries_the_anthropic_driver() {
        let provider: Provider = Anthropic::new("k")
            .base_url("https://proxy.example/v1")
            .into();
        assert_eq!(provider.driver_id(), crate::DriverId::Anthropic);
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!("{:?}", Anthropic::new("sk-ant-secret"));
        assert!(!rendered.contains("sk-ant-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}
