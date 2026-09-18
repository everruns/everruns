//! Resolving and authorizing the capabilities a session may use.

use super::*;

impl SessionService {
    /// Collect all capability IDs for a session (harness + agent + session-level).
    /// Deduplicates while preserving order: harness first, then agent, then session.
    pub(crate) async fn collect_session_capability_ids(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<Vec<String>> {
        let mut capability_ids = Vec::new();

        capability_ids.extend(
            self.resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
                .await?
                .map(|harness| {
                    harness
                        .capabilities
                        .into_iter()
                        .map(|cap| cap.capability_id().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );

        if let Some(agent_id) = agent_id
            && self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
                .is_some()
        {
            let agent_cap_rows = self.db.get_agent_capabilities(agent_id).await?;
            for r in &agent_cap_rows {
                if !capability_ids.contains(&r.capability_id) {
                    capability_ids.push(r.capability_id.clone());
                }
            }
        }

        for cap in session_capabilities {
            let cap_id = cap.capability_id().to_string();
            if !capability_ids.contains(&cap_id) {
                capability_ids.push(cap_id);
            }
        }

        Ok(capability_ids)
    }

    /// Collect merged capability configs for a session, preserving per-layer config.
    pub(crate) async fn collect_session_capability_configs(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<Vec<AgentCapabilityConfig>> {
        let mut capability_configs = self
            .resolve_effective_harness(org_id, HarnessId::from_uuid(harness_id))
            .await?
            .map(|harness| harness.capabilities)
            .unwrap_or_default();

        if let Some(agent_id) = agent_id
            && self
                .db
                .get_agent(org_id, AgentId::from_uuid(agent_id))
                .await?
                .is_some()
        {
            let agent_cap_rows = self.db.get_agent_capabilities(agent_id).await?;
            let agent_capabilities = agent_cap_rows
                .into_iter()
                .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
                .collect::<Vec<_>>();
            capability_configs = merge_capabilities(&capability_configs, &agent_capabilities);
        }

        let capability_configs = merge_capabilities(&capability_configs, session_capabilities);
        crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            self.db.as_ref(),
            org_id,
            capability_configs,
        )
        .await
    }

    /// EVE-709: reject session creation when a required built-in capability is not
    /// available in this deployment.
    ///
    /// The effective capability set (harness chain + agent + session) may name
    /// built-in capabilities that are feature-gated (e.g. `container_sandbox`
    /// behind `FEATURE_CONTAINER_SANDBOX`). When such a capability is disabled it
    /// is absent from the registry, its tools never register, and the session
    /// silently runs without them — degrading into a different execution
    /// environment. Rather than degrade silently, fail with a clear error naming
    /// the unavailable capabilities.
    ///
    /// Only plain built-in references are checked. Namespaced refs
    /// (`declarative:`, `plugin:`, `skill:`, `mcp:`) resolve from org data rather
    /// than the registry, so their absence from the registry is expected and is
    /// validated separately by `validate_capability_refs`.
    pub(crate) async fn require_available_capabilities(
        &self,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<()> {
        let capability_ids = self
            .collect_session_capability_ids(org_id, harness_id, agent_id, session_capabilities)
            .await?;

        let mut missing: Vec<String> = capability_ids
            .into_iter()
            .filter(|id| {
                !is_declarative_capability(id)
                    && !is_plugin_capability(id)
                    && !is_skill_capability(id)
                    && !is_mcp_capability(id)
                    && !self.capability_registry.has(id)
            })
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        missing.sort();
        missing.dedup();
        Err(BadRequestError::new(format!(
            "Harness requires capabilities unavailable in this deployment: {}",
            missing.join(", ")
        ))
        .into())
    }

    pub(crate) async fn require_admin_for_high_risk_session_capabilities(
        &self,
        caller: &Caller,
        org_id: i64,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_capabilities: &[AgentCapabilityConfig],
    ) -> Result<()> {
        if caller.role.has_permission(OrgRole::Admin) {
            return Ok(());
        }

        let mut capability_ids = self
            .collect_session_capability_ids(org_id, harness_id, agent_id, session_capabilities)
            .await?;
        // Expand declarative capability dependencies so hidden high-risk built-ins
        // (e.g. `web_fetch`, `bashkit_shell`) cannot bypass the admin gate. Validation
        // forbids nested declarative deps, so a single expansion pass is sufficient.
        let declarative_refs: Vec<String> = capability_ids
            .iter()
            .filter(|id| is_declarative_capability(id))
            .cloned()
            .collect();
        for cap_ref in &declarative_refs {
            let Some(name) = parse_declarative_capability_id(cap_ref) else {
                continue;
            };
            let Some(row) = self
                .db
                .get_declarative_capability_by_name(org_id, name)
                .await?
            else {
                continue;
            };
            // Fail closed: a broken declarative definition must NOT silently skip
            // dependency expansion, since this gates admin-only high-risk session
            // capability assignment.
            let definition: DeclarativeCapabilityDefinition =
                serde_json::from_value(row.definition).map_err(|error| {
                    anyhow::anyhow!(
                        "declarative capability {cap_ref} has malformed definition: {error}"
                    )
                })?;
            for dep in definition.dependencies {
                if !capability_ids.contains(&dep) {
                    capability_ids.push(dep);
                }
            }
        }
        let high_risk: Vec<String> = capability_ids
            .into_iter()
            .filter(|capability_id| {
                self.capability_registry
                    .get(capability_id)
                    .is_some_and(|capability| capability.risk_level() == RiskLevel::High)
            })
            .collect();
        if high_risk.is_empty() {
            return Ok(());
        }

        // EVE-437 / TM-AUTHZ: this is an authorization failure, not an
        // internal error. Returning an `anyhow::bail!` here used to map to
        // 500 because `classify_anyhow` did not recognize the message
        // substring. A `PolicyError` maps to `Forbidden` (403) at the
        // command boundary.
        Err(everruns_core::PolicyError::denied(
            "session_high_risk_capabilities",
            &format!(
                "Admin role required to create sessions with high-risk capabilities: {}",
                high_risk.join(", ")
            ),
        )
        .into())
    }
}
