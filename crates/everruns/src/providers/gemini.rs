//! Google Gemini provider configuration (requires the `gemini` feature).
//!
//! [`Gemini`] is the value-first provider configuration for talking to
//! Google's Gemini API. Pass it to
//! [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Completion::provider`](crate::llm::Completion::provider), and select the
//! provider-visible model id separately.
//!
//! The `everruns_drivers::gemini` driver is re-exported here
//! ([`GeminiChatDriver`], [`register_driver`]) for embedders who need the
//! low-level driver directly.

use std::fmt;

use everruns_provider::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns_drivers::gemini` driver for direct, low-level use.
pub use everruns_drivers::gemini::{GeminiChatDriver, register_driver};

/// Why an [`Gemini`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeminiError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the Gemini driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for GeminiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeminiError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no Gemini credentials in the environment; set {}",
                    vars.join(" or ")
                )
            }
        }
    }
}

impl std::error::Error for GeminiError {}

/// Gemini provider configuration.
///
/// Build one with [`Gemini::new`] (explicit, deterministic — reads no
/// environment) or [`Gemini::from_env`] (reads `GEMINI_API_KEY`, else
/// `GOOGLE_API_KEY`, and `GEMINI_BASE_URL` when set). Select the model
/// separately.
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct Gemini {
    api_key: String,
    base_url: Option<String>,
}

impl Gemini {
    /// Configure Gemini with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from the environment instead.
    ///
    /// ```
    /// use everruns::providers::gemini::Gemini;
    ///
    /// let provider = Gemini::new("your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure Gemini, reading the API key (and optional base URL) from the
    /// environment.
    ///
    /// The variables are the driver's own declaration — `GEMINI_API_KEY`,
    /// falling back to `GOOGLE_API_KEY` as the `google-genai` SDK does, and
    /// the optional `GEMINI_BASE_URL` — resolved through the shared
    /// [`EnvCredentialProvider`], so they cannot drift from what the driver
    /// reads. An unset or empty key returns [`GeminiError::MissingEnvVar`]
    /// rather than panicking.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::gemini::Gemini};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `GEMINI_API_KEY` or `GOOGLE_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(Gemini::from_env()?)
    ///     .model("gemini-3-flash")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, GeminiError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, GeminiError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let driver = everruns_drivers::gemini::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| GeminiError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }

    /// Override the API base URL (a proxy or gateway that speaks the Gemini
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

impl fmt::Debug for Gemini {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gemini")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<Gemini> for Provider {
    fn from(config: Gemini) -> Self {
        let (api_key, base_url) = config.into_parts();
        let mut provider = everruns_drivers::gemini::provider("gemini", api_key)
            .with_driver_id(crate::DriverId::Gemini);
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
        let (api_key, base_url) = Gemini::new("explicit").into_parts();
        assert_eq!(api_key, "explicit");
        assert_eq!(base_url, None);
    }

    #[test]
    fn from_lookup_reads_key_and_optional_base_url() {
        let (api_key, base_url) = Gemini::from_lookup(|name| match name {
            "GEMINI_API_KEY" => Some("gemini-env".to_string()),
            "GEMINI_BASE_URL" => Some("https://proxy.example/v1beta".to_string()),
            _ => None,
        })
        .expect("key present")
        .into_parts();
        assert_eq!(api_key, "gemini-env");
        assert_eq!(base_url, Some("https://proxy.example/v1beta".to_string()));
    }

    #[test]
    fn from_lookup_falls_back_to_google_api_key() {
        let (api_key, _) = Gemini::from_lookup(|name| {
            (name == "GOOGLE_API_KEY").then(|| "google-env".to_string())
        })
        .expect("fallback key present")
        .into_parts();
        assert_eq!(api_key, "google-env");
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = Gemini::from_lookup(|_| None).unwrap_err();
        let GeminiError::MissingEnvVar { vars } = err;
        assert!(vars.contains(&"GEMINI_API_KEY".to_string()), "{vars:?}");
    }

    #[test]
    fn the_provider_carries_the_gemini_driver() {
        let provider: Provider = Gemini::new("k").into();
        assert_eq!(provider.driver_id(), crate::DriverId::Gemini);
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!("{:?}", Gemini::new("gemini-secret"));
        assert!(!rendered.contains("gemini-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}
