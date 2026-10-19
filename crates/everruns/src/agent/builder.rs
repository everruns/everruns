//! Agent authoring and validation.
use super::*;

impl AgentBuilder {
    /// Declare a client-executed tool. Its calls park durably until
    /// [`Session::resolve_tools`](crate::Session::resolve_tools) supplies results.
    pub fn client_tool(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        self.client_tools
            .push(everruns_contracts::tool_types::ClientSideTool::new(
                name,
                description,
                parameters,
            ));
        self
    }
    /// Set the network policy applied to this agent and its sessions.
    pub fn network_access(
        mut self,
        access: everruns_core::network_access::NetworkAccessList,
    ) -> Self {
        self.network_access = Some(access);
        self
    }

    /// Set the agent's system instructions. Required.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Select the model the agent talks to. Required.
    ///
    /// Live models use their provider-visible string id. [`Model::new`] can
    /// bundle the provider, or it can be configured separately with
    /// [`provider`](Self::provider). A [`Model::simulated`] value needs no
    /// provider.
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Configure the service provider used by the selected live model.
    ///
    /// Accepts a raw [`Provider`] or a provider convenience such as
    /// [`OpenAI`](crate::providers::openai::OpenAI) with the `openai` feature.
    /// An agent currently accepts exactly one provider.
    pub fn provider(mut self, provider: impl Into<Provider>) -> Self {
        self.providers.push(provider.into());
        self
    }

    /// Compose host capability, provider, recovery and MCP credential services
    /// into every runtime Engine constructs for this Agent.
    ///
    /// See [`hosting::services`](crate::hosting::services). Services never
    /// replace Engine session authority or the Environment filesystem, and a
    /// host capability that collides with a Framework capability, alias or
    /// tool fails [`build`](Self::build). A later call replaces earlier
    /// services. Stability: experimental.
    pub fn host_services(mut self, services: crate::hosting::services::HostServices) -> Self {
        self.host_services = Some(services);
        self
    }

