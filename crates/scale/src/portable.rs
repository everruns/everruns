use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use everruns::{Agent, AgentBuilder, FunctionTool, Model};
use everruns_host::{EnvironmentBindingStore, HostBackends};
use everruns_provider::runtime_provider::Provider;
use serde::{Deserialize, Serialize};

/// A portable component category used in validation errors and registry keys.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ComponentKind {
    /// Model provider implementation.
    Provider,
    /// Model-callable tool implementation.
    Tool,
    /// Agent capability installer.
    Capability,
    /// Lifecycle hook installer.
    Hook,
    /// Workspace/environment implementation.
    Workspace,
    /// Process-local event sink implementation.
    EventSink,
    /// Embedded provider or execution driver.
    Driver,
    /// Arbitrary process-local code implementation.
    Code,
}

impl fmt::Display for ComponentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Provider => "provider",
            Self::Tool => "tool",
            Self::Capability => "capability",
            Self::Hook => "hook",
            Self::Workspace => "workspace",
            Self::EventSink => "event sink",
            Self::Driver => "driver",
            Self::Code => "code implementation",
        })
    }
}

/// Stable, human-readable registry address persisted by Scale.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RegistrationPath(String);

impl RegistrationPath {
    /// Parse a stable slash-delimited path such as `providers/openai/default`.
    pub fn new(path: impl Into<String>) -> Result<Self, PortableAgentError> {
        let path = path.into();
        let valid = !path.is_empty()
            && path.len() <= 200
            && path.split('/').all(|segment| {
                !segment.is_empty()
                    && segment.len() <= 64
                    && segment != "."
                    && segment != ".."
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            });
        if !valid {
            return Err(PortableAgentError::InvalidRegistrationPath {
                component: ComponentKind::Code,
                registration_path: path,
                reason: "expected 1-200 ASCII letters, digits, '.', '_', '-' and '/' with non-empty, non-traversal segments".into(),
            });
        }
        Ok(Self(path))
    }

    /// Borrow the canonical persisted path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RegistrationPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::new(path).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for RegistrationPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A versioned, serializable agent submission containing references only.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableAgent {
    /// Persistence format version. Currently `1`.
    pub version: u16,
    /// Human-readable agent name.
    pub name: String,
    /// Instructions supplied to the model.
    pub instructions: String,
    /// Provider-specific model identifier.
    pub model: String,
    /// Stable reference for the model provider implementation.
    pub provider: RegistrationPath,
    /// Stable reference for the restartable workspace/environment implementation.
    pub workspace: RegistrationPath,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Stable references for model-callable tools.
    pub tools: Vec<RegistrationPath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Stable references for capability installers.
    pub capabilities: Vec<RegistrationPath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Stable references for lifecycle hook installers.
    pub hooks: Vec<RegistrationPath>,
}

