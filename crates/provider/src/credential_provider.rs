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

    fn resolve(driver: &str, entries: &[(&str, &str)]) -> Option<ProviderCredentials> {
        EnvCredentialProvider::resolve_with(&DriverId::external(driver), |key| {
            entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        })
    }

    fn credentials(key: Option<&str>, url: Option<&str>) -> Option<ProviderCredentials> {
        Some(ProviderCredentials {
            api_key: key.map(str::to_owned),
            base_url: url.map(str::to_owned),
        })
    }

    #[test]
    fn explicit_driver_environment_names_preserve_both_fields_and_isolate_other_keys() {
        for (driver, key_name, url_name) in [
            ("openai", "OPENAI_API_KEY", "OPENAI_BASE_URL"),
            ("anthropic", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"),
            ("gemini", "GEMINI_API_KEY", "GEMINI_BASE_URL"),
            (
                "custom-driver.v2",
                "CUSTOM_DRIVER_V2_API_KEY",
                "CUSTOM_DRIVER_V2_BASE_URL",
            ),
        ] {
            let entries = [
                (key_name, "key"),
                (url_name, "https://proxy.example/v1"),
                ("OTHER_API_KEY", "unrelated"),
            ];
            assert_eq!(
                resolve(driver, &entries),
                credentials(Some("key"), Some("https://proxy.example/v1")),
                "{driver}"
            );
            assert_eq!(
                resolve(driver, &[(url_name, "https://proxy.example/v1")]),
                credentials(None, Some("https://proxy.example/v1"))
            );
            assert_eq!(resolve(driver, &[(key_name, ""), (url_name, "")]), None);
            assert_eq!(resolve(driver, &[]), None);
        }
        for driver in ["bedrock", "llmsim"] {
            assert_eq!(
                resolve(
                    driver,
                    &[
                        ("AWS_ACCESS_KEY_ID", "unrelated"),
                        ("OPENAI_API_KEY", "other provider")
                    ]
                ),
                None
            );
        }
    }

    #[test]
    fn completions_fallback_is_independent_per_field_and_never_overrides_specific_values() {
        for (specific_key, specific_url, expected_key, expected_url) in [
            (None, None, "legacy-key", "https://legacy.example/v1"),
            (
                Some(""),
                Some(""),
                "legacy-key",
                "https://legacy.example/v1",
            ),
            (
                Some("specific-key"),
                None,
                "specific-key",
                "https://legacy.example/v1",
            ),
            (
                None,
                Some("https://specific.example/v1"),
                "legacy-key",
                "https://specific.example/v1",
            ),
            (
                Some("specific-key"),
                Some("https://specific.example/v1"),
                "specific-key",
                "https://specific.example/v1",
            ),
        ] {
            let mut entries = vec![
                ("OPENAI_API_KEY", "legacy-key"),
                ("OPENAI_BASE_URL", "https://legacy.example/v1"),
            ];
            if let Some(key) = specific_key {
                entries.push(("OPENAI_COMPLETIONS_API_KEY", key));
            }
            if let Some(url) = specific_url {
                entries.push(("OPENAI_COMPLETIONS_BASE_URL", url));
            }
            assert_eq!(
                resolve("openai_completions", &entries),
                credentials(Some(expected_key), Some(expected_url))
            );
        }
        assert_eq!(
            resolve("openai_completions", &[("OPENAI_API_KEY", "legacy-key")]),
            credentials(Some("legacy-key"), None)
        );
        assert_eq!(
            resolve(
                "openai_completions",
                &[("OPENAI_API_KEY", ""), ("OPENAI_BASE_URL", "")]
            ),
            None
        );
    }

    #[test]
    fn debug_output_redacts_the_api_key() {
        for api_key in [None, Some(""), Some("sk-super-secret")] {
            let creds = ProviderCredentials {
                api_key: api_key.map(str::to_owned),
                base_url: Some("https://proxy.example/v1".into()),
            };
            let expected = if api_key.is_some() {
                "ProviderCredentials { api_key: Some(\"[REDACTED]\"), base_url: Some(\"https://proxy.example/v1\") }"
            } else {
                "ProviderCredentials { api_key: None, base_url: Some(\"https://proxy.example/v1\") }"
            };
            assert_eq!(format!("{creds:?}"), expected);
        }
    }
}
