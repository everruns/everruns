//! OpenAI-backed model configuration (requires the `openai` feature).
//!
//! [`OpenAI`] is the value-first entry point for talking to OpenAI's Responses
//! API. Convert it into a [`Model`] — directly or via
//! [`AgentBuilder::model`](crate::AgentBuilder::model) — and the agent's runtime
//! registers the OpenAI driver automatically.
//!
//! The existing `everruns-openai` drivers are re-exported here
//! ([`OpenAIChatDriver`], [`OpenAICompletionsChatDriver`], [`register_driver`])
//! for embedders who need the low-level driver directly.

use std::fmt;

use crate::Model;

/// API key environment variable, matching the repo-wide convention documented on
/// `everruns_core::credential_provider::EnvCredentialProvider`.
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
/// Optional base-URL override environment variable (OpenAI-compatible proxies,
/// self-hosted endpoints, Azure gateways).
const OPENAI_BASE_URL_ENV: &str = "OPENAI_BASE_URL";

/// Re-exported `everruns-openai` drivers for direct, low-level use.
pub use everruns_openai::{OpenAIChatDriver, OpenAICompletionsChatDriver, register_driver};

/// Why an [`OpenAI`] configuration could not be produced.
///
/// Typed and cheap to match on. Credential values never appear in the error —
/// only the name of the missing environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// A required environment variable was unset or empty.
    MissingEnvVar {
        /// The variable name that was expected (e.g. `OPENAI_API_KEY`).
        var: &'static str,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::MissingEnvVar { var } => {
                write!(f, "required environment variable {var} is not set")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// Configuration for an OpenAI-backed [`Model`].
///
/// Build one with [`OpenAI::new`] (explicit, deterministic — reads no
/// environment) or [`OpenAI::from_env`] (reads `OPENAI_API_KEY` and, when set,
/// `OPENAI_BASE_URL`). Both target OpenAI's Responses API via the recommended
/// [`OpenAIChatDriver`].
///
/// The API key is redacted from [`Debug`] output.
#[derive(Clone)]
pub struct OpenAI {
    model: String,
    api_key: String,
    base_url: Option<String>,
}

impl OpenAI {
    /// Configure OpenAI with an explicit model id and API key.
    ///
    /// Deterministic: reads no environment. Use [`from_env`](Self::from_env)
    /// to pick the key up from `OPENAI_API_KEY` instead.
    ///
    /// ```
    /// use everruns::providers::openai::OpenAI;
    ///
    /// let model = OpenAI::new("gpt-5-mini", "sk-your-key");
    /// # let _ = model;
    /// ```
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            base_url: None,
        }
    }

    /// Configure OpenAI, reading the API key (and optional base URL) from the
    /// environment.
    ///
    /// Reads `OPENAI_API_KEY` (required) and `OPENAI_BASE_URL` (optional),
    /// matching the repo-wide credential-variable convention. An unset or empty
    /// `OPENAI_API_KEY` returns [`ModelError::MissingEnvVar`] rather than
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
    ///     .model(OpenAI::from_env("gpt-5-mini")?)
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_env(model: impl Into<String>) -> Result<Self, ModelError> {
        Self::from_lookup(model, |name| std::env::var(name).ok())
    }

    /// Resolve from an injectable variable lookup — the testable core of
    /// [`from_env`](Self::from_env), so tests never mutate the process
    /// environment.
    fn from_lookup<F>(model: impl Into<String>, lookup: F) -> Result<Self, ModelError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        let api_key =
            lookup(OPENAI_API_KEY_ENV)
                .and_then(non_empty)
                .ok_or(ModelError::MissingEnvVar {
                    var: OPENAI_API_KEY_ENV,
                })?;
        let base_url = lookup(OPENAI_BASE_URL_ENV).and_then(non_empty);
        Ok(Self {
            model: model.into(),
            api_key,
            base_url,
        })
    }

    /// Override the API base URL (OpenAI-compatible proxy, self-hosted endpoint,
    /// or Azure gateway).
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Consume the config into its `(model, api_key, base_url)` parts for the
    /// facade's `Model` constructor. Crate-internal so the public surface never
    /// leaks the raw key.
    pub(crate) fn into_parts(self) -> (String, String, Option<String>) {
        (self.model, self.api_key, self.base_url)
    }
}

impl fmt::Debug for OpenAI {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAI")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl From<OpenAI> for Model {
    fn from(config: OpenAI) -> Self {
        Model::openai(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_environment_free_and_sets_no_base_url() {
        let config = OpenAI::new("gpt-5-mini", "sk-explicit");
        let (model, api_key, base_url) = config.into_parts();
        assert_eq!(model, "gpt-5-mini");
        assert_eq!(api_key, "sk-explicit");
        assert_eq!(base_url, None);
    }

    #[test]
    fn from_lookup_reads_key_and_optional_base_url() {
        let config = OpenAI::from_lookup("gpt-5-mini", |name| match name {
            "OPENAI_API_KEY" => Some("sk-env".to_string()),
            "OPENAI_BASE_URL" => Some("https://proxy.example/v1".to_string()),
            _ => None,
        })
        .expect("key present");
        let (model, api_key, base_url) = config.into_parts();
        assert_eq!(model, "gpt-5-mini");
        assert_eq!(api_key, "sk-env");
        assert_eq!(base_url, Some("https://proxy.example/v1".to_string()));
    }

    #[test]
    fn from_lookup_missing_key_is_typed_error() {
        let err = OpenAI::from_lookup("gpt-5-mini", |_| None).unwrap_err();
        assert_eq!(
            err,
            ModelError::MissingEnvVar {
                var: "OPENAI_API_KEY"
            }
        );
    }

    #[test]
    fn from_lookup_empty_key_is_missing() {
        let err = OpenAI::from_lookup("gpt-5-mini", |name| {
            (name == "OPENAI_API_KEY").then(String::new)
        })
        .unwrap_err();
        assert_eq!(
            err,
            ModelError::MissingEnvVar {
                var: "OPENAI_API_KEY"
            }
        );
    }

    #[test]
    fn base_url_builder_overrides() {
        let (_, _, base_url) = OpenAI::new("gpt-5-mini", "sk-explicit")
            .base_url("https://custom.example/v1")
            .into_parts();
        assert_eq!(base_url, Some("https://custom.example/v1".to_string()));
    }

    #[test]
    fn debug_redacts_api_key() {
        let rendered = format!(
            "{:?}",
            OpenAI::new("gpt-5-mini", "sk-super-secret").base_url("https://x/v1")
        );
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
        assert!(rendered.contains("[REDACTED]"), "got {rendered}");
        assert!(rendered.contains("gpt-5-mini"), "got {rendered}");
    }

    #[test]
    fn model_error_display_names_the_variable_only() {
        let rendered = ModelError::MissingEnvVar {
            var: "OPENAI_API_KEY",
        }
        .to_string();
        assert!(rendered.contains("OPENAI_API_KEY"), "got {rendered}");
    }
}
