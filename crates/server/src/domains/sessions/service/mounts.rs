//! Assembling a session's mounts: capabilities, initial files, and memory scopes.

use super::*;

impl SessionService {
    /// Apply capability mounts to a session's filesystem.
    ///
    /// Collects mounts from harness + agent + session capabilities.
    pub(crate) async fn apply_capability_mounts(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: impl Into<uuid::Uuid> + Copy,
        scoped_memory: Option<ScopedMemoryContext>,
    ) -> Result<()> {
        let session_id = session_id.into();

        let mounts = self
            .collect_capability_mounts(
                org_id,
                harness_id,
                agent_id,
                session_capabilities,
                session_id,
                scoped_memory,
            )
            .await?;
        if mounts.is_empty() {
            return Ok(()); // No mounts to apply
        }

        // Apply mounts to session filesystem
        let result = self
            .session_file_service
            .apply_capability_mounts(session_id, &mounts)
            .await?;

        if !result.is_success() {
            tracing::warn!(
                session_id = %session_id,
                agent_id = ?agent_id,
                errors = ?result.errors,
                "Some capability mounts failed to apply"
            );
        } else {
            tracing::debug!(
                session_id = %session_id,
                agent_id = ?agent_id,
                files_created = result.files_created,
                directories_created = result.directories_created,
                mount_points = result.mount_points_applied,
                "Capability mounts applied successfully"
            );
        }

        Ok(())
    }

    pub(crate) async fn collect_capability_mounts(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
        session_id: Uuid,
        scoped_memory: Option<ScopedMemoryContext>,
    ) -> Result<Vec<MountPoint>> {
        let capability_configs = self
            .collect_session_capability_configs(org_id, harness_id, agent_id, session_capabilities)
            .await?;

        let ctx = SystemPromptContext::without_file_store(SessionId::from_uuid(session_id));
        let resolved_configs =
            resolve_capability_configs(&capability_configs, &self.capability_registry)?;
        let mut mounts =
            collect_capabilities_with_configs(&resolved_configs, &self.capability_registry, &ctx)
                .await
                .mounts;
        mounts.extend(
            self.collect_workspace_memory_mounts(org_id, &resolved_configs)
                .await?,
        );
        // `skill:{uuid}` refs have no registry entry, so dependency resolution
        // drops them from `resolved_configs` — resolve them from org data
        // against the raw config list instead (same pattern as declarative and
        // plugin refs, which carry their definition in the config payload).
        mounts.extend(
            self.collect_registry_skill_mounts(org_id, &capability_configs)
                .await?,
        );
        ensure_no_reserved_memory_mounts(&mounts)?;
        if let Some(scoped_memory) = scoped_memory {
            mounts.extend(
                self.collect_scoped_memory_mounts(org_id, scoped_memory)
                    .await?,
            );
        }
        Ok(mounts)
    }

