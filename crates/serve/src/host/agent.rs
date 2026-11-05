//! Ordinary Agent registration, tool composition and members.
use super::*;

impl Host {
    // --- Agent assembly -------------------------------------------------------

    /// Turn a discovered agent into an `everruns::Agent`. Its approvals and
    /// questions are parked under `surface` when given, else under the
    /// session that asks.
    pub(crate) fn build_agent(
        &self,
        entry: &AgentEntry,
        surface: Option<String>,
        persistent: bool,
    ) -> crate::Result<everruns::Agent> {
        let local = self
            .data_dir
            .as_ref()
            .filter(|_| persistent && !self.engine.has_session_hosting())
            .map(|dir| LocalConfig::new(dir.join("everruns")));
        self.build_agent_with(entry, surface, local, None)
    }

    /// [`Host::build_agent`] over an explicit legacy `local` store, or with
    /// the shared directory `workspace` sessions of that store used. A cutover
    /// reads the first and runs moved sessions on the second.
    pub(crate) fn build_agent_with(
        &self,
        entry: &AgentEntry,
        surface: Option<String>,
        local: Option<LocalConfig>,
        workspace: Option<PathBuf>,
    ) -> crate::Result<everruns::Agent> {
        let spec = &entry.spec;
        let (model, _) = gateway::resolve(
            &spec.model,
            spec.offline.as_ref(),
            self.mode.allow_offline(),
            &self.gateway,
        )?;
        // Definitions and their build digest use one frozen asset snapshot.
        // The development watcher replaces the host when files change.
        let instructions = self
            .instructions
            .get(&entry.name)
            .cloned()
            .ok_or_else(|| anyhow!("agent instructions were not frozen"))?;
        let gate = Gate {
            host: self.me.clone(),
            surface,
        };
        let mut builder = everruns::Agent::builder()
            .name(&entry.name)
            .description(agent_description(entry))
            .model(model)
            .instructions(instructions);
        if self.engine.has_session_hosting() {
            builder = builder.durable_approvals().client_questions();
        } else {
            builder = builder.approver(gate.clone()).ask_user(gate);
        }
        // Skills use the built-in capability: each embedded skill is seeded
        // read-only where it looks (`.agents/skills/<name>/SKILL.md`), and the
        // agent discovers and activates them with its own tools.
        let skills = &self.app.inner.skills;
        if !skills.is_empty() {
            builder = builder.capability(everruns::Skills);
            for skill in skills {
                let prefix = format!("skills/{}/", skill.name);
                for asset in &self.app.inner.assets {
                    if let Some(relative) = asset.path.strip_prefix(&prefix) {
                        builder = builder.readonly_file(
                            format!(".agents/skills/{}/{relative}", skill.name),
                            self.assets
                                .get(asset.path)
                                .cloned()
                                .unwrap_or_else(|| asset.contents.to_string()),
                        );
                    }
                }
            }
        }
        for tool in self.app.tools_for(entry) {
            builder = builder.tool(self.function_tool(tool, entry.name.clone()));
        }
        // Members are hand-off targets of the Framework's Agents capability
        // (`spawn_agent`): each runs as its own session in this agent's run,
        // on this agent's host. Discovery rejects unknown names, self
        // membership and members that lead others.
        for name in &spec.members {
            let member = self
                .app
                .agent(name)
                .ok_or_else(|| anyhow!("agent `{}`: unknown member `{name}`", entry.name))?;
            builder = builder.member(self.build_agent_with(member, None, None, None)?);
        }
        for connection in &self.app.inner.connections {
            let Some(mcp) = &connection.value.mcp else {
                continue;
            };
            match mcp.to_everruns(connection.value.name) {
                Ok(server) => builder = builder.mcp_server(server),
                Err(err) if self.mode == Mode::Start => return Err(err),
                // Dev and evals run without the connection rather than not at all.
                Err(_) => {}
            }
        }
        if let Some(images) = &self.app.inner.config.images {
            // `[images]` opts the app into saving images: the default
            // read-only workspace would refuse the files otherwise.
            builder = builder
                .capability(images.capability())
                .allow_workspace_write(crate::config::ImagesConfig::OUTPUT_DIR)?;
        }
        builder = match self.app.inner.config.sandbox.kind {
            SandboxKind::None => builder,
            SandboxKind::Local => builder.capability(everruns::FileSystem),
            SandboxKind::Bashkit => builder.capability(BashkitShell::new()),
            // A hosting target supplies the microVM adapter; without one
            // (dev, self-hosted start) bashkit stands in.
            SandboxKind::Microvm => match self.microvm.get() {
                Some(microvm) => microvm(builder),
                None => builder.capability(BashkitShell::new()),
            },
        };
        if let Some(local) = local {
            #[allow(deprecated)]
            // Legacy facade lane: only the source of a pre-authority cutover
            // (EVE-1298) or the store of a host without session hosting.
            {
                builder = builder.local(local);
            }
        }
        if let Some(workspace) = workspace {
            builder = builder.workspace(workspace);
        }
        if let Some(customize) = &spec.customize {
            builder = customize(builder);
        }
        if let Some(package) = &spec.package {
            let package = package.clone().bind_mcp(|key| std::env::var(key).ok())?;
            builder = package.apply_to(builder)?;
        }
        builder
            .build()
            .map_err(|err| anyhow!("agent `{}`: {err}", entry.name))
    }

    /// A `#[tool]` as an everruns tool: each call gets a `Cx` over its
    /// `ToolCallContext`, and its approval rule becomes the runtime's gate.
    fn function_tool(&self, tool: &'static ToolRegistration, agent: String) -> FunctionTool {
        let host = self.me.clone();
        let function = FunctionTool::with_context(
            tool.name,
            tool.description,
            (tool.schema)(),
            move |call: ToolCallContext, args: Value| {
                (tool.call)(Cx::tool(host.clone(), agent.clone(), call), args)
            },
        );
        match tool.approval {
            Approval::Never => function,
            Approval::Always => function.always_needs_approval(),
            Approval::When(predicate) => function.needs_approval(predicate),
        }
    }
}

/// What an agent does: its description, else its doc comment. A lead sees it
/// next to the member's name as when to hand work to it.
fn agent_description(member: &AgentEntry) -> String {
    member
        .spec
        .description
        .clone()
        .or_else(|| (!member.doc.is_empty()).then(|| member.doc.to_string()))
        .unwrap_or_else(|| format!("The {} agent.", member.name))
}
