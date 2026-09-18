//! Application-facing harness configuration.

use std::fmt;
use std::sync::Arc;

use everruns_host::{
    ComputeCapabilities, ContainmentLevel, HarnessBuilder as RuntimeHarnessBuilder,
};
use everruns_provider::typed_id::HarnessId;

use crate::CapabilityRef;

/// A reusable description of what an agent runs on.
///
/// A Harness declares the capabilities available to a session, the Environment
/// properties it requires, and a lowest-precedence model default. Agent
/// instructions and starter files do not belong to this value.
#[derive(Clone)]
pub struct Harness {
    inner: Arc<HarnessInner>,
}

struct HarnessInner {
    id: HarnessId,
    name: String,
    required_capabilities: ComputeCapabilities,
    required_containment: ContainmentLevel,
    capabilities: Vec<CapabilityRef>,
    default_model: Option<String>,
}

impl Harness {
    /// Start describing a named Harness.
    pub fn builder(name: impl Into<String>) -> HarnessBuilder {
        HarnessBuilder {
            name: name.into(),
            required_capabilities: ComputeCapabilities::default(),
            required_containment: ContainmentLevel::None,
            capabilities: Vec::new(),
            default_model: None,
        }
    }

    /// The application-defined Harness name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Compute features an Environment must provide.
    ///
    /// The declaration is recorded but not negotiated during session creation.
    pub fn required_capabilities(&self) -> ComputeCapabilities {
        self.inner.required_capabilities
    }

    /// The minimum containment level an Environment must provide.
    ///
    /// The declaration is recorded but not negotiated during session creation.
    pub fn required_containment(&self) -> ContainmentLevel {
        self.inner.required_containment
    }

    /// Capability references activated by this Harness.
    pub fn capabilities(&self) -> &[CapabilityRef] {
        &self.inner.capabilities
    }

    /// The Harness's lowest-precedence model default.
    ///
    /// Framework Agents currently require their own model, which overrides this
    /// value when the Harness is bound.
    pub fn default_model(&self) -> Option<&str> {
        self.inner.default_model.as_deref()
    }

    pub(crate) fn is_same_binding(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn seeded(&self) -> everruns_host::SeededHarness {
        RuntimeHarnessBuilder::new(self.name(), "")
            .id(self.inner.id)
            .capabilities(self.capabilities().iter().cloned())
            .build()
    }
}

impl fmt::Debug for Harness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Harness")
            .field("name", &self.inner.name)
            .field("required_capabilities", &self.inner.required_capabilities)
            .field("required_containment", &self.inner.required_containment)
            .field(
                "capabilities",
                &self
                    .inner
                    .capabilities
                    .iter()
                    .map(CapabilityRef::id)
                    .collect::<Vec<_>>(),
            )
            .field("default_model", &self.inner.default_model)
            .finish()
    }
}

/// Fluent builder behind [`Harness::builder`].
#[derive(Clone, Debug)]
pub struct HarnessBuilder {
    name: String,
    required_capabilities: ComputeCapabilities,
    required_containment: ContainmentLevel,
    capabilities: Vec<CapabilityRef>,
    default_model: Option<String>,
}

impl HarnessBuilder {
    /// Declare the compute features the bound Environment must provide.
    pub fn requires_capabilities(mut self, capabilities: ComputeCapabilities) -> Self {
        self.required_capabilities = capabilities;
        self
    }

    /// Declare the minimum containment level the bound Environment must provide.
    pub fn requires_containment(mut self, containment: ContainmentLevel) -> Self {
        self.required_containment = containment;
        self
    }

    /// Add one serializable capability reference to the Harness.
    pub fn capability(mut self, capability: impl Into<CapabilityRef>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Set the Harness's lowest-precedence model default.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.default_model = Some(model.into());
        self
    }

    /// Validate and build the Harness.
    pub fn build(self) -> Result<Harness, HarnessBuildError> {
        if self.name.trim().is_empty() {
            return Err(HarnessBuildError::BlankName);
        }

        let registry = crate::agent::framework_capability_registry(false);
        let mut activated = everruns_capability::ActivationSet::new();
        let mut capabilities = Vec::with_capacity(self.capabilities.len());
        for capability in self.capabilities {
            capability
                .validate()
                .map_err(|error| HarnessBuildError::InvalidCapability {
                    id: capability.id().to_string(),
                    reason: error.reason(),
                })?;
            let canonical_id = registry
                .canonical_id(capability.id())
                .unwrap_or(capability.id())
                .to_string();
            crate::agent::validate_registered_capability_config(
                &registry,
                &canonical_id,
                capability.config_value(),
            )
            .map_err(|error| match error {
                crate::BuildError::InvalidCapability { id, reason } => {
                    HarnessBuildError::InvalidCapability { id, reason }
                }
                _ => unreachable!("capability config validation has one error shape"),
            })?;
            activated.activate(canonical_id.clone()).map_err(|_| {
                HarnessBuildError::DuplicateCapability {
                    id: canonical_id.clone(),
                }
            })?;
            capabilities.push(CapabilityRef::with_config(
                canonical_id,
                capability.config_value().clone(),
            ));
        }

        Ok(Harness {
            inner: Arc::new(HarnessInner {
                id: HarnessId::new(),
                name: self.name,
                required_capabilities: self.required_capabilities,
                required_containment: self.required_containment,
                capabilities,
                default_model: self.default_model,
            }),
        })
    }
}

/// Why a [`HarnessBuilder`] could not produce a [`Harness`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HarnessBuildError {
    /// The Harness name was empty or only whitespace.
    BlankName,
    /// A capability identifier or configuration was invalid.
    InvalidCapability {
        /// The rejected capability id.
        id: String,
        /// Why the capability was rejected.
        reason: String,
    },
    /// Two capability references resolve to the same implementation id.
    DuplicateCapability {
        /// The colliding capability id.
        id: String,
    },
}

impl fmt::Display for HarnessBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankName => formatter.write_str("harness name must not be blank"),
            Self::InvalidCapability { id, reason } => {
                write!(formatter, "invalid capability {id:?}: {reason}")
            }
            Self::DuplicateCapability { id } => {
                write!(formatter, "duplicate capability id {id:?}")
            }
        }
    }
}

impl std::error::Error for HarnessBuildError {}
