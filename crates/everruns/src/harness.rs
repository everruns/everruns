//! Application-facing harness configuration.

use std::fmt;
use std::sync::Arc;

use everruns_capability::serde::{Deserialize, Deserializer, Serialize, Serializer};
use everruns_host::{
    ComputeCapabilities, ContainmentLevel, HarnessBuilder as RuntimeHarnessBuilder,
};
use everruns_provider::typed_id::HarnessId;

use crate::session::{EnvironmentSessionBuilder, Session};
use crate::{CapabilityRef, SessionEnvironmentError};
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

    /// The `generic` Harness: the same capability floor a hosted org is
    /// provisioned with.
    ///
    /// One definition, not a copy. Both this and `crates/server/src/harnesses/`
    /// read `everruns_capability::generic_capabilities`, so an application
    /// stops approximating the platform default with a builder chain that
    /// silently drifts from it (EVE-1041).
    ///
    /// What it does *not* carry is the platform's base system prompt or its
    /// presentation fields — a Harness holds neither, by design. Compose
    /// instructions on the Agent and code-defined capabilities on top; the
    /// harness is the floor, not a ceiling.
    ///
    /// Capabilities the built binary did not compile in are inert rather than
    /// fatal, the same as any other reference to an unregistered capability,
    /// so this is usable from a facade built with a narrower feature set.
    pub fn generic() -> Self {
        let mut builder = Harness::builder(everruns_capability::GENERIC_HARNESS_NAME);
        for capability in everruns_capability::generic_capabilities() {
            builder = builder.capability(capability);
        }
        builder
            .build()
            .expect("the shared generic capability list is a valid Harness")
    }

    /// The application-defined Harness name.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Compute features an Environment must provide.
    pub fn required_capabilities(&self) -> ComputeCapabilities {
        self.inner.required_capabilities
    }

    /// The minimum containment level an Environment must provide.
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
    pub(crate) fn negotiate(
        &self,
        environment: &everruns_host::Environment,
    ) -> Result<(), SessionEnvironmentError> {
        let required = self.required_capabilities();
        let available = environment.capabilities();
        for (capability, required, available) in [
            (
                "native_processes",
                required.native_processes,
                available.native_processes,
            ),
            ("packages", required.packages, available.packages),
            ("pty", required.pty, available.pty),
            ("ports", required.ports, available.ports),
            (
                "portable_checkpoint",
                required.portable_checkpoint,
                available.portable_checkpoint,
            ),
            (
                "network_enforced",
                required.network_enforced,
                available.network_enforced,
            ),
        ] {
            if required && !available {
                return Err(SessionEnvironmentError::MissingHarnessCapability { capability });
            }
        }

        let available = environment.containment().level;
        let required = self.required_containment();
        if available < required {
            return Err(SessionEnvironmentError::InsufficientHarnessContainment {
                required,
                available,
            });
        }
        Ok(())
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

    pub(crate) fn bind_harness(&self, harness: Harness) -> Result<(), SessionEnvironmentError> {
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
        self.session.start_with_harness(self.harness).await?;
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
        self.harness.negotiate(&self.environment)?;
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
    use std::sync::Arc;

    use super::*;
    use async_trait::async_trait;
    use everruns_host::{
        Compute, ComputeError, ComputeKind, ComputeSession, Durability, Environment, WorkspaceHead,
    };
    struct TestCompute {
        kind: ComputeKind,
        capabilities: ComputeCapabilities,
        containment: ContainmentLevel,
    }

    #[async_trait]
    impl Compute for TestCompute {
        fn id(&self) -> &str {
            "test"
        }

        fn kind(&self) -> ComputeKind {
            self.kind
        }

        fn capabilities(&self) -> ComputeCapabilities {
            self.capabilities
        }

        fn enforced_containment(&self) -> ContainmentLevel {
            self.containment
        }

        fn durability(&self) -> Durability {
            Durability::Checkpointed
        }

        async fn connect(
            &self,
            _head: &WorkspaceHead,
        ) -> Result<Arc<dyn ComputeSession>, ComputeError> {
            Err(ComputeError::Unsupported("test execution"))
        }
    }

    fn test_agent() -> crate::Agent {
        crate::Agent::builder()
            .instructions("Reply deterministically.")
            .model(crate::Model::simulated("ok"))
            .build()
            .expect("valid test agent")
    }

    async fn test_environment(
        session: &Session,
        kind: ComputeKind,
        capabilities: ComputeCapabilities,
        containment: ContainmentLevel,
    ) -> Environment {
        let default = session
            .inner
            .execution
            .default_environment()
            .await
            .expect("default environment");
        Environment::builder()
            .workspace(default.workspace_head().clone())
            .compute(Arc::new(TestCompute {
                kind,
                capabilities,
                containment,
            }))
            .build()
            .expect("valid test environment")
    }

    #[test]
    fn generic_is_the_shared_platform_floor() {
        let harness = Harness::generic();
        assert_eq!(harness.name(), "generic");

        let ids: Vec<&str> = harness
            .capabilities()
            .iter()
            .map(|capability| capability.capability_id())
            .collect();
        let shared = everruns_capability::generic_capabilities();
        assert_eq!(ids.len(), shared.len());
        for capability in &shared {
            assert!(
                ids.contains(&capability.capability_id()),
                "{} missing from the Framework generic harness",
                capability.capability_id()
            );
        }
    }

    #[test]
    fn generic_carries_capability_config_through() {
        // The floor is references *plus config*; dropping the config half
        // would make the two surfaces agree on names while behaving
        // differently, which is the drift this is meant to end.
        let harness = Harness::generic();
        let web_fetch = harness
            .capabilities()
            .iter()
            .find(|capability| capability.capability_id() == "web_fetch")
            .expect("web_fetch is part of generic");
        assert_eq!(
            web_fetch.config_value().get("enable_file_download"),
            Some(&everruns_capability::serde_json::json!(true))
        );
    }

    #[test]
    fn generic_declares_no_environment_requirement() {
        // `generic` is the portable floor: it must bind to a session with no
        // Environment, which is what `engine.create(agent).harness(..)` does
        // without `.environment(..)`.
        let harness = Harness::generic();
        assert_eq!(harness.required_containment(), ContainmentLevel::None);
    }

    #[test]
    fn deserialization_regenerates_runtime_association_identity() {
        let harness = Harness::builder("portable").build().unwrap();
        let serialized = serde_json::to_string(&harness).unwrap();
        let deserialized: Harness = serde_json::from_str(&serialized).unwrap();

        assert_ne!(deserialized.inner.id, harness.inner.id);
    }

    #[tokio::test]
    async fn missing_native_processes_is_rejected_at_session_start() {
        let session = crate::InMemoryEngine::new().create(test_agent());
        let environment = test_environment(
            &session,
            ComputeKind::Vfs,
            ComputeCapabilities::default(),
            ContainmentLevel::Isolated,
        )
        .await;
        let harness = Harness::builder("coding")
            .requires_capabilities(ComputeCapabilities {
                native_processes: true,
                ..Default::default()
            })
            .build()
            .expect("valid harness");

        let error = session
            .harness(harness)
            .environment(environment)
            .start()
            .await
            .err()
            .expect("missing native processes must reject the session");
        assert_eq!(
            error,
            SessionEnvironmentError::MissingHarnessCapability {
                capability: "native_processes"
            }
        );
        assert!(error.to_string().contains("native_processes"));
    }

    #[tokio::test]
    async fn native_process_harness_starts_on_compatible_container() {
        let session = crate::InMemoryEngine::new().create(test_agent());
        let environment = test_environment(
            &session,
            ComputeKind::Container,
            ComputeCapabilities {
                native_processes: true,
                ..Default::default()
            },
            ContainmentLevel::Isolated,
        )
        .await;
        let harness = Harness::builder("coding")
            .requires_capabilities(ComputeCapabilities {
                native_processes: true,
                ..Default::default()
            })
            .build()
            .expect("valid harness");

        session
            .harness(harness)
            .environment(environment)
            .start()
            .await
            .expect("compatible container starts");
    }

    #[tokio::test]
    async fn every_required_compute_capability_is_negotiated_environment_first() {
        let requirements = [
            (
                "native_processes",
                ComputeCapabilities {
                    native_processes: true,
                    ..Default::default()
                },
            ),
            (
                "packages",
                ComputeCapabilities {
                    packages: true,
                    ..Default::default()
                },
            ),
            (
                "pty",
                ComputeCapabilities {
                    pty: true,
                    ..Default::default()
                },
            ),
            (
                "ports",
                ComputeCapabilities {
                    ports: true,
                    ..Default::default()
                },
            ),
            (
                "portable_checkpoint",
                ComputeCapabilities {
                    portable_checkpoint: true,
                    ..Default::default()
                },
            ),
            (
                "network_enforced",
                ComputeCapabilities {
                    network_enforced: true,
                    ..Default::default()
                },
            ),
        ];

        for (capability, required) in requirements {
            let session = crate::InMemoryEngine::new().create(test_agent());
            let environment = test_environment(
                &session,
                ComputeKind::Vfs,
                ComputeCapabilities::default(),
                ContainmentLevel::Isolated,
            )
            .await;
            let harness = Harness::builder("requirements")
                .requires_capabilities(required)
                .build()
                .expect("valid harness");

            let error = session
                .environment(environment)
                .harness(harness)
                .start()
                .await
                .err()
                .expect("missing capability must reject the session");

            assert_eq!(
                error,
                SessionEnvironmentError::MissingHarnessCapability { capability }
            );
        }
    }

    #[tokio::test]
    async fn containment_requirement_uses_environment_ordering() {
        let session = crate::InMemoryEngine::new().create(test_agent());
        let environment = test_environment(
            &session,
            ComputeKind::Container,
            ComputeCapabilities::default(),
            ContainmentLevel::Native,
        )
        .await;
        let harness = Harness::builder("isolated")
            .requires_containment(ContainmentLevel::Isolated)
            .build()
            .expect("valid harness");

        let error = session
            .harness(harness)
            .environment(environment)
            .start()
            .await
            .err()
            .expect("native containment is weaker than isolated");

        assert_eq!(
            error,
            SessionEnvironmentError::InsufficientHarnessContainment {
                required: ContainmentLevel::Isolated,
                available: ContainmentLevel::Native,
            }
        );

        let session = crate::InMemoryEngine::new().create(test_agent());
        let environment = test_environment(
            &session,
            ComputeKind::Container,
            ComputeCapabilities::default(),
            ContainmentLevel::Isolated,
        )
        .await;
        let harness = Harness::builder("native")
            .requires_containment(ContainmentLevel::Native)
            .build()
            .expect("valid harness");

        session
            .environment(environment)
            .harness(harness)
            .start()
            .await
            .expect("isolated containment satisfies native");
    }

    #[tokio::test]
    async fn environment_without_harness_skips_negotiation() {
        let session = crate::InMemoryEngine::new().create(test_agent());
        let environment = test_environment(
            &session,
            ComputeKind::Vfs,
            ComputeCapabilities::default(),
            ContainmentLevel::None,
        )
        .await;

        session
            .environment(environment)
            .start()
            .await
            .expect("an environment does not need to satisfy an absent harness");
    }
}
