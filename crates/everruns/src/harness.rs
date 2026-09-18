//! Application-facing harness configuration.

use std::fmt;
use std::sync::Arc;

use everruns_capability::serde::{Deserialize, Deserializer, Serialize, Serializer};
use everruns_host::{
    ComputeCapabilities, ContainmentLevel, HarnessBuilder as RuntimeHarnessBuilder,
};
use everruns_provider::typed_id::HarnessId;

use crate::CapabilityRef;
use crate::session::{EnvironmentSessionBuilder, Session, SessionEnvironmentError};

/// A reusable description of what an agent runs on.
///
/// A Harness declares the capabilities available to a session, the Environment
/// properties it requires, and a lowest-precedence model default. Agent
/// instructions and starter files do not belong to this value.
///
/// Serialization contains only the portable definition. Deserialization
/// validates that definition and creates a new runtime association identity.
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

#[derive(Serialize)]
#[serde(crate = "everruns_capability::serde")]
struct HarnessDefinitionRef<'a> {
    name: &'a str,
    required_capabilities: ComputeCapabilities,
    required_containment: ContainmentLevel,
    capabilities: &'a [CapabilityRef],
    default_model: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(crate = "everruns_capability::serde")]
struct HarnessDefinitionValue {
    name: String,
    required_capabilities: ComputeCapabilities,
    required_containment: ContainmentLevel,
    #[serde(default)]
    capabilities: Vec<CapabilityRef>,
    #[serde(default)]
    default_model: Option<String>,
}

impl Serialize for Harness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        HarnessDefinitionRef {
            name: self.name(),
            required_capabilities: self.required_capabilities(),
            required_containment: self.required_containment(),
            capabilities: self.capabilities(),
            default_model: self.default_model(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Harness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = HarnessDefinitionValue::deserialize(deserializer)?;
        let mut builder = Self::builder(value.name)
            .requires_capabilities(value.required_capabilities)
            .requires_containment(value.required_containment);
        for capability in value.capabilities {
            builder = builder.capability(capability);
        }
        if let Some(model) = value.default_model {
            builder = builder.model(model);
        }
        builder
            .build()
            .map_err(everruns_capability::serde::de::Error::custom)
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

        let registry = crate::capability_config::framework_capability_registry(false);
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
            crate::capability_config::validate_registered_capability_config(
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

impl Session {
    /// Permanently bind this new session to an explicit Environment.
    ///
    /// [`EnvironmentSessionBuilder::start`] persists the opaque head binding
    /// before any runtime can execute. The same head is observable through
    /// [`workspace_head`](Self::workspace_head) for the session lifetime.
    pub fn environment(self, environment: everruns_host::Environment) -> EnvironmentSessionBuilder {
        EnvironmentSessionBuilder {
            session: self,
            environment,
        }
    }

    /// Permanently bind this new session to a workspace head.
    ///
    /// This is the common workspace-only form of [`environment`](Self::environment):
    /// `engine.create(agent).workspace(head).start().await?`.
    pub fn workspace(self, head: everruns_host::WorkspaceHead) -> EnvironmentSessionBuilder {
        self.environment(everruns_host::Environment::new(head))
    }

    /// Bind this new session permanently to a Harness.
    ///
    /// The Harness remains optional. Without this call, session construction
    /// follows the existing anonymous-harness path.
    pub fn harness(self, harness: Harness) -> HarnessSessionBuilder {
        HarnessSessionBuilder {
            session: self,
            harness,
        }
    }

    fn bind_harness(&self, harness: Harness) -> Result<(), SessionEnvironmentError> {
        if self.has_started() {
            return Err(SessionEnvironmentError::AlreadyStarted);
        }
        if let Some(bound) = self.inner.harness.get() {
            return if bound.is_same_binding(&harness) {
                Ok(())
            } else {
                Err(SessionEnvironmentError::HarnessAlreadyBound)
            };
        }
        self.inner.execution.bind_harness(harness.clone())?;
        self.inner
            .harness
            .set(harness)
            .map_err(|_| SessionEnvironmentError::HarnessAlreadyBound)
    }
}

/// A not-yet-running Session with its Harness selected.
pub struct HarnessSessionBuilder {
    session: Session,
    harness: Harness,
}

impl HarnessSessionBuilder {
    /// Select an Environment for this Harness-bound Session.
    pub fn environment(
        self,
        environment: everruns_host::Environment,
    ) -> HarnessEnvironmentSessionBuilder {
        HarnessEnvironmentSessionBuilder {
            session: self.session,
            harness: self.harness,
            environment,
        }
    }

    /// Select a workspace head for this Harness-bound Session.
    pub fn workspace(self, head: everruns_host::WorkspaceHead) -> HarnessEnvironmentSessionBuilder {
        self.environment(everruns_host::Environment::new(head))
    }

    /// Freeze the Harness binding and select the Agent's default Environment.
    pub async fn start(self) -> Result<Session, SessionEnvironmentError> {
        self.session.bind_harness(self.harness)?;
        self.session.start().await?;
        Ok(self.session)
    }
}

/// A not-yet-running Session with its Harness and Environment selected.
pub struct HarnessEnvironmentSessionBuilder {
    session: Session,
    harness: Harness,
    environment: everruns_host::Environment,
}

impl HarnessEnvironmentSessionBuilder {
    /// Persist and freeze both bindings before execution starts.
    pub async fn start(self) -> Result<Session, SessionEnvironmentError> {
        self.session.bind_harness(self.harness)?;
        EnvironmentSessionBuilder {
            session: self.session,
            environment: self.environment,
        }
        .start()
        .await
    }
}

impl EnvironmentSessionBuilder {
    /// Add a Harness to the selected Environment before starting.
    pub fn harness(self, harness: Harness) -> HarnessEnvironmentSessionBuilder {
        HarnessEnvironmentSessionBuilder {
            session: self.session,
            harness,
            environment: self.environment,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialization_regenerates_runtime_association_identity() {
        let harness = Harness::builder("portable").build().unwrap();
        let serialized = serde_json::to_string(&harness).unwrap();
        let deserialized: Harness = serde_json::from_str(&serialized).unwrap();

        assert_ne!(deserialized.inner.id, harness.inner.id);
    }
}
