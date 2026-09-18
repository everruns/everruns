//! Real LLM provider configuration for the facade.
//!
//! Each provider lives behind its own cargo feature so the default facade build
//! stays fully offline — no provider crate, no Reqwest edge. Enable the `openai`
//! feature to configure OpenAI-backed models through [`openai::OpenAI`], or
//! `openrouter` for [`openrouter::OpenRouter`].
//!
//! When an application does not care *which* vendor it reaches — a script, a
//! test harness, a tool that runs on whoever's machine — [`from_env`] picks the
//! one the environment is configured for.

use std::fmt;

#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "openrouter")]
pub mod openrouter;

/// Why no provider could be configured from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FromEnvError {
    /// No enabled provider found its credentials.
    NoCredentials {
        /// Every variable that would have configured a provider, in the order
        /// [`from_env`] looks for them.
        vars: Vec<String>,
    },
    /// The facade was built with no provider feature enabled, so there is
    /// nothing to look for.
    NoProvidersCompiled,
}

impl fmt::Display for FromEnvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FromEnvError::NoCredentials { vars } => write!(
                f,
                "no provider credentials in the environment; set one of {}",
                vars.join(", ")
            ),
            FromEnvError::NoProvidersCompiled => f.write_str(
                "no provider feature is enabled; build `everruns` with `openai` or `openrouter`",
            ),
        }
    }
}

impl std::error::Error for FromEnvError {}

/// Configure whichever provider the environment carries credentials for.
///
/// Enabled providers are tried in a fixed, documented order — OpenAI, then
/// OpenRouter — so the choice is reproducible rather than dependent on feature
/// resolution or iteration order. Each is resolved through its own
/// `from_env`, which reads the variables that provider's driver declares
/// (`OPENAI_API_KEY`/`OPENAI_BASE_URL`, `OPENROUTER_API_KEY`/
/// `OPENROUTER_BASE_URL`).
///
/// For a specific vendor, or to be explicit about which one an application
/// uses, construct it directly instead — this is the convenience for callers
/// that genuinely do not mind.
///
/// # Errors
///
/// [`FromEnvError::NoCredentials`] when no enabled provider found a key, naming
/// every variable that would have worked, and
/// [`FromEnvError::NoProvidersCompiled`] when the build has no provider feature
/// at all.
///
/// ```no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::Model;
///
/// let provider = everruns::providers::from_env()?;
/// let answer = Model::new("gpt-5-mini", provider);
/// # let _ = answer;
/// # Ok(())
/// # }
/// ```
pub fn from_env() -> Result<crate::Provider, FromEnvError> {
    #[allow(unused_mut)]
    let mut vars: Vec<String> = Vec::new();

    #[cfg(feature = "openai")]
    {
        match openai::OpenAI::from_env() {
            Ok(config) => return Ok(config.into()),
            Err(openai::OpenAIError::MissingEnvVar { vars: declared }) => vars.extend(declared),
        }
    }

    #[cfg(feature = "openrouter")]
    {
        match openrouter::OpenRouter::from_env() {
            Ok(config) => return Ok(config.into()),
            Err(openrouter::OpenRouterError::MissingEnvVar { vars: declared }) => {
                vars.extend(declared)
            }
        }
    }

    if vars.is_empty() {
        return Err(FromEnvError::NoProvidersCompiled);
    }
    Err(FromEnvError::NoCredentials { vars })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_names_what_to_set() {
        let error = FromEnvError::NoCredentials {
            vars: vec!["OPENAI_API_KEY".into(), "OPENROUTER_API_KEY".into()],
        };
        let rendered = error.to_string();
        assert!(rendered.contains("OPENAI_API_KEY"), "{rendered}");
        assert!(rendered.contains("OPENROUTER_API_KEY"), "{rendered}");
    }

    #[test]
    fn a_build_without_providers_says_so_rather_than_blaming_the_environment() {
        let rendered = FromEnvError::NoProvidersCompiled.to_string();
        assert!(rendered.contains("no provider feature"), "{rendered}");
    }
}
