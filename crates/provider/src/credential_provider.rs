// Pluggable provider credential source (knowledge/foundations/llm-drivers.md, knowledge/foundations/providers.md)
//
// Driver crates never read the process environment for credentials. Reading
// `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc. from a shared host environment is
// unsafe in the multitenant server: a platform-level key would silently fund
// tenant execution (the fail-closed Key Resolution Contract in
// `knowledge/foundations/llm-drivers.md`).
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
///
/// `Debug` redacts the API key (EVE-879): resolved credentials flow through
/// host wiring that logs liberally, so the secret must never reach output.
#[derive(Clone, Default, PartialEq, Eq)]
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

impl std::fmt::Debug for ProviderCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCredentials")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .finish()
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
/// Variables follow the open driver id: `<UPPERCASE_ID>_API_KEY` and
/// `<UPPERCASE_ID>_BASE_URL`, with punctuation normalized to `_`.
/// `openai_completions` also falls back to the historical `OPENAI_*` names.
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
        let stem = driver
            .as_str()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let key_var = format!("{stem}_API_KEY");
        let url_var = format!("{stem}_BASE_URL");

        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        let mut api_key = lookup(&key_var).and_then(non_empty);
        let mut base_url = lookup(&url_var).and_then(non_empty);
        if driver == &DriverId::OpenAICompletions {
            api_key = api_key.or_else(|| lookup("OPENAI_API_KEY").and_then(non_empty));
            base_url = base_url.or_else(|| lookup("OPENAI_BASE_URL").and_then(non_empty));
        }

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
    fn debug_output_redacts_the_api_key() {
        let creds = ProviderCredentials {
            api_key: Some("sk-super-secret".to_string()),
            base_url: Some("https://proxy.example/v1".to_string()),
        };
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("sk-super-secret"), "{rendered}");
        assert!(rendered.contains("[REDACTED]"));
        assert!(
            rendered.contains("https://proxy.example/v1"),
            "base URL is not a secret: {rendered}"
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
