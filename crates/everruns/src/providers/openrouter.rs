//! OpenRouter provider configuration (requires the `openrouter` feature).
//!
//! [`OpenRouter`] is the value-first provider configuration for reaching models
//! through OpenRouter. Pass it to
//! [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Completion::provider`](crate::llm::Completion::provider), and select the
//! provider-visible model id (`vendor/model`) separately.
//!
//! The `everruns-openrouter` driver is re-exported here
//! ([`OpenRouterChatDriver`], [`register_driver`]) for embedders who need the
//! low-level driver directly.

use std::fmt;

use everruns_provider::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns-openrouter` driver for direct, low-level use.
pub use everruns_openrouter::{OpenRouterChatDriver, register_driver};

/// Why an [`OpenRouter`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenRouterError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the OpenRouter driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for OpenRouterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenRouterError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no OpenRouter credentials in the environment; set {}",
                    vars.join(" or ")
                )
            }
        }
    }
}

impl std::error::Error for OpenRouterError {}

/// OpenRouter provider configuration.
///
/// Build one with [`OpenRouter::new`] (explicit, deterministic — reads no
/// environment) or [`OpenRouter::from_env`] (reads `OPENROUTER_API_KEY` and,
/// when set, `OPENROUTER_BASE_URL`). Select the model separately.
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct OpenRouter {
    api_key: String,
    base_url: Option<String>,
}

impl OpenRouter {
    /// Configure OpenRouter with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from `OPENROUTER_API_KEY` instead.
    ///
    /// ```
    /// use everruns::providers::openrouter::OpenRouter;
    ///
    /// let provider = OpenRouter::new("sk-or-your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure OpenRouter, reading the API key (and optional base URL) from
    /// the environment.
    ///
    /// The variables are the driver's own declaration — `OPENROUTER_API_KEY`
    /// and the optional `OPENROUTER_BASE_URL` — resolved through the shared
    /// [`EnvCredentialProvider`], so they cannot drift from what the driver
    /// reads. An unset or empty key returns
    /// [`OpenRouterError::MissingEnvVar`] rather than panicking.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::openrouter::OpenRouter};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `OPENROUTER_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(OpenRouter::from_env()?)
    ///     .model("openai/gpt-5-mini")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, OpenRouterError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, OpenRouterError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let driver = everruns_openrouter::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| OpenRouterError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }

    /// Override the API base URL (an OpenRouter-compatible proxy or gateway).
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

impl fmt::Debug for OpenRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenRouter")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<OpenRouter> for Provider {
    fn from(config: OpenRouter) -> Self {
        let (api_key, base_url) = config.into_parts();
        let mut provider = everruns_openrouter::provider("openrouter", api_key)
            .with_driver_id(crate::DriverId::OpenRouter);
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
        let (api_key, base_url) = OpenRouter::new("sk-or-explicit").into_parts();
        assert_eq!(api_key, "sk-or-explicit");
        assert_eq!(base_url, None);
    }

    #[test]
    fn from_lookup_reads_key_and_optional_base_url() {
        let (api_key, base_url) = OpenRouter::from_lookup(|name| match name {
            "OPENROUTER_API_KEY" => Some("sk-or-env".to_string()),
            "OPENROUTER_BASE_URL" => Some("https://proxy.example/api/v1".to_string()),
            _ => None,
        })
        .expect("key present")
        .into_parts();
        assert_eq!(api_key, "sk-or-env");
        assert_eq!(base_url, Some("https://proxy.example/api/v1".to_string()));
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = OpenRouter::from_lookup(|_| None).unwrap_err();
        // The names come from the driver's declaration, not from this module.
        assert_eq!(
            err,
            OpenRouterError::MissingEnvVar {
                vars: vec![
                    "OPENROUTER_API_KEY".to_string(),
                    "OPENROUTER_BASE_URL".to_string()
                ],
            }
        );
    }

    #[test]
    fn another_vendors_variable_does_not_configure_openrouter() {
        for other in ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
            assert!(
                OpenRouter::from_lookup(|name| (name == other).then(|| "key".to_string())).is_err(),
                "{other} must not configure OpenRouter"
            );
        }
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!("{:?}", OpenRouter::new("sk-or-secret"));
        assert!(!rendered.contains("sk-or-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}
