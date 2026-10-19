//! Host provider-request metadata (EVE-1302).
//!
//! A host that attributes its model traffic (application name and version,
//! OpenRouter `HTTP-Referer`/`X-Title`) attaches one [`RequestMetadata`] value
//! through [`HostServices::request_metadata`](super::services::HostServices::request_metadata).
//! Engine seeds it into the execution configuration of every runtime it
//! constructs for the Agent, so the same entries reach every provider request:
//! turn model calls, out-of-band completions, context queries, rebuilt and
//! resumed sessions on the legacy and the coherent hosting lanes. The effective
//! entries are visible on [`SessionContext::request_metadata`](crate::SessionContext::request_metadata).
//!
//! The value is plain, credential-free attribution: providers receive it, and
//! the Framework shows it in session inspection and request capture. Never put
//! secrets in it; credentials belong to provider resolution.

use std::collections::BTreeMap;
use std::fmt;

use everruns_contracts::openresponses_types::{
    MAX_METADATA_KEY_LENGTH, MAX_METADATA_KEYS, MAX_METADATA_VALUE_LENGTH, SYSTEM_LLM_METADATA_KEYS,
};

/// Metadata key the OpenRouter driver sends as the `HTTP-Referer` header.
pub const OPENROUTER_HTTP_REFERER_KEY: &str = "openrouter.http_referer";
/// Metadata key the OpenRouter driver sends as the `X-Title` header.
pub const OPENROUTER_X_TITLE_KEY: &str = "openrouter.x_title";

/// Most entries a host may attach: provider metadata is capped at
/// `MAX_METADATA_KEYS`, and the runtime's own correlation keys need room.
pub const MAX_REQUEST_METADATA_ENTRIES: usize = MAX_METADATA_KEYS - SYSTEM_LLM_METADATA_KEYS.len();

/// Validated-at-build, credential-free metadata a host adds to every provider
/// request of an Agent's sessions.
///
/// Entries are checked when the Agent is built: keys are 1–64 characters of
/// ASCII letters, digits, `_`, `.`, `-` or `:`; values are at most 512 bytes
/// without control characters (some drivers send entries as HTTP headers); at
/// most [`MAX_REQUEST_METADATA_ENTRIES`] entries; and no entry may claim a
/// runtime-owned correlation key such as `session_id` or `turn_id`
/// ([`RequestMetadata::RESERVED_KEYS`]), which the runtime stamps itself.
///
/// ```
/// use everruns::hosting::services::{HostServices, RequestMetadata};
///
/// let services = HostServices::new().request_metadata(
///     RequestMetadata::new()
///         .entry("app", "my-app")
///         .entry("my_app_version", "1.2.3")
///         .openrouter_attribution("https://my-app.example", "My App"),
/// );
/// # let _ = services;
/// ```
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RequestMetadata {
    entries: BTreeMap<String, String>,
}

impl RequestMetadata {
    /// Correlation keys the runtime owns on every provider request.
    pub const RESERVED_KEYS: &'static [&'static str] = SYSTEM_LLM_METADATA_KEYS;

    /// No entries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `key` to `value`, replacing an earlier value for the same key.
    pub fn entry(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.insert(key.into(), value.into());
        self
    }

    /// Set the OpenRouter attribution entries the OpenRouter driver sends as
    /// the `HTTP-Referer` and `X-Title` headers. Other providers receive them
    /// as ordinary request metadata.
    pub fn openrouter_attribution(
        self,
        http_referer: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        self.entry(OPENROUTER_HTTP_REFERER_KEY, http_referer)
            .entry(OPENROUTER_X_TITLE_KEY, title)
    }

    /// The value set for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn validate(&self) -> Result<(), crate::BuildError> {
        let invalid = |key: &str, reason: String| crate::BuildError::InvalidRequestMetadata {
            key: key.to_string(),
            reason,
        };
        if self.entries.len() > MAX_REQUEST_METADATA_ENTRIES {
            return Err(invalid(
                "",
                format!(
                    "{} entries exceed the limit of {MAX_REQUEST_METADATA_ENTRIES}",
                    self.entries.len()
                ),
            ));
        }
        for (key, value) in &self.entries {
            if key.is_empty() || key.len() > MAX_METADATA_KEY_LENGTH {
                return Err(invalid(
                    key,
                    format!("key must be 1 to {MAX_METADATA_KEY_LENGTH} characters"),
                ));
            }
            if !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-:".contains(&byte))
            {
                return Err(invalid(
                    key,
                    "key may contain only ASCII letters, digits, '_', '.', '-' and ':'".into(),
                ));
            }
            // THREAT[TM-LLM-053]: the runtime stamps these correlation ids on
            // every request; a host entry must not claim execution identity.
            if Self::RESERVED_KEYS
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(key))
            {
                return Err(invalid(
                    key,
                    "key is reserved for runtime-owned correlation".into(),
                ));
            }
            if value.len() > MAX_METADATA_VALUE_LENGTH {
                return Err(invalid(
                    key,
                    format!("value exceeds {MAX_METADATA_VALUE_LENGTH} bytes"),
                ));
            }
            if value.chars().any(char::is_control) {
                return Err(invalid(
                    key,
                    "value must not contain control characters".into(),
                ));
            }
        }
        Ok(())
    }
}

impl fmt::Debug for RequestMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for RequestMetadata {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        entries
            .into_iter()
            .fold(Self::new(), |metadata, (key, value)| {
                metadata.entry(key, value)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildError;

    fn reason(metadata: RequestMetadata) -> (String, String) {
        match metadata.validate() {
            Err(BuildError::InvalidRequestMetadata { key, reason }) => (key, reason),
            other => panic!("expected InvalidRequestMetadata, got {other:?}"),
        }
    }

    #[test]
    fn accepts_attribution_and_app_entries() {
        let metadata = RequestMetadata::new()
            .entry("app", "yolop")
            .entry("yolop_version", "0.9.0")
            .entry("everruns_core_version", "0.1.0")
            .openrouter_attribution("https://yolop.example", "Yolop");
        metadata.validate().unwrap();
        assert_eq!(metadata.len(), 5);
        assert_eq!(metadata.get(OPENROUTER_X_TITLE_KEY), Some("Yolop"));
    }

    #[test]
    fn rejects_reserved_correlation_keys_in_any_case() {
        for key in RequestMetadata::RESERVED_KEYS
            .iter()
            .map(|key| key.to_string())
            .chain(["Session_ID".to_string()])
        {
            let (rejected, reason) = reason(RequestMetadata::new().entry(key.clone(), "spoof"));
            assert_eq!(rejected, key);
            assert!(reason.contains("reserved"), "{reason}");
        }
    }

    #[test]
    fn rejects_malformed_keys_values_and_overflow() {
        assert!(
            reason(RequestMetadata::new().entry("", "x"))
                .1
                .contains("1 to")
        );
        assert!(
            reason(RequestMetadata::new().entry("k".repeat(65), "x"))
                .1
                .contains("1 to")
        );
        assert!(
            reason(RequestMetadata::new().entry("bad key", "x"))
                .1
                .contains("ASCII")
        );
        assert!(
            reason(RequestMetadata::new().entry("app", "v".repeat(513)))
                .1
                .contains("exceeds")
        );
        assert!(
            reason(RequestMetadata::new().openrouter_attribution("https://x\r\nX-Evil: 1", "t"))
                .1
                .contains("control")
        );
        let too_many: RequestMetadata = (0..=MAX_REQUEST_METADATA_ENTRIES)
            .map(|index| (format!("k{index}"), "v"))
            .collect();
        assert!(reason(too_many).1.contains("limit"));
    }
}
