// Pluggable provider credential source (specs/llm-drivers.md, specs/providers.md)
//
// Driver crates never read the process environment for credentials. Reading
// `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc. from a shared host environment is
// unsafe in the multitenant server: a platform-level key would silently fund
// tenant execution (the fail-closed Key Resolution Contract in
// `specs/llm-drivers.md`).
//
// Instead, credential loading is an explicit, injectable concern. A caller that
// wants env-based credentials — a CLI, a dev entrypoint, a standalone embedder —
// constructs an [`EnvCredentialProvider`] and passes it in. The server never
// constructs one; it resolves credentials from the encrypted database. This is
// the single, common seam across every driver, so adding a new driver does not
// add a new place that touches the environment.

use crate::provider::DriverId;

/// Credentials resolved for a single driver: the API key and an optional base
/// URL override. This mirrors the credential fields a driver constructor or a
/// `DriverConfig` accepts, decoupled from where they came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCredentials {
    /// API key / secret for the provider account.
    pub api_key: Option<String>,
    /// Optional endpoint override (OpenAI-compatible proxies, self-hosted, etc.).
    pub base_url: Option<String>,
}

impl ProviderCredentials {
    /// Whether any credential value is present.
    pub fn is_empty(&self) -> bool {
        self.api_key.is_none() && self.base_url.is_none()
    }
}

/// A source of provider credentials, injected by the caller.
///
/// Drivers and dev stores depend on this trait, not on the environment. The
/// multitenant server path resolves credentials from the encrypted database and
/// does not use a `CredentialProvider`; only explicit standalone/dev entrypoints
/// construct one (typically [`EnvCredentialProvider`]).
pub trait CredentialProvider: Send + Sync {
    /// Resolve credentials for the given driver, or `None` when this source has
    /// none for it.
    fn resolve(&self, driver: &DriverId) -> Option<ProviderCredentials>;
}

/// A [`CredentialProvider`] that reads credentials from the process environment.
///
/// This is the shared library implementation for env-based credentials and the
/// sanctioned pattern for any caller that wants them: driver/library code never
/// reads credential env vars itself, so new env-credential logic belongs here.
/// (Some standalone examples and `#[ignore]` live tests still read `*_API_KEY`
/// directly to gate themselves; that caller-side code should prefer this type.)
///
/// It is intended for standalone/CLI/dev use and MUST NOT be constructed on
/// org-scoped server execution paths — doing so would reopen the env fallback the
/// Key Resolution Contract forbids.
///
/// Recognized variables (per driver):
///
/// | Driver | API key | Base URL |
/// |--------|---------|----------|
/// | `openai`, `openai_completions` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` |
/// | `openrouter` | `OPENROUTER_API_KEY` | `OPENROUTER_BASE_URL` |
/// | `anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL` |
/// | `gemini` | `GEMINI_API_KEY` | `GEMINI_BASE_URL` |
///
/// Other drivers (Azure OpenAI, Bedrock, MAI, external) return `None`; their
/// dev/standalone credentials are supplied explicitly by the caller.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCredentialProvider;

impl EnvCredentialProvider {
    /// Construct the env-backed credential provider.
    pub fn new() -> Self {
        Self
    }

    /// Resolve credentials using an injectable lookup (testable without
    /// touching the real process environment).
    fn resolve_with<F>(driver: &DriverId, lookup: F) -> Option<ProviderCredentials>
    where
        F: Fn(&str) -> Option<String>,
    {
        let (key_var, url_var) = match driver {
            DriverId::OpenAI | DriverId::OpenAICompletions => ("OPENAI_API_KEY", "OPENAI_BASE_URL"),
            DriverId::OpenRouter => ("OPENROUTER_API_KEY", "OPENROUTER_BASE_URL"),
            DriverId::Anthropic => ("ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"),
            DriverId::Gemini => ("GEMINI_API_KEY", "GEMINI_BASE_URL"),
            _ => return None,
        };

        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        let api_key = lookup(key_var).and_then(non_empty);
        let base_url = lookup(url_var).and_then(non_empty);

        let creds = ProviderCredentials { api_key, base_url };
        (!creds.is_empty()).then_some(creds)
    }
}

impl CredentialProvider for EnvCredentialProvider {
    fn resolve(&self, driver: &DriverId) -> Option<ProviderCredentials> {
        Self::resolve_with(driver, |name| std::env::var(name).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(map: &HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        let owned: HashMap<String, String> = map
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name: &str| owned.get(name).cloned()
    }

    #[test]
    fn openai_reads_key_and_base_url() {
        let env = HashMap::from([
            ("OPENAI_API_KEY", "sk-test"),
            ("OPENAI_BASE_URL", "https://proxy.example/v1"),
        ]);
        let creds = EnvCredentialProvider::resolve_with(&DriverId::OpenAI, lookup_from(&env))
            .expect("credentials");
        assert_eq!(creds.api_key.as_deref(), Some("sk-test"));
        assert_eq!(creds.base_url.as_deref(), Some("https://proxy.example/v1"));
    }

    #[test]
    fn openai_completions_shares_openai_key() {
        let env = HashMap::from([("OPENAI_API_KEY", "sk-test")]);
        let creds =
            EnvCredentialProvider::resolve_with(&DriverId::OpenAICompletions, lookup_from(&env))
                .expect("credentials");
        assert_eq!(creds.api_key.as_deref(), Some("sk-test"));
        assert!(creds.base_url.is_none());
    }

    #[test]
    fn anthropic_and_gemini_keys() {
        let env = HashMap::from([("ANTHROPIC_API_KEY", "sk-ant"), ("GEMINI_API_KEY", "g-key")]);
        let lookup = lookup_from(&env);
        assert_eq!(
            EnvCredentialProvider::resolve_with(&DriverId::Anthropic, &lookup)
                .and_then(|c| c.api_key)
                .as_deref(),
            Some("sk-ant"),
        );
        assert_eq!(
            EnvCredentialProvider::resolve_with(&DriverId::Gemini, &lookup)
                .and_then(|c| c.api_key)
                .as_deref(),
            Some("g-key"),
        );
    }

    #[test]
    fn empty_or_missing_yields_none() {
        let env = HashMap::from([("OPENAI_API_KEY", "")]);
        assert!(
            EnvCredentialProvider::resolve_with(&DriverId::OpenAI, lookup_from(&env)).is_none()
        );

        let empty = HashMap::new();
        assert!(
            EnvCredentialProvider::resolve_with(&DriverId::Anthropic, lookup_from(&empty))
                .is_none()
        );
    }

    #[test]
    fn unsupported_drivers_return_none() {
        let env = HashMap::from([("AWS_ACCESS_KEY_ID", "x")]);
        let lookup = lookup_from(&env);
        assert!(EnvCredentialProvider::resolve_with(&DriverId::Bedrock, &lookup).is_none());
        assert!(EnvCredentialProvider::resolve_with(&DriverId::LlmSim, &lookup).is_none());
    }
}