impl PortableAgent {
    /// Begin a portable agent definition.
    pub fn builder(
        instructions: impl Into<String>,
        model: impl Into<String>,
        provider: RegistrationPath,
        workspace: RegistrationPath,
    ) -> PortableAgentBuilder {
        PortableAgentBuilder {
            name: "agent".into(),
            instructions: instructions.into(),
            model: model.into(),
            provider,
            workspace,
            tools: Vec::new(),
            capabilities: Vec::new(),
            hooks: Vec::new(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), PortableAgentError> {
        if self.version != 1 {
            return Err(PortableAgentError::UnsupportedVersion(self.version));
        }
        if self.name.trim().is_empty() || self.name.len() > 200 {
            return Err(PortableAgentError::InvalidDefinition(
                "name must contain 1-200 bytes".into(),
            ));
        }
        if self.instructions.trim().is_empty() || self.instructions.len() > 1_000_000 {
            return Err(PortableAgentError::InvalidDefinition(
                "instructions must contain 1-1,000,000 bytes".into(),
            ));
        }
        if self.model.trim().is_empty() || self.model.len() > 200 {
            return Err(PortableAgentError::InvalidDefinition(
                "model must contain 1-200 bytes".into(),
            ));
        }
        let component_count = self.tools.len() + self.capabilities.len() + self.hooks.len();
        if component_count > 256 {
            return Err(PortableAgentError::InvalidDefinition(
                "portable agents support at most 256 tool, capability, and hook references".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for (kind, path) in self.references() {
            if !seen.insert((kind, path.clone())) {
                return Err(PortableAgentError::DuplicateRegistration {
                    component: kind,
                    registration_path: path.to_string(),
                });
            }
        }
        Ok(())
    }

    fn references(&self) -> impl Iterator<Item = (ComponentKind, &RegistrationPath)> {
        std::iter::once((ComponentKind::Provider, &self.provider))
            .chain(std::iter::once((ComponentKind::Workspace, &self.workspace)))
            .chain(self.tools.iter().map(|path| (ComponentKind::Tool, path)))
            .chain(
                self.capabilities
                    .iter()
                    .map(|path| (ComponentKind::Capability, path)),
            )
            .chain(self.hooks.iter().map(|path| (ComponentKind::Hook, path)))
    }
}

/// Builder that has no API for closures, drivers, sinks, or code definitions.
pub struct PortableAgentBuilder {
    name: String,
    instructions: String,
    model: String,
    provider: RegistrationPath,
    workspace: RegistrationPath,
    tools: Vec<RegistrationPath>,
    capabilities: Vec<RegistrationPath>,
    hooks: Vec<RegistrationPath>,
}

impl PortableAgentBuilder {
    /// Set the human-readable agent name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Add a registered tool reference.
    pub fn tool(mut self, path: RegistrationPath) -> Self {
        self.tools.push(path);
        self
    }

    /// Add a registered capability reference.
    pub fn capability(mut self, path: RegistrationPath) -> Self {
        self.capabilities.push(path);
        self
    }

    /// Add a registered lifecycle hook reference.
    pub fn hook(mut self, path: RegistrationPath) -> Self {
        self.hooks.push(path);
        self
    }

    /// Validate and build the versioned portable definition.
    pub fn build(self) -> Result<PortableAgent, PortableAgentError> {
        let definition = PortableAgent {
            version: 1,
            name: self.name,
            instructions: self.instructions,
            model: self.model,
            provider: self.provider,
            workspace: self.workspace,
            tools: self.tools,
            capabilities: self.capabilities,
            hooks: self.hooks,
        };
        definition.validate()?;
        Ok(definition)
    }
}

type Installer = Arc<dyn Fn(AgentBuilder) -> AgentBuilder + Send + Sync>;

/// Process-local implementations addressed by stable portable paths.
///
/// The implementations themselves are never persisted. Registration is an
/// application bootstrap concern and duplicate paths fail closed.
#[derive(Clone, Default)]
pub struct Registry {
    providers: BTreeMap<RegistrationPath, Provider>,
    tools: BTreeMap<RegistrationPath, FunctionTool>,
    capabilities: BTreeMap<RegistrationPath, Installer>,
    hooks: BTreeMap<RegistrationPath, Installer>,
    workspaces: BTreeMap<RegistrationPath, Installer>,
}

impl Registry {
    /// Create an empty process-local implementation registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider implementation at a stable path.
    pub fn register_provider(
        &mut self,
        path: RegistrationPath,
        provider: Provider,
    ) -> Result<(), PortableAgentError> {
        insert_unique(&mut self.providers, path, provider, ComponentKind::Provider)
    }

    /// Register a tool implementation at a stable path.
    pub fn register_tool(
        &mut self,
        path: RegistrationPath,
        tool: FunctionTool,
    ) -> Result<(), PortableAgentError> {
        insert_unique(&mut self.tools, path, tool, ComponentKind::Tool)
    }

    /// Register a capability installer at a stable path.
    pub fn register_capability<F>(
        &mut self,
        path: RegistrationPath,
        installer: F,
    ) -> Result<(), PortableAgentError>
    where
        F: Fn(AgentBuilder) -> AgentBuilder + Send + Sync + 'static,
    {
        insert_unique(
            &mut self.capabilities,
            path,
            Arc::new(installer),
            ComponentKind::Capability,
        )
    }

    /// Register a lifecycle hook installer at a stable path.
    pub fn register_hook<F>(
        &mut self,
        path: RegistrationPath,
        installer: F,
    ) -> Result<(), PortableAgentError>
    where
        F: Fn(AgentBuilder) -> AgentBuilder + Send + Sync + 'static,
    {
        insert_unique(
            &mut self.hooks,
            path,
            Arc::new(installer),
            ComponentKind::Hook,
        )
    }

    /// Register the process-local workspace/environment installer for a stable path.
    pub fn register_workspace<F>(
        &mut self,
        path: RegistrationPath,
        installer: F,
    ) -> Result<(), PortableAgentError>
    where
        F: Fn(AgentBuilder) -> AgentBuilder + Send + Sync + 'static,
    {
        insert_unique(
            &mut self.workspaces,
            path,
            Arc::new(installer),
            ComponentKind::Workspace,
        )
    }

    pub(crate) fn resolve(&self, definition: &PortableAgent) -> Result<Agent, PortableAgentError> {
        self.resolve_configured(definition, None)
    }

    pub(crate) fn resolve_with_backends(
        &self,
        definition: &PortableAgent,
        backends: HostBackends,
        binding_store: Arc<dyn EnvironmentBindingStore>,
    ) -> Result<Agent, PortableAgentError> {
        self.resolve_configured(definition, Some((backends, binding_store)))
    }

    fn resolve_configured(
        &self,
        definition: &PortableAgent,
        execution: Option<(HostBackends, Arc<dyn EnvironmentBindingStore>)>,
    ) -> Result<Agent, PortableAgentError> {
        definition.validate()?;
        let provider = self.providers.get(&definition.provider).ok_or_else(|| {
            PortableAgentError::MissingRegistration {
                component: ComponentKind::Provider,
                registration_path: definition.provider.to_string(),
            }
        })?;
        let mut builder = Agent::builder()
            .name(definition.name.clone())
            .instructions(definition.instructions.clone())
            .model(Model::from(definition.model.clone()))
            .provider(provider.clone());
        if let Some((backends, binding_store)) = execution {
            builder = builder.execution_backends(backends, binding_store);
        }
        let installer = self.workspaces.get(&definition.workspace).ok_or_else(|| {
            PortableAgentError::MissingRegistration {
                component: ComponentKind::Workspace,
                registration_path: definition.workspace.to_string(),
            }
        })?;
        builder = installer(builder);
        for path in &definition.tools {
            let tool =
                self.tools
                    .get(path)
                    .ok_or_else(|| PortableAgentError::MissingRegistration {
                        component: ComponentKind::Tool,
                        registration_path: path.to_string(),
                    })?;
            builder = builder.tool(tool.clone());
        }
        for (kind, paths, installers) in [
            (
                ComponentKind::Capability,
                &definition.capabilities,
                &self.capabilities,
            ),
            (ComponentKind::Hook, &definition.hooks, &self.hooks),
        ] {
            for path in paths {
                let installer = installers.get(path).ok_or_else(|| {
                    PortableAgentError::MissingRegistration {
                        component: kind,
                        registration_path: path.to_string(),
                    }
                })?;
                builder = installer(builder);
            }
        }
        builder
            .build()
            .map_err(|error| PortableAgentError::InvalidDefinition(error.to_string()))
    }
}

fn insert_unique<T>(
    values: &mut BTreeMap<RegistrationPath, T>,
    path: RegistrationPath,
    value: T,
    component: ComponentKind,
) -> Result<(), PortableAgentError> {
    match values.entry(path) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
        Entry::Occupied(entry) => Err(PortableAgentError::DuplicateRegistration {
            component,
            registration_path: entry.key().to_string(),
        }),
    }
}

/// Typed portable-definition and registry failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PortableAgentError {
    /// The persisted definition uses an unsupported format version.
    #[error("unsupported portable agent version {0}")]
    UnsupportedVersion(u16),
    /// A bounded portable field or reference set is invalid.
    #[error("invalid portable agent definition: {0}")]
    InvalidDefinition(String),
    /// A component address is not a valid stable registration path.
    #[error("invalid {component} registration path {registration_path:?}: {reason}")]
    InvalidRegistrationPath {
        /// Kind of component addressed by the path.
        component: ComponentKind,
        /// Rejected registration path.
        registration_path: String,
        /// Stable validation explanation.
        reason: String,
    },
    /// A portable reference has no process-local implementation registration.
    #[error("missing {component} registration at {registration_path}")]
    MissingRegistration {
        /// Kind of missing component.
        component: ComponentKind,
        /// Unresolved stable registration path.
        registration_path: String,
    },
    /// Startup attempted to register two implementations at the same path.
    #[error("duplicate {component} registration at {registration_path}")]
    DuplicateRegistration {
        /// Kind of duplicate component.
        component: ComponentKind,
        /// Duplicated stable registration path.
        registration_path: String,
    },
    /// Scale could not initialize or access its persistence layer.
    #[error("scale persistence is unavailable: {0}")]
    Persistence(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_paths_are_stable_and_reject_code_like_inputs() {
        assert_eq!(
            RegistrationPath::new("tools/weather/v1").unwrap().as_str(),
            "tools/weather/v1"
        );
        for invalid in [
            "",
            "/tool",
            "tool/",
            "tool//v1",
            "tool/../v1",
            "tool::{closure}",
            "tool path",
        ] {
            assert!(
                RegistrationPath::new(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
        assert!(
            serde_json::from_str::<RegistrationPath>(r#""tool/../v1""#).is_err(),
            "deserialization must not bypass path validation"
        );
    }

    #[test]
    fn portable_json_contains_references_not_implementations() {
        let definition = PortableAgent::builder(
            "portable",
            "model",
            RegistrationPath::new("providers/openai/default").unwrap(),
            RegistrationPath::new("workspaces/project/v1").unwrap(),
        )
        .tool(RegistrationPath::new("tools/weather/v1").unwrap())
        .capability(RegistrationPath::new("capabilities/files/v1").unwrap())
        .hook(RegistrationPath::new("hooks/audit/v1").unwrap())
        .build()
        .unwrap();
        let json = serde_json::to_string(&definition).unwrap();
        for forbidden in ["closure", "driver", "event_sink", "function_pointer", "fn("] {
            assert!(!json.contains(forbidden));
        }
        assert!(json.contains("tools/weather/v1"));
        assert!(json.contains("workspaces/project/v1"));
    }

    #[test]
    fn scale_manifest_has_no_server_or_worker_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("everruns-server"));
        assert!(!manifest.contains("everruns-worker"));
    }

    #[test]
    fn duplicate_registration_does_not_replace_the_original_implementation() {
        let path = RegistrationPath::new("tools/example/v1").unwrap();
        let mut values = BTreeMap::new();
        insert_unique(&mut values, path.clone(), 1, ComponentKind::Tool).unwrap();
        assert!(matches!(
            insert_unique(&mut values, path.clone(), 2, ComponentKind::Tool),
            Err(PortableAgentError::DuplicateRegistration {
                component: ComponentKind::Tool,
                ..
            })
        ));
        assert_eq!(values.get(&path), Some(&1));
    }
}
