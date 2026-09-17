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

use everruns_provider::credential_provider::EnvCredentialProvider;

use crate::Provider;

/// Re-exported `everruns-openai` drivers for direct, low-level use.
pub use everruns_openai::{OpenAIChatDriver, OpenAICompletionsChatDriver, register_driver};

/// Why an [`OpenAI`] provider configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the names of the variables the driver declares.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenAIError {
    /// None of the driver's declared variables carried a usable credential.
    MissingEnvVar {
        /// Every variable the OpenAI driver declares, most preferred first.
        vars: Vec<String>,
    },
}

impl fmt::Display for OpenAIError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenAIError::MissingEnvVar { vars } => {
                write!(
                    f,
                    "no OpenAI credentials in the environment; set {}",
                    vars.join(" or ")
                )
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
    /// The variables are the OpenAI driver's own declaration — `OPENAI_API_KEY`
    /// and the optional `OPENAI_BASE_URL`, matching the `openai` SDK — resolved
    /// through the shared [`EnvCredentialProvider`]. Nothing here restates
    /// them, so they cannot drift from what the driver reads. An unset or empty
    /// key returns [`OpenAIError::MissingEnvVar`] rather than panicking.
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
        let driver = everruns_openai::descriptor();
        let credentials = EnvCredentialProvider::resolve_with(&driver, lookup)
            .filter(|credentials| credentials.api_key().is_some())
            .ok_or_else(|| OpenAIError::MissingEnvVar {
                vars: driver.declared_env_vars(),
            })?;
        Ok(Self {
            api_key: credentials.api_key().expect("filtered above").to_string(),
            base_url: credentials.base_url().map(str::to_owned),
        })
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
        // The names come from the driver's declaration, not from this module.
        assert_eq!(
            err,
            OpenAIError::MissingEnvVar {
                vars: vec!["OPENAI_API_KEY".to_string(), "OPENAI_BASE_URL".to_string()],
            }
        );
    }

    #[test]
    fn from_lookup_empty_key_is_missing() {
        let err =
            OpenAI::from_lookup(|name| (name == "OPENAI_API_KEY").then(String::new)).unwrap_err();
        assert!(matches!(err, OpenAIError::MissingEnvVar { .. }));
    }

    #[test]
    fn a_base_url_alone_is_not_a_credential() {
        let err = OpenAI::from_lookup(|name| {
            (name == "OPENAI_BASE_URL").then(|| "https://proxy.example/v1".to_string())
        })
        .unwrap_err();
        assert!(matches!(err, OpenAIError::MissingEnvVar { .. }));
    }

    #[test]
    fn another_vendors_variable_does_not_configure_openai() {
        for other in ["ANTHROPIC_API_KEY", "GEMINI_API_KEY", "AWS_ACCESS_KEY_ID"] {
            assert!(
                OpenAI::from_lookup(|name| (name == other).then(|| "key".to_string())).is_err(),
                "{other} must not configure OpenAI"
            );
        }
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
    fn openai_error_display_names_the_variables_only() {
        let rendered = OpenAIError::MissingEnvVar {
            vars: vec!["OPENAI_API_KEY".to_string()],
        }
        .to_string();
        assert!(rendered.contains("OPENAI_API_KEY"), "got {rendered}");
    }
}
