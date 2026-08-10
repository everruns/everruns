//! The open conversion contract from application values to capability
//! registrations.

use crate::reference::CapabilityRef;

/// Convert an application value into one capability registration.
///
/// This trait is intentionally public and non-sealed. Third-party crates can
/// implement it without depending on `everruns-core` or host internals:
/// return a [`CapabilitySpec`] built from a stable [`CapabilityRef`]. The
/// consuming boundary (e.g. the Framework's `AgentBuilder::build`) validates
/// identifiers, JSON configuration, duplicates, and implementation
/// collisions. Conversion itself is infallible and performs no registration.
///
/// # Example
///
/// ```
/// use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};
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
    /// Consume the value and return its normalized specification.
    fn into_capability(self) -> CapabilitySpec;
}

/// The normalized value produced by [`IntoCapability`].
///
/// A spec always activates exactly one [`CapabilityRef`]. With the
/// `definition` feature it may also carry the matching code-defined
/// [`Definition`](crate::definition::Definition) that the host must register.
/// Applications normally construct specs by converting a [`CapabilityRef`], a
/// typed built-in value, or a `Definition`.
///
/// Duplicate IDs are never merged and later registrations never overwrite
/// earlier ones. The consuming boundary rejects duplicates after resolving
/// built-in aliases, including a reference paired with a code-defined
/// implementation and an implementation that would shadow a built-in.
#[derive(Clone, Debug)]
pub struct CapabilitySpec {
    reference: CapabilityRef,
    #[cfg(feature = "definition")]
    definition: Option<crate::definition::Definition>,
}

impl CapabilitySpec {
    /// Normalize a dynamic capability reference.
    pub fn reference(reference: CapabilityRef) -> Self {
        Self {
            reference,
            #[cfg(feature = "definition")]
            definition: None,
        }
    }

    /// Normalize a code-defined capability and activate its stable ID.
    #[cfg(feature = "definition")]
    pub fn definition(definition: crate::definition::Definition) -> Self {
        Self {
            reference: CapabilityRef::new(definition.id()),
            definition: Some(definition),
        }
    }

    /// The reference that will be activated for the agent.
    pub fn capability_ref(&self) -> &CapabilityRef {
        &self.reference
    }

    /// Split the spec into its reference and optional code-defined
    /// implementation for host consumption.
    pub fn into_parts(self) -> CapabilitySpecParts {
        CapabilitySpecParts {
            reference: self.reference,
            #[cfg(feature = "definition")]
            definition: self.definition,
        }
    }
}

/// The decomposed contents of a [`CapabilitySpec`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CapabilitySpecParts {
    /// The reference activated for the agent.
    pub reference: CapabilityRef,
    /// The code-defined implementation to register, when present.
    #[cfg(feature = "definition")]
    pub definition: Option<crate::definition::Definition>,
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

#[cfg(feature = "definition")]
impl From<crate::definition::Definition> for CapabilitySpec {
    fn from(definition: crate::definition::Definition) -> Self {
        Self::definition(definition)
    }
}

#[cfg(feature = "definition")]
impl IntoCapability for crate::definition::Definition {
    fn into_capability(self) -> CapabilitySpec {
        self.into()
    }
}
