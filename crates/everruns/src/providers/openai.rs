//! OpenAI provider configuration (requires the `openai` feature).
//!
//! [`OpenAI`] is the value-first provider configuration for talking to OpenAI's
//! Responses API. Pass it to [`AgentBuilder::provider`](crate::AgentBuilder::provider)
//! and select the provider-visible model id separately.
//!
//! The existing `everruns-openai` drivers are re-exported here
//! ([`OpenAIChatDriver`], [`OpenAICompletionsChatDriver`], [`register_driver`])
//! for embedders who need the low-level driver directly.

use std::fmt;

use crate::Provider;

/// API key environment variable, matching the repo-wide convention documented on
/// `everruns_provider::credential_provider::EnvCredentialProvider`.
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
/// Optional base-URL override environment variable (OpenAI-compatible proxies,
/// self-hosted endpoints, Azure gateways).
const OPENAI_BASE_URL_ENV: &str = "OPENAI_BASE_URL";

/// Re-exported `everruns-openai` drivers for direct, low-level use.
pub use everruns_openai::{OpenAIChatDriver, OpenAICompletionsChatDriver, register_driver};

/// Why an [`OpenAI`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the name of the missing environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenAIError {
    /// A required environment variable was unset or empty.
    MissingEnvVar {
        /// The variable name that was expected (e.g. `OPENAI_API_KEY`).
        var: &'static str,
    },
}

impl fmt::Display for OpenAIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenAIError::MissingEnvVar { var } => {
                write!(f, "required environment variable {var} is not set")
            }
        }
    }
}

impl std::error::Error for OpenAIError {}

/// OpenAI provider configuration.
///
/// Build one with [`OpenAI::new`] (explicit, deterministic — reads no environment)
/// or [`OpenAI::from_env`] (reads `OPENAI_API_KEY` and, when set,
/// `OPENAI_BASE_URL`). Select the model separately on the agent builder. Both
/// paths target OpenAI's Responses API via the recommended [`OpenAIChatDriver`].
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct OpenAI {
    api_key: String,
    base_url: Option<String>,
}

impl OpenAI {
    /// Configure OpenAI with an explicit API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from `OPENAI_API_KEY` instead.
    ///
    /// ```
    /// use everruns::providers::openai::OpenAI;
    ///
    /// let provider = OpenAI::new("sk-your-key");
    /// # let _ = provider;
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure OpenAI, reading the API key (and optional base URL) from the
    /// environment.
    ///
    /// Reads `OPENAI_API_KEY` (required) and `OPENAI_BASE_URL` (optional),
    /// matching the repo-wide credential-variable convention. An unset or empty
    /// `OPENAI_API_KEY` returns [`OpenAIError::MissingEnvVar`] rather than
    /// panicking.
    ///
    /// This is the sanctioned standalone/dev entry point for env-based
    /// credentials; explicit constructors stay environment-free.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::openai::OpenAI};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Requires `OPENAI_API_KEY` in the environment.
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(OpenAI::from_env()?)
    ///     .model("gpt-5-mini")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env() -> Result<Self, OpenAIError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(lookup: F) -> Result<Self, OpenAIError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        let api_key =
            lookup(OPENAI_API_KEY_ENV)
                .and_then(non_empty)
                .ok_or(OpenAIError::MissingEnvVar {
                    var: OPENAI_API_KEY_ENV,
                })?;
        let base_url = lookup(OPENAI_BASE_URL_ENV).and_then(non_empty);
        Ok(Self { api_key, base_url })
    }

    /// Override the API base URL (OpenAI-compatible proxy, self-hosted endpoint,
    /// or Azure gateway).
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

impl fmt::Debug for OpenAI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAI")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<OpenAI> for Provider {
    fn from(config: OpenAI) -> Self {
        let (api_key, base_url) = config.into_parts();
        let mut provider = everruns_openai::provider("openai", api_key);
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
        let config = OpenAI::new("sk-explicit");
        let (api_key, base_url) = config.into_parts();
        assert_eq!(api_key, "sk-explicit");
        assert_eq!(base_url, None);
    }

    #[test]
    fn from_lookup_reads_key_and_optional_base_url() {
        let config = OpenAI::from_lookup(|name| match name {
            "OPENAI_API_KEY" => Some("sk-env".to_string()),
            "OPENAI_BASE_URL" => Some("https://proxy.example/v1".to_string()),
            _ => None,
        })
        .expect("key present");
        let (api_key, base_url) = config.into_parts();
        assert_eq!(api_key, "sk-env");
        assert_eq!(base_url, Some("https://proxy.example/v1".to_string()));
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = OpenAI::from_lookup(|_| None).unwrap_err();
        assert_eq!(
            err,
            OpenAIError::MissingEnvVar {
                var: "OPENAI_API_KEY"
            }
        );
    }

    #[test]
    fn from_lookup_empty_key_is_missing() {
        let err =
            OpenAI::from_lookup(|name| (name == "OPENAI_API_KEY").then(String::new)).unwrap_err();
        assert_eq!(
            err,
            OpenAIError::MissingEnvVar {
                var: "OPENAI_API_KEY"
            }
        );
    }

    #[test]
    fn base_url_builder_overrides() {
        let (_, base_url) = OpenAI::new("sk-explicit")
            .base_url("https://custom.example/v1")
            .into_parts();
        assert_eq!(base_url, Some("https://custom.example/v1".to_string()));
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!(
            "{:?}",
            OpenAI::new("sk-super-secret").base_url("https://x/v1")
        );
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
        assert!(rendered.contains("[REDACTED]"), "got {rendered}");
    }

    #[test]
    fn openai_error_display_names_the_variable_only() {
        let rendered = OpenAIError::MissingEnvVar {
            var: "OPENAI_API_KEY",
        }
        .to_string();
        assert!(rendered.contains("OPENAI_API_KEY"), "got {rendered}");
    }
}