    /// Set a human-readable name. Optional; defaults to `"agent"`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a tool the agent can call.
    ///
    /// Accepts anything that is [`IntoTool`](crate::IntoTool), including a
    /// [`FunctionTool`](crate::FunctionTool) backed by an async function or
    /// closure. Capability references use [`capability`](Self::capability)
    /// instead. Tool names and JSON schemas are validated, and duplicate names
    /// rejected, at [`build`](Self::build).
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{Agent, FunctionTool, Model};
    /// use serde_json::json;
    ///
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("done"))
    ///     .capability("test_math")
    ///     .tool(FunctionTool::new(
    ///         "roll",
    ///         "Roll a die.",
    ///         json!({ "type": "object", "properties": {} }),
    ///         |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "value": 4 })) },
    ///     ))
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn tool(mut self, tool: impl IntoTool) -> Self {
        self.tools.push(tool.into_tool());
        self
    }

    /// Register a handler with an explicit executor-loss recovery policy.
    /// Invocation identities remain stable across replacement Engines.
    pub fn on_agent_start_with_recovery<F, Fut, O>(
        mut self,
        recovery: crate::HookRecovery,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::AgentStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_agent_start(handler);
        self.lifecycle_hooks
            .set_last_recovery(crate::HookPoint::AgentStart, recovery);
        self
    }

    /// Register a handler with an explicit executor-loss recovery policy.
    /// Invocation identities remain stable across replacement Engines.
    pub fn on_turn_start_with_recovery<F, Fut, O>(
        mut self,
        recovery: crate::HookRecovery,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::TurnStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_turn_start(handler);
        self.lifecycle_hooks
            .set_last_recovery(crate::HookPoint::TurnStart, recovery);
        self
    }

    /// Register a handler with an explicit executor-loss recovery policy.
    /// Invocation identities remain stable across replacement Engines.
    pub fn on_tool_start_with_recovery<F, Fut, O>(
        mut self,
        recovery: crate::HookRecovery,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::ToolStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_start(handler);
        self.lifecycle_hooks
            .set_last_recovery(crate::HookPoint::ToolStart, recovery);
        self
    }

    /// Register a handler with an explicit executor-loss recovery policy.
    /// Invocation identities remain stable across replacement Engines.
    pub fn on_tool_end_with_recovery<F, Fut, O>(
        mut self,
        recovery: crate::HookRecovery,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::ToolEndContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_end(handler);
        self.lifecycle_hooks
            .set_last_recovery(crate::HookPoint::ToolEnd, recovery);
        self
    }

    /// Register a handler with an explicit executor-loss recovery policy.
    /// Invocation identities remain stable across replacement Engines.
    pub fn on_completion_with_recovery<F, Fut, O>(
        mut self,
        recovery: crate::HookRecovery,
        handler: F,
    ) -> Self
    where
        F: Fn(crate::CompletionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_completion(handler);
        self.lifecycle_hooks
            .set_last_recovery(crate::HookPoint::Completion, recovery);
        self
    }

    /// Run an async handler before the first turn attempted by each session.
    ///
    /// Handlers run sequentially in registration order and are awaited. An
    /// error stops the turn before execution; the next call retries the full
    /// agent-start chain. Infallible handlers may return `()`. Run cancellation
    /// drops the active handler future and skips the rest. Inspecting a session
    /// neither starts the agent nor invokes this handler.
    pub fn on_agent_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::AgentStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_agent_start(handler);
        self
    }

    /// Run an async handler before every turn enters the runtime.
    ///
    /// Handlers see the exact input but cannot mutate it. They run sequentially
    /// in registration order; the first error prevents the turn. Run
    /// cancellation drops the active handler future and skips the rest.
    pub fn on_turn_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::TurnStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_turn_start(handler);
        self
    }

    /// Run an async handler before every model-requested tool call.
    ///
    /// Multiple handlers run sequentially for one call. Tool calls in a
    /// parallel batch may run their independent hook chains concurrently. A
    /// handler error blocks only that tool call and becomes a model-visible
    /// generic tool error; application error details stay on
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Later start
    /// handlers for that call are skipped. Run cancellation drops an active
    /// handler future.
    pub fn on_tool_start<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::ToolStartContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_start(handler);
        self
    }

    /// Run an async handler after every tool call reaches a terminal result.
    ///
    /// This includes calls blocked by another pre-tool hook; their context
    /// carries the resulting error. The call has already settled, so handler
    /// errors cannot undo it. All handlers still run; failures are returned on
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Run cancellation
    /// may drop the active chain with the rest of the in-flight turn.
    pub fn on_tool_end<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::ToolEndContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_tool_end(handler);
        self
    }

    /// Run an async handler after a non-cancelled turn reaches an outcome.
    ///
    /// Completion handlers run sequentially after the runtime settles. Their
    /// errors are isolated because the turn is already committed; all handlers
    /// run, and failures are appended to
    /// [`Turn::hook_failures`](crate::Turn::hook_failures). Once completion
    /// begins, cancelling that turn's run token does not interrupt the chain.
    pub fn on_completion<F, Fut, O>(mut self, handler: F) -> Self
    where
        F: Fn(crate::CompletionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = O> + Send + 'static,
        O: crate::IntoHookResult + 'static,
    {
        self.lifecycle_hooks.on_completion(handler);
        self
    }

    /// Add one capability through the Framework's open conversion contract.
    ///
    /// Accepts typed built-ins such as [`CompactionConfig`](crate::CompactionConfig)
    /// and [`ToolSearch`](crate::ToolSearch), a code-defined
    /// `capability::Definition` (with the `capabilities` feature), a dynamic
    /// [`CapabilityRef`](crate::CapabilityRef), or any third-party value that
    /// implements [`IntoCapability`](crate::IntoCapability). Plain strings are
    /// concise references with default configuration.
    ///
    /// Conversion happens immediately and is infallible. Identifier and JSON
    /// validation, built-in schema validation, duplicate detection, and
    /// implementation/reference collision checks happen together in
    /// [`build`](Self::build). A stable ID may be activated only once; duplicate
    /// inputs are errors rather than last-write-wins overrides.
    pub fn capability(mut self, capability: impl crate::IntoCapability) -> Self {
        self.capabilities.push(capability.into_capability());
        self
    }

    /// Limit the number of model/tool iterations allowed in one turn.
    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    /// Prefer (or forbid) parallel tool calls within a single reasoning step.
    pub fn parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = Some(parallel);
        self
    }

    /// Seed a file into the agent's initial workspace.
    pub fn initial_file(mut self, file: InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    /// Seed an editable UTF-8 text file into the initial workspace.
    pub fn file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.initial_files.push(InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".to_string(),
            is_readonly: false,
        });
        self
    }

    /// Seed a read-only UTF-8 text file into the initial workspace.
    pub fn readonly_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.initial_files.push(InitialFile {
            path: path.into(),
            content: content.into(),
            encoding: "text".to_string(),
            is_readonly: true,
        });
        self
    }

    /// Use one real host directory as this agent's shared default `/workspace`.
    ///
    /// The runtime rejects traversal and symlink escapes at every filesystem
    /// operation. A missing directory is created before the session first uses
    /// it and it must be selected by trusted application configuration, not
    /// model input.
    /// This is shorthand for one shared, reopenable default WorkspaceHead over
    /// that directory. Every session created from this Agent explicitly binds
    /// to that same shared head before execution. Use an explicit
    /// [`Environment`] and backend-created head when sessions need isolation,
    /// forking, or backend-specific lifecycle operations.
    pub fn workspace(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
        if !self
            .capabilities
            .iter()
            .any(|capability| capability.capability_ref().id() == "session_file_system")
        {
            self.capabilities
                .push(crate::CapabilityRef::new("session_file_system").into());
        }
        self
    }

    /// Set the read and write policy for this agent's workspace.
    ///
    /// The default is [`WorkspacePolicy::read_only`](crate::WorkspacePolicy::read_only):
    /// ordinary files are readable, writes and hidden or sensitive paths are
    /// denied, and framework-managed `.agents` content remains readable.
    /// Select [`WorkspacePolicy::read_write`](crate::WorkspacePolicy::read_write)
    /// or build narrower scopes for an explicit write opt-in.
    pub fn workspace_policy(mut self, policy: crate::WorkspacePolicy) -> Self {
        self.workspace_policy = policy;
        self
    }

    /// Register a backend for durable typed resume of backend-owned heads.
    ///
    /// Starting a session from a live Environment also remembers its backend
    /// for this Agent lifetime. Register it here when a newly constructed Agent
    /// must resume persisted sessions after process restart.
    pub fn workspace_backend(mut self, backend: Arc<dyn WorkspaceBackend>) -> Self {
        self.workspace_backends.push(backend);
        self
    }

    /// Compatibility name for [`AgentBuilder::workspace_backend`].
    #[deprecated(note = "use AgentBuilder::workspace_backend")]
    pub fn workspace_provider(self, backend: Arc<dyn WorkspaceBackend>) -> Self {
        self.workspace_backend(backend)
    }

    /// Add a scoped MCP server to this agent.
    pub fn mcp_server(mut self, server: crate::McpServer) -> Self {
        self.mcp_servers.push(server);
        self
    }

    /// Load and enable a local plugin directory.
    ///
    /// Reading and compilation happen immediately so invalid or unsafe plugin
    /// input fails before the agent is built. Non-fatal warnings are available
    /// from [`Session::inspect`](crate::Session::inspect). Plugins contribute
    /// instructions and capabilities, so select the directory from trusted
    /// application configuration rather than model or request input.
    pub fn plugin(mut self, path: impl AsRef<Path>) -> Result<Self, crate::PluginError> {
        let loaded = crate::plugin::load(path.as_ref())?;
        self.capabilities.push(loaded.capability);
        self.plugin_warnings.extend(loaded.warnings);
        Ok(self)
    }

    /// Enable restart-survivable local state and a real workspace.
    ///
    /// Task, schedule, and session identity state is stored in SQLite.
    /// Conversation history is reconstructed from the canonical local event
    /// log. Requires the `local` feature.
    #[cfg(feature = "local")]
    pub fn local(mut self, config: crate::LocalConfig) -> Self {
        self.local = Some(config);
        if !self
            .capabilities
            .iter()
            .any(|capability| capability.capability_ref().id() == "session_file_system")
        {
            self.capabilities
                .push(crate::CapabilityRef::new("session_file_system").into());
        }
        self
    }

    /// Validate the description and produce an [`Agent`].
    ///
    /// # Errors
    ///
    /// - [`BuildError::BlankInstructions`] if instructions are missing or only
    ///   whitespace.
    /// - [`BuildError::MissingModel`] if no model was set.
    /// - [`BuildError::MissingProvider`] if a live model has no provider.
    /// - [`BuildError::MultipleProviders`] if more than one provider was set.
    /// - [`BuildError::InvalidToolName`] if a function tool's name is not a
    ///   valid model-facing identifier.
    /// - [`BuildError::InvalidToolSchema`] if a function tool's JSON schema is
    ///   not a valid arguments schema.
    /// - [`BuildError::DuplicateTool`] if two `.tool(..)` calls share a name.
    /// - [`BuildError::InvalidMcpServer`] if an MCP server is unnamed,
    ///   incomplete, or duplicates another server name.
    /// - [`BuildError::InvalidCapability`] if a capability ID, JSON config, or
    ///   code-defined descriptor is invalid.
    /// - [`BuildError::DuplicateCapability`] if two inputs resolve to the same
    ///   capability implementation, including aliases and reference/implementation
    ///   collisions.
    /// - [`BuildError::MissingApprover`] if a function tool
    ///   [needs approval](crate::FunctionTool::needs_approval) and no approver
    ///   was set.
    pub fn build(self) -> Result<Agent, BuildError> {
        #[cfg(feature = "local")]
        if self.local.is_some() && self.backends.is_some() {
            return Err(BuildError::ConflictingBackends);
        }
        let mut workspace_backends = HashMap::new();
        for backend in &self.workspace_backends {
            let id = backend.id();
            if workspace_backends
                .insert(id.clone(), backend.clone())
                .is_some()
            {
                return Err(legacy_duplicate_workspace_provider(id.to_string()));
            }
        }
        let default_workspace_root = self.workspace_root.clone().or({
            #[cfg(feature = "local")]
            {
                self.local
                    .as_ref()
                    .map(|configuration| configuration.workspace_root.clone())
            }
            #[cfg(not(feature = "local"))]
            {
                None
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        let default_workspace = default_workspace_root
            .map(crate::default_workspace::DefaultWorkspace::directory)
            .unwrap_or_else(crate::default_workspace::DefaultWorkspace::in_memory);
        #[cfg(target_arch = "wasm32")]
        let default_workspace = {
            if default_workspace_root.is_some() {
                return Err(BuildError::UnsupportedHostFeature("native workspace"));
            }
            crate::default_workspace::DefaultWorkspace::in_memory()
        };
        let default_backend = default_workspace.backend();
        let default_backend_id = default_backend.id();
        if workspace_backends
            .insert(default_backend_id.clone(), default_backend)
            .is_some()
        {
            return Err(legacy_duplicate_workspace_provider(
                default_backend_id.to_string(),
            ));
        }
        let instructions = self.instructions.unwrap_or_default();
        if instructions.trim().is_empty() {
            return Err(BuildError::BlankInstructions);
        }
        let model = self.model.ok_or(BuildError::MissingModel)?;
        let mut providers = self.providers;
        if let Some(provider) = model.bundled_provider {
            providers.push(provider);
        }
        let host_providers = self
            .host_services
            .as_ref()
            .is_some_and(|services| services.providers.is_some());
        let (model, provider) = if host_providers {
            // One owner per model: host-resolved models carry no Agent provider.
            if let Some(provider) = providers.first() {
                return Err(BuildError::ConflictingProviders {
                    model: model
                        .provider_key
                        .map_or_else(|| provider.id().to_string(), |key| key.to_string()),
                    provider: "host".to_string(),
                });
            }
            let key = model.provider_key.ok_or(BuildError::MissingProvider)?;
            (ModelSpec::on(key, model.id), None)
        } else {
            if providers.is_empty() {
                return Err(BuildError::MissingProvider);
            }
            if providers.len() > 1 {
                let mut registered = providers
                    .iter()
                    .map(|provider| provider.id().to_string())
                    .collect::<Vec<_>>();
                registered.sort();
                return Err(BuildError::MultipleProviders { registered });
            }
            let provider = providers.pop().expect("provider count checked above");
            if let Some(key) = &model.provider_key
                && key != provider.id()
            {
                return Err(BuildError::ConflictingProviders {
                    model: key.to_string(),
                    provider: provider.id().to_string(),
                });
            }
            (
                ModelSpec::on(provider.id().clone(), model.id),
                Some(provider),
            )
        };
        let name = self.name.unwrap_or_else(|| "agent".to_string());

        // Function tools retain their own public entrypoint. Privately they are
        // registered as single-tool capabilities, so their names participate in
        // the same implementation-ID collision checks as every other input.
        let mut function_tools = Vec::new();
        let mut seen_tool_names: HashSet<String> = HashSet::new();
        for tool in self.tools {
            let tool_name = tool.name().to_string();
            if !seen_tool_names.insert(tool_name.clone()) {
                return Err(BuildError::DuplicateTool { name: tool_name });
            }
            let function_tool = tool.into_function();
            validate_tool_name(function_tool.name()).map_err(|reason| {
                BuildError::InvalidToolName {
                    name: tool_name.clone(),
                    reason,
                }
            })?;
            validate_tool_schema(function_tool.schema()).map_err(|reason| {
                BuildError::InvalidToolSchema {
                    name: tool_name,
                    reason,
                }
            })?;
            function_tools.push(function_tool);
        }

        let mcp_servers = crate::mcp::into_scoped(self.mcp_servers)
            .map_err(|reason| BuildError::InvalidMcpServer { reason })?;

        // Use the same built-in set that this Agent will place on each private
        // runtime. This makes build-time schema checks and implementation
        // collision detection match execution instead of relying on a closed
        // facade-owned list.
        let capability_registry = {
            #[cfg(feature = "local")]
            if self.local.is_some() {
                crate::local::local_capability_registry()
            } else {
                framework_capability_registry(false)
            }
            #[cfg(not(feature = "local"))]
            {
                framework_capability_registry(false)
            }
        };

        let mut capabilities = Vec::new();
        let mut capability_implementations = Vec::new();
        // Neutral duplicate/collision rejection shared with product registries.
        let mut activated_capabilities = everruns_contracts::ActivationSet::new();
        if let Some(services) = &self.host_services {
            services.validate(&capability_registry, &mut seen_tool_names)?;
        }
        let host_capability = |id: &str| {
            self.host_services
                .as_ref()
                .and_then(|services| services.resolves(id))
                .map(str::to_string)
        };

        for input in self.capabilities {
            let parts = input.into_parts();
            let input_id = parts.reference.id().to_string();
            everruns_contracts::validate_capability_id(&input_id).map_err(|error| {
                BuildError::InvalidCapability {
                    id: input_id.clone(),
                    reason: error.reason(),
                }
            })?;
            everruns_contracts::validate_capability_config(
                &input_id,
                parts.reference.config_value(),
            )
            .map_err(|error| BuildError::InvalidCapability {
                id: input_id.clone(),
                reason: error.reason(),
            })?;

            let canonical_id = capability_registry
                .canonical_id(&input_id)
                .map(str::to_string)
                .or_else(|| host_capability(&input_id))
                .unwrap_or_else(|| input_id.clone());
            if activated_capabilities.contains(&canonical_id) {
                return Err(BuildError::DuplicateCapability { id: canonical_id });
            }

            #[cfg(feature = "capabilities")]
            if let Some(definition) = parts.definition {
                definition
                    .validate()
                    .map_err(|error| BuildError::InvalidCapability {
                        id: input_id.clone(),
                        reason: error.reason(),
                    })?;
                if capability_registry.get(definition.id()).is_some()
                    || host_capability(definition.id()).is_some()
                {
                    return Err(BuildError::DuplicateCapability {
                        id: definition.id().to_string(),
                    });
                }
                for tool in definition.tools() {
                    let spec = tool.spec();
                    validate_tool_name(spec.name()).map_err(|reason| {
                        BuildError::InvalidToolName {
                            name: spec.name().to_string(),
                            reason,
                        }
                    })?;
                    validate_tool_schema(spec.input_schema()).map_err(|reason| {
                        BuildError::InvalidToolSchema {
                            name: spec.name().to_string(),
                            reason,
                        }
                    })?;
                    if !seen_tool_names.insert(spec.name().to_string()) {
                        return Err(BuildError::DuplicateTool {
                            name: spec.name().to_string(),
                        });
                    }
                }
                capability_implementations.push(CapabilityImplementation::Definition(definition));
            }

            validate_registered_capability_config(
                &capability_registry,
                &canonical_id,
                parts.reference.config_value(),
            )?;
            if let Some(services) = &self.host_services {
                services.validate_reference(&canonical_id, parts.reference.config_value())?;
            }
            activated_capabilities
                .activate(canonical_id.clone())
                .map_err(|error| BuildError::DuplicateCapability {
                    id: error.id().to_string(),
                })?;
            capabilities.push(everruns_contracts::CapabilityRef::with_config(
                canonical_id,
                parts.reference.config_value().clone(),
            ));
        }

        // Host-activated capabilities follow the Agent's own references, in
        // registration order, so instruction order is deterministic.
        if let Some(services) = &self.host_services {
            for capability in &services.capabilities {
                let Some(config) = &capability.activation else {
                    continue;
                };
                let id = capability.implementation.id().to_string();
                activated_capabilities
                    .activate(id.clone())
                    .map_err(|error| BuildError::DuplicateCapability {
                        id: error.id().to_string(),
                    })?;
                capabilities.push(everruns_contracts::CapabilityRef::with_config(
                    id,
                    config.clone(),
                ));
            }
        }

        // Approval-gated function tools share one host gate whose policy names
        // exactly those tools. Fail closed: a gated tool without an approver
        // is a build error, never a silently ungated call.
        let approval_predicates = crate::tool::approval_predicates(&function_tools);
        for tool in &self.client_tools {
            validate_tool_name(&tool.name).map_err(|reason| BuildError::InvalidToolName {
                name: tool.name.clone(),
                reason,
            })?;
            validate_tool_schema(&tool.parameters).map_err(|reason| {
                BuildError::InvalidToolSchema {
                    name: tool.name.clone(),
                    reason,
                }
            })?;
            if !seen_tool_names.insert(tool.name.clone()) {
                return Err(BuildError::DuplicateTool {
                    name: tool.name.clone(),
                });
            }
        }
        if let Some((first_gated, _)) = approval_predicates.first() {
            #[cfg(feature = "builtins")]
            {
                let Some(approver) = self.approver.clone() else {
                    return Err(BuildError::MissingApprover {
                        tool: first_gated.clone(),
                    });
                };
                let id = everruns_core::builtins::TOOL_APPROVAL_CAPABILITY_ID.to_string();
                if capability_registry.get(&id).is_some()
                    || activated_capabilities.activate(id.clone()).is_err()
                {
                    return Err(BuildError::DuplicateCapability { id });
                }
                capabilities.push(everruns_contracts::CapabilityRef::new(id.as_str()));
                capability_implementations.push(CapabilityImplementation::Approval(
                    approval::approval_capability(
                        approver,
                        approval_predicates.into_iter().collect(),
                    ),
                ));
            }
            #[cfg(not(feature = "builtins"))]
            return Err(BuildError::MissingApprover {
                tool: first_gated.clone(),
            });
        }

        for function_tool in function_tools {
            let id = function_tool.name().to_string();
            everruns_contracts::validate_capability_id(&id).map_err(|error| {
                BuildError::InvalidCapability {
                    id: id.clone(),
                    reason: error.reason(),
                }
            })?;
            if capability_registry.get(&id).is_some()
                || host_capability(&id).is_some()
                || activated_capabilities.activate(id.clone()).is_err()
            {
                return Err(BuildError::DuplicateCapability { id });
            }
            capabilities.push(everruns_contracts::CapabilityRef::new(id.as_str()));
            capability_implementations.push(CapabilityImplementation::Function(function_tool));
        }
        #[cfg(feature = "builtins")]
        if let Some(ask_user) = self.ask_user {
            capability_implementations.push(CapabilityImplementation::AskUser(ask_user));
        }

        Ok(Agent {
            name,
            instructions,
            model,
            provider,
            capabilities,
            capability_implementations,
            initial_files: self.initial_files,
            network_access: self.network_access,
            package_manifest: self.package_manifest,
            max_iterations: self.max_iterations,
            parallel_tool_calls: self.parallel_tool_calls,
            communication: self.communication,
            workspace_root: self.workspace_root,
            default_workspace,
            workspace_policy: self.workspace_policy,
            mcp_servers,
            plugin_warnings: self.plugin_warnings,
            #[cfg(feature = "local")]
            local: self.local,
            lifecycle_hooks: self.lifecycle_hooks,
            client_tools: self.client_tools,
            client_questions: self.client_questions,
            host_services: self.host_services.map(Arc::new),
            backends: self.backends,
            state: Arc::new(AgentState::new(workspace_backends)),
        })
    }
}
