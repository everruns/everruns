//! Open, validated capability identifiers.

use serde::{Deserialize, Serialize};

use crate::error::CapabilityError;

/// The namespace reserved for Everruns-internal capability machinery.
///
/// Application- and third-party-authored capability ids must not start with
/// this prefix; [`validate_capability_id`] rejects it.
pub const RESERVED_CAPABILITY_ID_NAMESPACE: &str = "__everruns_";

/// Capability identifier — an open, string-based ID.
///
/// IDs are open strings rather than variants in a central enum, so new
/// capabilities can be added without database migrations or Everruns source
/// edits. A stable identifier is made from ASCII letters, digits, `_`, `-`,
/// `.`, or `:`, starts with a letter or `_`, and fits within 128 bytes; the
/// `__everruns_` namespace is reserved.
///
/// [`CapabilityId::new`] is infallible so persisted and in-flight values can
/// be carried without re-validation; boundaries that accept new identifiers
/// (Framework agent build, product write paths) enforce the grammar with
/// [`validate_capability_id`] / [`CapabilityId::validate`] or the validating
/// [`FromStr`](std::str::FromStr) implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityId(String);

impl CapabilityId {
    /// Create a capability ID from a string without validating it.
    ///
    /// Use this for values that were already validated at a boundary (or that
    /// predate validation). New identifiers should go through
    /// [`CapabilityId::parse`] or a boundary that calls
    /// [`validate_capability_id`].
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Create a validated capability ID.
    pub fn parse(id: impl Into<String>) -> Result<Self, CapabilityError> {
        let id = id.into();
        validate_capability_id(&id)?;
        Ok(Self(id))
    }

    /// Validate this ID against the open-ID grammar and reserved namespaces.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        validate_capability_id(&self.0)
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the ID and return the underlying string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CapabilityId {
    type Err = CapabilityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<&str> for CapabilityId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for CapabilityId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for CapabilityId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for CapabilityId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for CapabilityId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for CapabilityId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Validate a capability identifier against the shared open-ID grammar.
///
/// The rules are the single source of truth for Framework build validation
/// and product write paths: non-empty, at most 128 bytes, ASCII letters,
/// digits, `_`, `-`, `.` or `:` only, starting with a letter or `_`, and not
/// in the reserved `__everruns_` namespace.
pub fn validate_capability_id(id: &str) -> Result<(), CapabilityError> {
    let invalid = |reason: String| CapabilityError::InvalidId {
        id: id.to_string(),
        reason,
    };
    if id.is_empty() {
        return Err(invalid("capability id must not be empty".to_string()));
    }
    if id.len() > 128 {
        return Err(invalid(format!(
            "capability id must be at most 128 bytes (got {})",
            id.len()
        )));
    }
    if id.starts_with(RESERVED_CAPABILITY_ID_NAMESPACE) {
        return Err(invalid(
            "capability id uses the reserved '__everruns_' namespace".to_string(),
        ));
    }
    let mut chars = id.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(invalid(
            "capability id must start with a letter or underscore".to_string(),
        ));
    }
    if id
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')))
    {
        return Err(invalid(
            "capability id may only contain ASCII letters, digits, '_', '-', '.' or ':'"
                .to_string(),
        ));
    }
    Ok(())
}

// ============================================================================
// Plugin capability ID helpers
// ============================================================================
//
// `plugin:` is one of the open reference namespaces (`mcp:`, `skill:`,
// `declarative:`, `plugin:`). The other namespace helpers live with their
// capability implementations; the plugin helpers live here because plugin
// references are pure identity (no implementation crate owns them).
//
// Server-managed plugin refs use stable installation public IDs. Standalone
// Agent Plugins refs use manifest names of at most 64 ASCII bytes, so persisted
// capability reference columns reserve 71 bytes including `plugin:`.

/// The `plugin:` prefix used to identify installed plugin capabilities.
pub const PLUGIN_CAPABILITY_PREFIX: &str = "plugin:";

/// Construct a plugin capability reference from its stable identity suffix.
///
/// Server-managed plugins use the installation public ID. Standalone runtime
/// plugins use the manifest name because they do not have an installation row.
pub fn plugin_capability_id(identity: &str) -> String {
    format!("{PLUGIN_CAPABILITY_PREFIX}{identity}")
}

/// Return `true` if `capability_id` is a `plugin:…` reference.
pub fn is_plugin_capability(capability_id: &str) -> bool {
    capability_id.starts_with(PLUGIN_CAPABILITY_PREFIX)
}

/// Strip the `plugin:` prefix and return the identity suffix.
pub fn parse_plugin_capability_id(capability_id: &str) -> Option<&str> {
    capability_id.strip_prefix(PLUGIN_CAPABILITY_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_validates() {
        assert_eq!(
            "noop".parse::<CapabilityId>().unwrap(),
            CapabilityId::new("noop")
        );
        assert!("".parse::<CapabilityId>().is_err());
        assert!("2fast".parse::<CapabilityId>().is_err());
    }

    #[test]
    fn valid_ids_pass() {
        for id in [
            "noop",
            "current_time",
            "vendor.search",
            "mcp:550e8400-e29b-41d4-a716-446655440000",
            "plugin:plg_0193",
            "declarative:research_pack",
            "_private",
            "a",
            "__everruns", // Similar prefix is not the reserved namespace.
            &"a".repeat(128),
        ] {
            validate_capability_id(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }

    #[test]
    fn invalid_ids_fail_with_reasons() {
        for (id, fragment) in [
            ("", "must not be empty"),
            ("2fast", "start with a letter"),
            ("has space", "may only contain"),
            ("vendor/custom", "may only contain"),
            ("éclair", "start with a letter"),
            ("vendor.éclair", "may only contain"),
            ("__everruns_private", "reserved"),
            (&"x".repeat(129), "at most 128 bytes"),
        ] {
            let err = validate_capability_id(id).unwrap_err();
            assert!(
                err.reason().contains(fragment),
                "{id:?}: {} should contain {fragment:?}",
                err.reason()
            );
            assert_eq!(err.id(), id);
        }
    }

    #[test]
    fn serde_is_transparent() {
        for text in ["current_time", "my_custom_capability"] {
            let id = CapabilityId::new(text);
            assert_eq!(id.to_string(), text);
            assert_eq!(id.as_str(), text);
            assert_eq!(serde_json::to_value(&id).unwrap(), serde_json::json!(text));
            let parsed: CapabilityId = serde_json::from_value(serde_json::json!(text)).unwrap();
            assert_eq!(parsed, id);
        }
    }

    #[test]
    fn plugin_helpers_round_trip() {
        let id = plugin_capability_id("plg_01");
        assert_eq!(id, "plugin:plg_01");
        assert!(is_plugin_capability(&id));
        assert_eq!(parse_plugin_capability_id(&id), Some("plg_01"));
        assert!(!is_plugin_capability("noop"));
        assert_eq!(parse_plugin_capability_id("noop"), None);
        assert!(!is_plugin_capability("pluginish:plg_01"));
        assert_eq!(parse_plugin_capability_id("pluginish:plg_01"), None);
    }
}