    /// Copy harness/agent/session starter files into the session filesystem.
    pub(crate) async fn apply_initial_files(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_initial_files: &[InitialFile],
        session_id: Uuid,
    ) -> Result<()> {
        let files = self
            .collect_initial_files(org_id, harness_id, agent_id, session_initial_files)
            .await?;
        ensure_no_reserved_memory_initial_files(&files)?;

        for file in files {
            self.session_file_service
                .create_file(
                    session_id,
                    CreateFileInput {
                        path: normalize_initial_file_path(&file.path),
                        content: Some(file.content),
                        encoding: Some(file.encoding),
                        is_readonly: Some(file.is_readonly),
                    },
                )
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn collect_initial_files(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_initial_files: &[InitialFile],
    ) -> Result<Vec<InitialFile>> {
        let harness_files = self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .map(|harness| harness.initial_files)
            .unwrap_or_default();

        let agent_files = if let Some(agent_id) = agent_id
            && let Some(row) = self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
        {
            serde_json::from_value::<Vec<InitialFile>>(row.initial_files).unwrap_or_default()
        } else {
            vec![]
        };

        // Fold: harness → agent → session (same merge semantics as AgentConfigOverlay)
        let merged = merge_initial_files(&harness_files, &agent_files);
        let merged = merge_initial_files(&merged, session_initial_files);

        Ok(merged)
    }

    pub(crate) async fn collect_workspace_memory_mounts(
        &self,
        org_id: i64,
        capability_configs: &[AgentCapabilityConfig],
    ) -> Result<Vec<MountPoint>> {
        let Some(config) = capability_configs
            .iter()
            .find(|config| config.capability_id() == MEMORY_CAPABILITY_ID)
        else {
            return Ok(vec![]);
        };
        let memory_config: MemoryConfig = serde_json::from_value(config.config_value().clone())
            .map_err(|error| {
                BadRequestError::new(format!("Invalid workspace memory config: {error}"))
            })?;
        let mut mounts = Vec::with_capacity(memory_config.mounts.len());

        for mount in memory_config.mounts {
            let memory_id = MemoryId::parse(&mount.memory)
                .map_err(|_| BadRequestError::new("Invalid workspace memory ID"))?;
            let memory = self
                .db
                .get_memory(org_id, memory_id)
                .await?
                .filter(|memory| memory.status == "active")
                .ok_or_else(|| ResourceNotFoundError::new("Memory"))?;
            if memory.scope != "org" {
                return Err(BadRequestError::new(
                    "Scoped memories are server-managed and cannot be mounted explicitly",
                )
                .into());
            }

            if memory.is_readonly && mount.mode == MemoryMountAccess::ReadWrite {
                return Err(BadRequestError::new(format!(
                    "Memory {} is read-only and cannot be mounted readwrite",
                    mount.memory
                ))
                .into());
            }

            let access = if memory.is_readonly || mount.mode == MemoryMountAccess::ReadOnly {
                MountAccess::ReadOnly
            } else {
                MountAccess::ReadWrite
            };
            let files = self.db.list_all_memory_files(memory.id).await?;
            mounts.push(MountPoint::new(
                mount.path,
                access,
                MountSource::directory(memory_files_to_mount_entries(files)),
                MEMORY_CAPABILITY_ID,
            ));
        }

        Ok(mounts)
    }

    /// Mount registry skills referenced as `skill:{uuid}` capability refs.
    ///
    /// Each active skill is reconstructed into `/.agents/skills/{name}/`
    /// (SKILL.md + bundled text files) so the built-in `SkillsCapability`
    /// discovers it alongside workspace skills.
    ///
    /// A skill that is missing or not active is skipped with a warning rather
    /// than failing session creation: `validate_capability_refs` accepts a
    /// `skill:{uuid}` ref at any status, so archiving or disabling a skill must
    /// not take down every agent that references it. A session missing one
    /// skill is still runnable — unlike a missing memory mount, which is fatal.
    pub(crate) async fn collect_registry_skill_mounts(
        &self,
        org_id: i64,
        capability_configs: &[AgentCapabilityConfig],
    ) -> Result<Vec<MountPoint>> {
        let mut seen = HashSet::new();
        let mut mounts = Vec::new();
        for config in capability_configs {
            let cap_id = config.capability_id();
            if !is_skill_capability(cap_id) {
                continue;
            }
            let skill_uuid = parse_skill_capability_id(cap_id).ok_or_else(|| {
                BadRequestError::new(format!("Invalid skill capability reference: {cap_id}"))
            })?;
            if !seen.insert(skill_uuid) {
                continue;
            }
            let Some(row) = self.db.get_skill(org_id, skill_uuid).await? else {
                tracing::warn!(
                    skill_id = %skill_uuid,
                    "Referenced skill not found; skipping session mount"
                );
                continue;
            };
            if row.status != "active" {
                tracing::warn!(
                    skill_id = %skill_uuid,
                    status = %row.status,
                    "Referenced skill is not active; skipping session mount"
                );
                continue;
            }
            let skill = crate::domains::skills::queries::row_to_skill(&row);

            // Bundled files: text only. SKILL.md is reconstructed from the
            // stored fields, so drop any archived copy to keep it canonical.
            let files: Vec<(String, String)> = self
                .db
                .list_skill_files(skill_uuid)
                .await?
                .into_iter()
                .filter(|file| file.path != "SKILL.md")
                .filter_map(|file| {
                    if file.is_binary {
                        tracing::warn!(
                            skill = %skill.name,
                            path = %file.path,
                            "Skipping binary skill file in session mount"
                        );
                        return None;
                    }
                    file.content.map(|content| (file.path, content))
                })
                .collect();

            let capability = AttachSkillCapability::from_registry_with_options(
                skill_uuid,
                skill.name,
                skill.description,
                row.instructions.clone(),
                files,
                skill.user_invocable,
                skill.disable_model_invocation,
            );
            mounts.extend(everruns_core::capabilities::Capability::mounts(&capability));
        }
        Ok(mounts)
    }

    pub(crate) async fn collect_scoped_memory_mounts(
        &self,
        org_id: i64,
        context: ScopedMemoryContext,
    ) -> Result<Vec<MountPoint>> {
        let mut mounts = Vec::with_capacity(2);

        if let Some(agent_id) = context.agent_id {
            let memory = self
                .get_or_create_scoped_memory(
                    org_id,
                    "agent",
                    Some(agent_id),
                    None,
                    format!("agent-memory-{}", agent_id.uuid().simple()),
                    "Server-managed per-agent memory.",
                )
                .await?;
            mounts.push(
                self.memory_row_to_mount(memory, AGENT_MEMORY_MOUNT_PATH)
                    .await?,
            );
        }

        if let Some(user_id) = context.user_id {
            let memory = self
                .get_or_create_scoped_memory(
                    org_id,
                    "user",
                    None,
                    Some(user_id),
                    format!("user-memory-{}", user_id.simple()),
                    "Server-managed per-user memory.",
                )
                .await?;
            mounts.push(
                self.memory_row_to_mount(memory, USER_MEMORY_MOUNT_PATH)
                    .await?,
            );
        }

        Ok(mounts)
    }

    pub(crate) async fn get_or_create_scoped_memory(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<AgentId>,
        owner_user_id: Option<Uuid>,
        name: String,
        description: &str,
    ) -> Result<MemoryRow> {
        if let Some(memory) = self
            .db
            .get_memory_by_scope_owner(org_id, scope, owner_agent_id, owner_user_id)
            .await?
            .filter(|memory| memory.status == "active")
        {
            return Ok(memory);
        }

        self.db
            .create_memory(
                org_id,
                CreateMemoryRow {
                    public_id: MemoryId::new().to_string(),
                    name,
                    description: Some(description.to_string()),
                    scope: scope.to_string(),
                    owner_agent_id,
                    owner_user_id,
                    source_type: "manual".to_string(),
                    source_config: serde_json::json!({}),
                    is_readonly: false,
                    sync_status: "idle".to_string(),
                    owner_principal_id: None,
                    resolved_owner_user_id: owner_user_id,
                },
            )
            .await
    }

    pub(crate) async fn memory_row_to_mount(
        &self,
        memory: MemoryRow,
        mount_path: &str,
    ) -> Result<MountPoint> {
        let files = self.db.list_all_memory_files(memory.id).await?;
        Ok(MountPoint::new(
            mount_path,
            MountAccess::ReadWrite,
            MountSource::directory(memory_files_to_mount_entries(files)),
            MEMORY_CAPABILITY_ID,
        ))
    }
}
