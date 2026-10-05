//! Portable agent authoring adapters.
use super::*;

impl Agent {
    /// Export authored configuration and assets. Function tools export their
    /// schemas; importing them requires host handlers. Code-defined capabilities
    /// and hooks require a host binding and cannot be serialized as behavior.
    pub fn to_package(&self) -> Result<crate::AgentPackage, crate::PackageError> {
        use crate::package::failure;
        use everruns_core::tools::Tool as _;
        if !self.lifecycle_hooks.is_empty() {
            return Err(failure(
                "bindings",
                "lifecycle hooks require host code; export the original package instead",
            ));
        }
        let mut m = self.package_manifest.clone().unwrap_or_else(|| {
            serde_json::from_value(
                serde_json::json!({"name":self.name,"instructions":self.instructions}),
            )
            .expect("manifest defaults")
        });
        m.name = self.name.clone();
        m.instructions = self.instructions.clone();
        m.model = Some(self.model.clone());
        m.initial_files = self
            .initial_files
            .iter()
            .cloned()
            .map(everruns_core::agent_package::File::Inline)
            .collect();
        m.capabilities = self
            .capabilities
            .iter()
            .filter(|c| {
                !self.capability_implementations.iter().any(
                    |i| matches!(i, CapabilityImplementation::Function(t) if t.name() == c.id()),
                )
            })
            .cloned()
            .collect();
        m.max_iterations = self.max_iterations;
        m.parallel_tool_calls = self.parallel_tool_calls;
        m.network_access = self.network_access.clone();
        m.mcp_servers = everruns_core::agent_package::redact_mcp(&self.mcp_servers);
        m.tools.clear();
        for implementation in &self.capability_implementations {
            match implementation {
                CapabilityImplementation::Function(tool) if tool.approval().is_none() => m
                    .tools
                    .push(everruns_contracts::tool_types::ToolDefinition::function(
                        tool.name(),
                        tool.description(),
                        tool.schema().clone(),
                    )),
                _ => {
                    return Err(failure(
                        "bindings",
                        "code-defined capabilities, approval gates and interactive handlers require host code; export the original package instead",
                    ));
                }
            }
        }
        crate::AgentPackage::new(m)
    }
}

impl AgentBuilder {
    /// Apply a portable file definition after binding any required custom tools.
    /// Provider credentials and channel transports remain host-owned.
    pub fn package(mut self, package: &crate::AgentPackage) -> Result<Self, crate::PackageError> {
        use crate::package::failure;
        let m = package.manifest();
        let files = package.0.files()?;
        if m.sandbox_policy.is_some() {
            return Err(failure(
                "sandbox_policy",
                "bind the hosted Sandbox policy explicitly",
            ));
        }
        let registry = framework_capability_registry(false);
        for reference in &m.capabilities {
            if let Some(bound) = self
                .capabilities
                .iter()
                .find(|c| c.capability_ref().id() == reference.id())
            {
                if bound.capability_ref().config_value() != reference.config_value() {
                    return Err(failure(
                        "capabilities",
                        format!("bound config differs for {}", reference.id()),
                    ));
                }
                continue;
            }
            if !registry.has(reference.id())
                && !everruns_core::is_declarative_capability(reference.id())
            {
                return Err(failure(
                    "capabilities",
                    format!(
                        "{} requires a host implementation or a Cargo feature",
                        reference.id()
                    ),
                ));
            }
            self = self.capability(reference.clone());
        }
        for required in &m.tools {
            let bound = self
                .tools
                .iter()
                .find(|tool| tool.name() == required.name())
                .ok_or_else(|| {
                    failure(
                        "tools",
                        format!(
                            "bind {} with AgentBuilder::tool before applying the package",
                            required.name()
                        ),
                    )
                })?;
            if bound.clone().into_function().schema() != required.parameters() {
                return Err(failure(
                    "tools",
                    format!("schema differs for {}", required.name()),
                ));
            }
        }
        // A placeholder is never sent literally as a credential to a server.
        let servers = match &package.1 {
            Some(servers) => servers.clone(),
            None => package.0.bind_mcp(|_| None)?,
        };
        for (name, server) in servers {
            #[cfg(not(feature = "mcp"))]
            {
                let _ = (name, server);
                return Err(failure("mcpServers", "enable the mcp Cargo feature"));
            }
            #[cfg(feature = "mcp")]
            {
                if let Some(preset) = &server.preset {
                    if self.mcp_servers.iter().any(|bound| bound.name == name) {
                        continue;
                    }
                    return Err(failure(
                        "mcpServers",
                        format!(
                            "{preset}: bind {name} with AgentBuilder::mcp_server before applying the package"
                        ),
                    ));
                }
                #[cfg(not(feature = "mcp-stdio"))]
                if server.transport_type == everruns_core::McpServerTransportType::Stdio {
                    return Err(failure("mcpServers", "enable the mcp-stdio Cargo feature"));
                }
                self = self.mcp_server(crate::McpServer {
                    name,
                    inner: server,
                });
            }
        }
        self.name = Some(m.name.clone());
        self.instructions = Some(m.instructions.clone());
        if self.model.is_none()
            && let Some(model) = &m.model
        {
            self.model = Some(Model::from(model.model.clone()));
        }
        if let Some(model) = &m.model
            && self.providers.iter().any(|p| p.id() != &model.provider)
        {
            return Err(failure(
                "model",
                "bound provider does not match manifest provider",
            ));
        }
        self.initial_files.extend(files);
        self.max_iterations = m.max_iterations;
        self.parallel_tool_calls = m.parallel_tool_calls;
        self.network_access = m.network_access.clone();
        self.package_manifest = Some(m.clone());
        Ok(self)
    }
}
