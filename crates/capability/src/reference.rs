//! Capability references: identity plus per-agent JSON object configuration.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

use crate::error::CapabilityError;
use crate::id::CapabilityId;

/// A reference to a capability implementation plus per-agent JSON.
///
/// This is the one semantic model for "a capability attached to an agent":
/// the Framework activates it, the product persists it (the historical
/// `AgentCapabilityConfig` attachment row and `BuiltInCapabilityDefinition`
/// provisioning entry are this type), and worker resolution consumes it. It
/// serializes as `{"ref": "<id>", "config": {…}}` everywhere.
///
/// IDs are open strings rather than variants in a central enum. They must use
/// a stable identifier made from ASCII letters, digits, `_`, `-`, `.`, or
/// `:`, start with a letter or `_`, and fit within 128 bytes. The
/// `__everruns_` namespace is reserved.
///
/// Configuration defaults to `{}` and must be a JSON object. Boundaries that
/// accept new values (Framework agent build, product write paths) enforce
/// both rules via [`CapabilityRef::validate`]; the referenced implementation
/// owns the inner schema.
///
/// Configuration is redacted from `Debug`, but it is not a secret store:
/// hosts may persist or otherwise inspect it. Put credentials in a
/// provider-owned secret mechanism and pass only a non-secret handle here.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRef {
    #[serde(rename = "ref")]
    id: CapabilityId,
    #[serde(default = "empty_object", deserialize_with = "object_or_default")]
    config: Value,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// Normalize absent/`null` configuration to `{}` on deserialize so persisted
/// attachments written before the object boundary was enforced keep loading.
fn object_or_default<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => empty_object(),
        other => other,
    })
}

impl fmt::Debug for CapabilityRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilityRef")
            .field("id", &self.id)
            .field("config", &"<redacted>")
            .finish()
    }
}

impl CapabilityRef {
    /// Reference a stable capability ID with default (`{}`) configuration.
    pub fn new(id: impl Into<CapabilityId>) -> Self {
        Self {
            id: id.into(),
            config: empty_object(),
        }
    }

    /// Reference a capability ID with explicit configuration.
    pub fn with_config(id: impl Into<CapabilityId>, config: Value) -> Self {
        Self {
            id: id.into(),
            config,
        }
    }

    /// Attach implementation-defined per-agent JSON configuration.
    ///
    /// Validation is deferred to the consuming boundary (e.g. the Framework's
    /// `AgentBuilder::build`) so capability values remain easy to compose and
    /// pass through application configuration layers.
    pub fn config(mut self, config: impl Into<Value>) -> Self {
        self.config = config.into();
        self
    }

    /// The stable capability identifier.
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// The stable capability identifier (alias of [`CapabilityRef::id`]).
    pub fn capability_id(&self) -> &str {
        self.id.as_str()
    }

    /// The typed capability identifier.
    pub fn typed_id(&self) -> &CapabilityId {
        &self.id
    }

    /// The implementation-defined JSON configuration.
    pub fn config_value(&self) -> &Value {
        &self.config
    }

    /// Mutable access to the configuration (host hydration paths).
    pub fn config_mut(&mut self) -> &mut Value {
        &mut self.config
    }

    /// Replace the configuration in place.
    pub fn set_config(&mut self, config: impl Into<Value>) {
        self.config = config.into();
    }

    /// Replace the identifier in place (host alias-canonicalization paths).
    pub fn set_id(&mut self, id: impl Into<CapabilityId>) {
        self.id = id.into();
    }

    /// Split the reference into its identity and configuration.
    pub fn into_parts(self) -> (CapabilityId, Value) {
        (self.id, self.config)
    }

    /// Validate the identifier grammar and the JSON object config boundary.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        self.id.validate()?;
        validate_capability_config(self.id.as_str(), &self.config)
    }
}

impl From<CapabilityId> for CapabilityRef {
    fn from(id: CapabilityId) -> Self {
        Self::new(id)
    }
}

impl From<&str> for CapabilityRef {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for CapabilityRef {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

/// Validate the shared JSON object configuration boundary.
///
/// Capability configuration must be a JSON object; the referenced
/// implementation owns the inner schema.
pub fn validate_capability_config(id: &str, config: &Value) -> Result<(), CapabilityError> {
    if config.is_object() {
        Ok(())
    } else {
        Err(CapabilityError::InvalidConfig {
            id: id.to_string(),
            reason: "capability config must be a JSON object".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_defaults_to_empty_object() {
        for cap in [
            CapabilityRef::new("current_time"),
            CapabilityId::new("current_time").into(),
            "current_time".into(),
            String::from("current_time").into(),
        ] {
            assert_eq!(cap.capability_id(), "current_time");
            assert_eq!(cap.config_value(), &json!({}));
            assert_eq!(
                serde_json::to_value(&cap).unwrap(),
                json!({"ref": "current_time", "config": {}})
            );
        }
    }

    #[test]
    fn serializes_as_persisted_attachment_shape() {
        let cap = CapabilityRef::with_config("current_time", json!({"timezone": "UTC"}));
        let value = serde_json::to_value(&cap).unwrap();
        assert_eq!(
            value,
            json!({"ref": "current_time", "config": {"timezone": "UTC"}})
        );

        let parsed: CapabilityRef = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn missing_and_null_config_deserialize_to_empty_object() {
        let missing: CapabilityRef = serde_json::from_str(r#"{"ref":"noop"}"#).unwrap();
        assert_eq!(missing.config_value(), &json!({}));

        let null: CapabilityRef = serde_json::from_str(r#"{"ref":"noop","config":null}"#).unwrap();
        assert_eq!(null.config_value(), &json!({}));
    }

    #[test]
    fn debug_redacts_config_values() {
        let cap = CapabilityRef::with_config(
            "vendor.search",
            json!({"api_key": "sk-super-secret", "index": "prod"}),
        );
        let debug = format!("{cap:?}");
        assert!(debug.contains("vendor.search"));
        assert!(!debug.contains("sk-super-secret"));
        assert!(!debug.contains("api_key"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn validate_enforces_id_and_object_boundary() {
        CapabilityRef::new("current_time").validate().unwrap();
        assert!(CapabilityRef::new("2fast").validate().is_err());
        CapabilityRef::with_config("noop", json!({"enabled": true}))
            .validate()
            .unwrap();
        for config in [
            json!("string"),
            json!(null),
            json!([]),
            json!(42),
            json!(true),
        ] {
            let err = CapabilityRef::with_config("noop", config)
                .validate()
                .unwrap_err();
            assert_eq!(err.id(), "noop");
            assert!(err.reason().contains("JSON object"));
        }
    }

    #[test]
    fn builder_and_mutation() {
        let mut cap = CapabilityRef::new("noop").config(json!({"a": 1}));
        assert_eq!(cap.config_value(), &json!({"a": 1}));
        cap.set_config(json!({"b": 2}));
        assert_eq!(cap.config_value(), &json!({"b": 2}));
        let (id, config) = cap.into_parts();
        assert_eq!(id.as_str(), "noop");
        assert_eq!(config, json!({"b": 2}));
    }
}
