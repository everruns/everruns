//! Groq provider configuration (requires the `groq` feature).
//!
//! [`Groq`] is the value-first provider configuration for talking to
//! Groq API. Pass it to
//! [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Completion::provider`](crate::llm::Completion::provider), and select the
//! provider-visible model id separately.
//!
//! The `everruns_drivers::groq` driver is re-exported here
//! ([`GroqChatDriver`], [`register_driver`]) for embedders who need the
//! low-level driver directly.

use std::fmt;

use everruns_contracts::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns_drivers::groq` driver for direct, low-level use.
pub use everruns_drivers::groq::{GroqChatDriver, register_driver};

/// Why an [`Groq`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GroqError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the Groq driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for GroqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GroqError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no Groq credentials in the environment; set {}",
                    vars.join(" or ")
                )
            }
        }
    }
}

impl std::error::Error for GroqError {}

/// Groq provider configuration.
///
/// Build one with [`Groq::new`] (explicit, deterministic — reads no
/// environment) or [`Groq::from_env`] (reads `GROQ_API_KEY`, and `GROQ_BASE_URL` when set). Select the model
/// separately.
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct Groq {
    api_key: String,
    base_url: Option<String>,
}

impl Groq {
    /// Configure Groq with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from the environment instead.
    ///
    /// ```
    /// use everruns::providers::groq::Groq;
    ///
    /// let provider = Groq::new("your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure Groq, reading the API key (and optional base URL) from the
    /// environment.
    ///
    /// The variables are the driver's own declaration — `GROQ_API_KEY`, and
    /// the optional `GROQ_BASE_URL` — resolved through the shared
    /// [`EnvCredentialProvider`], so they cannot drift from what the driver
    /// reads. An unset or empty key returns [`GroqError::MissingEnvVar`]
    /// rather than panicking.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::groq::Groq};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `GROQ_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(Groq::from_env()?)
    ///     .model("your-model-id")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, GroqError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, GroqError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let driver = everruns_drivers::groq::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| GroqError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }

    /// Override the API base URL (a proxy or gateway that speaks the Groq
    /// API).
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

impl fmt::Debug for Groq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Groq")
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<Groq> for Provider {
    fn from(config: Groq) -> Self {
        let (api_key, base_url) = config.into_parts();
        let mut provider =
            everruns_drivers::groq::provider("groq", api_key).with_driver_id(crate::DriverId::Groq);
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
        let (api_key, base_url) = Groq::from_lookup(|name| match name {
            "GROQ_API_KEY" => Some("groq-env".to_string()),
            "GROQ_BASE_URL" => Some("https://proxy.example/v1beta".to_string()),
            _ => None,
        })
        .expect("key present")
        .into_parts();
        assert_eq!(api_key, "groq-env");
        assert_eq!(base_url, Some("https://proxy.example/v1beta".to_string()));
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = Groq::from_lookup(|_| None).unwrap_err();
        let GroqError::MissingEnvVar { vars } = err;
        assert!(vars.contains(&"GROQ_API_KEY".to_string()), "{vars:?}");
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!("{:?}", Groq::new("groq-secret"));
        assert!(!rendered.contains("groq-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}
