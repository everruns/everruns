//! Ordinary Agent registration, tool composition and child session calls.
use super::*;

impl Host {
    // --- Agent assembly -------------------------------------------------------

    /// Turn a discovered agent into an `everruns::Agent`. Its approvals and
    /// questions are parked under `surface` when given (a subagent's show on
    /// its parent session), else under the session that asks.
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
        if !entry.sub {
            for sub in self.app.inner.agents.iter().filter(|agent| agent.sub) {
                builder = builder.tool(self.subagent_tool(sub));
            }
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
            // Legacy facade lane until EVE-1298 moves this onto Engine hosting.
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

    /// `ask_<name>`: run a subagent to completion on a task and return its
    /// answer. The child session's tool activity is reported as progress on
    /// the parent call, and its approvals and questions surface on the parent
    /// session.
    fn subagent_tool(&self, sub: &AgentEntry) -> FunctionTool {
        let description = sub
            .spec
            .description
            .clone()
            .or_else(|| (!sub.doc.is_empty()).then(|| sub.doc.to_string()))
            .unwrap_or_else(|| format!("Delegate a task to the {} subagent.", sub.name));
        let me = self.me.clone();
        let sub = sub.clone();
        let tool = FunctionTool::with_context(
            format!("ask_{}", sub.name),
            description,
            json!({
                "type": "object",
                "properties": { "task": { "type": "string", "description": "What the subagent should do, with all context it needs." } },
                "required": ["task"],
            }),
            move |call: ToolCallContext, args: Value| {
                let (me, sub) = (me.clone(), sub.clone());
                async move {
                    let Some(host) = me.upgrade() else {
                        return Ok::<_, String>(ToolResponse::error("host is shutting down"));
                    };
                    let task = args
                        .get("task")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let parent = call.session_id().to_string();
                    let agent = match host.build_agent(&sub, Some(parent.clone()), true) {
                        Ok(agent) => agent,
                        Err(err) => return Ok(ToolResponse::error(format!("{err:#}"))),
                    };
                    // A child session on the host's own engine, persisted in
                    // the same store as its parent.
                    let child = if host.engine.has_session_hosting() {
                        let identity = format!(
                            "serve-child:{}:{}:{}",
                            call.turn_id().unwrap_or_default(),
                            call.tool_call_id(),
                            sub.name
                        );
                        let child_id = SessionId::from_uuid(uuid::Uuid::new_v5(
                            &call.session_id().uuid(),
                            identity.as_bytes(),
                        ));
                        match host
                            .engine
                            .create_registered_with_metadata(
                                child_id,
                                &everruns::hosting::AgentBuild::new(&sub.name, &host.build_id),
                                json!({"parent_session_id":parent}),
                            )
                            .await
                        {
                            Ok(child) => {
                                let _ = host.store.insert_session(&SessionRow {
                                    id: child.id(),
                                    agent: sub.name.clone(),
                                    build_id: host.build_id.clone(),
                                    title: None,
                                    tags: Vec::new(),
                                    hints: None,
                                    metadata: Some(json!({"parent_session_id":parent})),
                                    created_at: now(),
                                    updated_at: now(),
                                });
                                host.go_live(&child.id(), child.clone());
                                child
                            }
                            Err(error) => return Ok(ToolResponse::error(error.to_string())),
                        }
                    } else {
                        match &sub.spec.package {
                            Some(package) => match package.create(&host.engine, agent) {
                                Ok(child) => child,
                                Err(err) => return Ok(ToolResponse::error(err.to_string())),
                            },
                            None => host.engine.create(agent),
                        }
                    };
                    let hosted = host.engine.has_session_hosting();
                    drop(host);
                    let mut events = child.events();
                    let run = async {
                        if hosted {
                            let message = everruns::MessageId::from_uuid(uuid::Uuid::new_v5(
                                &child.session_id().uuid(),
                                b"serve-child-input",
                            ));
                            child
                                .send_with_id(message, task.as_str())
                                .await?
                                .wait()
                                .await
                        } else {
                            child.send_and_wait(task.as_str()).await
                        }
                    };
                    tokio::pin!(run);
                    let turn = loop {
                        tokio::select! {
                            turn = &mut run => break turn,
                            event = events.recv() => {
                                if let Ok(Some(event)) = event
                                    && let Some(line) = child_progress(&sub.name, &event)
                                {
                                    call.progress(line).await;
                                }
                            }
                        }
                    };
                    Ok(match turn {
                        Ok(turn) if turn.success => ToolResponse::text(turn.response),
                        Ok(turn) => ToolResponse::error(turn.error.unwrap_or(turn.response)),
                        Err(err) => ToolResponse::error(err.to_string()),
                    })
                }
            },
        );
        // A recovered call rejoins its deterministic child/admission instead
        // of creating duplicate child work after an uncertain tool outcome.
        if self.engine.has_session_hosting() {
            tool.idempotent()
        } else {
            tool
        }
    }
}
