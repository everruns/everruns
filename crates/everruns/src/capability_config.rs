//! Open, value-first capability configuration for Framework agents.

use std::fmt;

use serde_json::{Map, Value};

/// Convert an application value into one Framework capability registration.
///
/// This trait is intentionally public and non-sealed. Third-party crates can
/// implement it without depending on `everruns-core` or host internals: return
/// a [`CapabilitySpec`] built from a stable [`CapabilityRef`]. The Framework
/// validates identifiers, JSON configuration, duplicates, and implementation
/// collisions when [`AgentBuilder::build`](crate::AgentBuilder::build) runs.
/// Conversion itself is infallible and performs no registration.
///
/// # Example
///
/// ```
/// use everruns::{CapabilityRef, CapabilitySpec, IntoCapability};
/// use serde_json::json;
///
/// struct VendorSearch {
///     index: String,
/// }
///
/// impl IntoCapability for VendorSearch {
///     fn into_capability(self) -> CapabilitySpec {
///         CapabilityRef::new("vendor.search")
///             .config(json!({ "index": self.index }))
///             .into()
///     }
/// }
/// ```
pub trait IntoCapability {
    /// Consume the value and return its normalized Framework specification.
    fn into_capability(self) -> CapabilitySpec;
}

/// A dynamic reference to a capability implementation plus per-agent JSON.
///
/// Use this escape hatch when capability identity or configuration comes from
/// a database, plugin, or another runtime catalog. IDs are open strings rather
/// than variants in a Framework-owned enum. They must use a stable identifier
/// made from ASCII letters, digits, `_`, `-`, `.`, or `:`, start with a letter
/// or `_`, and fit within 128 bytes. The `__everruns_` namespace is reserved.
///
/// Configuration defaults to `{}` and must be a JSON object. At agent build
/// time, known built-ins also run their implementation-owned validator;
/// declarative/plugin payloads run the shared declarative validator. For an
/// otherwise unknown third-party ID, the Framework can validate only the ID
/// and object boundary—the referenced implementation owns the inner schema.
///
/// Configuration is redacted from `Debug`, but it is not a secret store: hosts
/// may persist or otherwise inspect it. Put credentials in a provider-owned
/// secret mechanism and pass only a non-secret handle here.
#[derive(Clone, PartialEq)]
pub struct CapabilityRef {
    id: String,
    config: Value,
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
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            config: Value::Object(Map::new()),
        }
    }

    /// Attach implementation-defined per-agent JSON configuration.
    ///
    /// Validation is deferred to [`AgentBuilder::build`](crate::AgentBuilder::build)
    /// so capability values remain easy to compose and pass through application
    /// configuration layers.
    pub fn config(mut self, config: impl Into<Value>) -> Self {
        self.config = config.into();
        self
    }

    /// The stable capability identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The implementation-defined JSON configuration.
    pub fn config_value(&self) -> &Value {
        &self.config
    }
}

/// The normalized Framework value produced by [`IntoCapability`].
///
/// A spec always activates exactly one [`CapabilityRef`]. With the default
/// `capabilities` feature it may also carry the matching code-defined
/// `capability::Definition` that the private in-process host must register when
/// the `capabilities` feature is enabled. Applications normally construct specs
/// by converting a [`CapabilityRef`], a typed built-in value, or a `Definition`.
///
/// Duplicate IDs are never merged and later registrations never overwrite
/// earlier ones. [`AgentBuilder::build`](crate::AgentBuilder::build) rejects
/// duplicates after resolving built-in aliases, including a reference paired
/// with a code-defined implementation and an implementation that would shadow
/// a built-in.
#[derive(Clone, Debug)]
pub struct CapabilitySpec {
    reference: CapabilityRef,
    #[cfg(feature = "capabilities")]
    definition: Option<crate::capability::Definition>,
}

impl CapabilitySpec {
    /// Normalize a dynamic capability reference.
    pub fn reference(reference: CapabilityRef) -> Self {
        Self {
            reference,
            #[cfg(feature = "capabilities")]
            definition: None,
        }
    }

    /// Normalize a code-defined capability and activate its stable ID.
    #[cfg(feature = "capabilities")]
    pub fn definition(definition: crate::capability::Definition) -> Self {
        Self {
            reference: CapabilityRef::new(definition.id()),
            definition: Some(definition),
        }
    }

    /// The reference that will be activated for the agent.
    pub fn capability_ref(&self) -> &CapabilityRef {
        &self.reference
    }

    pub(crate) fn into_parts(self) -> CapabilitySpecParts {
        CapabilitySpecParts {
            reference: self.reference,
            #[cfg(feature = "capabilities")]
            definition: self.definition,
        }
    }
}

pub(crate) struct CapabilitySpecParts {
    pub reference: CapabilityRef,
    #[cfg(feature = "capabilities")]
    pub definition: Option<crate::capability::Definition>,
}

impl From<CapabilityRef> for CapabilitySpec {
    fn from(reference: CapabilityRef) -> Self {
        Self::reference(reference)
    }
}

impl IntoCapability for CapabilitySpec {
    fn into_capability(self) -> CapabilitySpec {
        self
    }
}

impl IntoCapability for CapabilityRef {
    fn into_capability(self) -> CapabilitySpec {
        self.into()
    }
}

impl IntoCapability for &str {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new(self).into()
    }
}

impl IntoCapability for String {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new(self).into()
    }
}

impl IntoCapability for &String {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new(self.as_str()).into()
    }
}

// Retain the 0.17 raw-config input without making it part of the documented
// Framework surface. CapabilityRef is the stable application API.
impl IntoCapability for everruns_core::AgentCapabilityConfig {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new(self.capability_id())
            .config(self.config)
            .into()
    }
}

#[cfg(feature = "capabilities")]
impl From<crate::capability::Definition> for CapabilitySpec {
    fn from(definition: crate::capability::Definition) -> Self {
        Self::definition(definition)
    }
}

#[cfg(feature = "capabilities")]
impl IntoCapability for crate::capability::Definition {
    fn into_capability(self) -> CapabilitySpec {
        self.into()
    }
}

pub(crate) fn validate_capability_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("capability id must not be empty".to_string());
    }
    if id.len() > 128 {
        return Err(format!(
            "capability id must be at most 128 bytes (got {})",
            id.len()
        ));
    }
    if id.starts_with("__everruns_") {
        return Err("capability id uses the reserved '__everruns_' namespace".to_string());
    }
    let mut chars = id.chars();
    let first = chars.next().expect("non-empty checked above");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err("capability id must start with a letter or underscore".to_string());
    }
    if id
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':')))
    {
        return Err(
            "capability id may only contain ASCII letters, digits, '_', '-', '.' or ':'"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn validate_capability_config(config: &Value) -> Result<(), String> {
    if config.is_object() {
        Ok(())
    } else {
        Err("capability config must be a JSON object".to_string())
    }
}
