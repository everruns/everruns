// Validate capability references on write paths.
//
// Ensures that capability refs persisted on agents, harnesses, and sessions
// actually resolve: built-in IDs must exist in the registry, `mcp:{uuid}`
// refs must point to an MCP server in the caller's org, and `skill:{uuid}`
// refs must point to a skill in the caller's org. Server-managed plugin refs
// resolve by stable installation public ID.
//
// See EVE-154 for context.

use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::storage::StorageBackend;
use anyhow::Result;
use everruns_core::capabilities::{
    AgentCapabilityConfig, CapabilityRegistry, declarative_capability_id,
    is_declarative_capability, is_mcp_capability, is_skill_capability,
    parse_declarative_capability_id, parse_mcp_capability_id, parse_skill_capability_id,
};
use everruns_core::{is_plugin_capability, parse_plugin_capability_id};

use crate::domains::common::CommandError;

pub fn validate_feature_gated_capability_refs(
    feature_flags: &everruns_platform::FeatureFlags,
    capabilities: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    if let Some(flag) = capabilities.iter().find_map(|capability| {
        everruns_platform::FeatureFlags::required_for_capability(capability.capability_id())
            .filter(|flag| !feature_flags.is_enabled(flag))
    }) {
        return Err(CommandError::feature_not_enabled(flag));
    }
    Ok(())
}

/// Validate that all capability references in `capabilities` resolve.
///
/// - Built-in IDs are checked against a static `everruns_platform::capabilities::hosted_capability_registry()`.
/// - `mcp:{uuid}` refs are resolved against the org's MCP servers.
/// - `skill:{uuid}` refs are resolved against the org's skills.
/// - `plugin:{install_id}` refs resolve only to that active org installation.
///
/// Returns `Ok(())` on success, or a typed error identifying the first invalid ref.
pub async fn validate_capability_refs(
    db: &StorageBackend,
    org_id: i64,
    capabilities: &[AgentCapabilityConfig],
) -> Result<()> {
    // Lazily build registry only when needed (when there are non-virtual refs)
    let mut registry: Option<CapabilityRegistry> = None;

    for cap in capabilities {
        let cap_id = cap.capability_id();

        // Neutral contract checks first (EVE-873): the same open-ID grammar
        // and JSON-object config boundary the Framework enforces at agent
        // build time, so invalid IDs and non-object configs fail identically
        // on product write paths.
        if let Err(error) = everruns_capability::validate_capability_id(cap_id) {
            return Err(BadRequestError::new(format!(
                "Invalid capability reference '{cap_id}': {}",
                error.reason()
            ))
            .into());
        }
        if let Err(error) =
            everruns_capability::validate_capability_config(cap_id, cap.config_value())
        {
            return Err(BadRequestError::new(format!(
                "Invalid capability config for '{cap_id}': {}",
                error.reason()
            ))
            .into());
        }

        if is_mcp_capability(cap_id) {
            let uuid = parse_mcp_capability_id(cap_id)
                .ok_or_else(|| anyhow::anyhow!("Invalid MCP capability reference: {cap_id}"))?;
            db.get_mcp_server(org_id, uuid)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("MCP server"))?;
        } else if is_skill_capability(cap_id) {
            let uuid = parse_skill_capability_id(cap_id)
                .ok_or_else(|| anyhow::anyhow!("Invalid skill capability reference: {cap_id}"))?;
            db.get_skill(org_id, uuid)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Skill"))?;
        } else if is_declarative_capability(cap_id) {
            let name = parse_declarative_capability_id(cap_id).ok_or_else(|| {
                anyhow::anyhow!("Invalid declarative capability reference: {cap_id}")
            })?;
            let row = db
                .get_declarative_capability_by_name(org_id, name)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Declarative capability"))?;
            if !matches!(row.status.as_str(), "active" | "disabled") {
                return Err(ResourceNotFoundError::new("Declarative capability").into());
            }
        } else if is_plugin_capability(cap_id) {
            let public_id = parse_plugin_capability_id(cap_id).ok_or_else(|| {
                BadRequestError::new(format!("Invalid plugin capability reference: {cap_id}"))
            })?;
            let row = db
                .get_plugin_install_by_public_id(org_id, public_id)
                .await?;
            if row.is_none_or(|row| row.status != "active") {
                return Err(BadRequestError::new(format!(
                    "Plugin capability '{cap_id}' is unavailable. Remove it or reinstall the plugin and select the new capability."
                ))
                .into());
            }
        } else {
            // Built-in capability — check registry
            let reg = registry
                .get_or_insert_with(everruns_platform::capabilities::hosted_capability_registry);
            let Some(capability) = reg.get(cap_id) else {
                return Err(ResourceNotFoundError::new("Capability").into());
            };
            capability
                .validate_config(cap.config_value())
                .map_err(|message| {
                    BadRequestError::new(format!("Invalid capability config: {message}"))
                })?;
        }
    }

    Ok(())
}

/// Normalize user-entered capability refs before persistence.
///
/// Built-ins, MCP, skill, and already-prefixed declarative refs are left as-is.
/// If a plain ref is not a built-in but matches an active/disabled declarative
/// capability name in the caller's org, it is stored canonically as
/// `declarative:<name>`.
pub async fn normalize_capability_refs(
    db: &StorageBackend,
    org_id: i64,
    capabilities: Vec<AgentCapabilityConfig>,
) -> Result<Vec<AgentCapabilityConfig>> {
    let registry = everruns_platform::capabilities::hosted_capability_registry();
    let mut normalized = Vec::with_capacity(capabilities.len());

    for cap in capabilities {
        let cap_id = cap.capability_id();
        if is_mcp_capability(cap_id)
            || is_skill_capability(cap_id)
            || is_declarative_capability(cap_id)
            || is_plugin_capability(cap_id)
            || registry.get(cap_id).is_some()
        {
            normalized.push(cap);
            continue;
        }

        if let Some(row) = db
            .get_declarative_capability_by_name(org_id, cap_id)
            .await?
            && matches!(row.status.as_str(), "active" | "disabled")
        {
            normalized.push(AgentCapabilityConfig::with_config(
                declarative_capability_id(&row.name),
                cap.config_value().clone(),
            ));
            continue;
        }

        normalized.push(cap);
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{
        CreateDeclarativeCapabilityRow, CreateMcpServerRow, CreatePluginInstallRow, CreateSkillRow,
    };
    use everruns_core::{DEFAULT_ORG_ID, PluginInstallId, plugin_capability_id};
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn feature_gated_capability_requires_effective_flag() {
        let capabilities = vec![AgentCapabilityConfig::new("agent_handoff")];
        let error = validate_feature_gated_capability_refs(
            &everruns_platform::FeatureFlags::default(),
            &capabilities,
        )
        .unwrap_err();
        assert_eq!(error.message(), "Feature 'agent_delegation' is not enabled");

        let enabled = everruns_platform::FeatureFlags {
            agent_delegation: true,
            ..everruns_platform::FeatureFlags::default()
        };
        validate_feature_gated_capability_refs(&enabled, &capabilities).unwrap();
    }

    #[tokio::test]
    async fn invalid_capability_ids_fail_like_the_framework() {
        // One rule set (EVE-873): every ID the Framework's AgentBuilder
        // rejects must be rejected by the server write path with the same
        // neutral reason.
        let db = Arc::new(StorageBackend::in_memory());
        for id in [
            "",
            "2fast",
            "has space",
            "vendor/custom",
            "__everruns_private",
        ] {
            let framework_reason = everruns_capability::validate_capability_id(id)
                .unwrap_err()
                .reason();
            let err =
                validate_capability_refs(&db, DEFAULT_ORG_ID, &[AgentCapabilityConfig::new(id)])
                    .await
                    .unwrap_err();
            assert!(err.downcast_ref::<BadRequestError>().is_some(), "{id:?}");
            assert!(
                err.to_string().contains(&framework_reason),
                "{id:?}: server error {err} should carry framework reason {framework_reason:?}"
            );
        }
    }

    #[tokio::test]
    async fn non_object_capability_config_fails_like_the_framework() {
        let db = Arc::new(StorageBackend::in_memory());
        for config in [
            serde_json::json!("string"),
            serde_json::json!(42),
            serde_json::json!([1, 2]),
        ] {
            let err = validate_capability_refs(
                &db,
                DEFAULT_ORG_ID,
                &[AgentCapabilityConfig::with_config("current_time", config)],
            )
            .await
            .unwrap_err();
            assert!(err.downcast_ref::<BadRequestError>().is_some());
            assert!(
                err.to_string()
                    .contains("capability config must be a JSON object"),
                "unexpected error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn valid_builtin_capability_passes() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::new("current_time")];

        validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn plain_declarative_name_normalizes_to_canonical_ref() {
        let db = Arc::new(StorageBackend::in_memory());
        db.create_declarative_capability(
            DEFAULT_ORG_ID,
            CreateDeclarativeCapabilityRow {
                public_id: everruns_core::DeclarativeCapabilityId::new().to_string(),
                name: "research_pack".to_string(),
                display_name: Some("Research Pack".to_string()),
                description: "Research defaults".to_string(),
                definition: serde_json::json!({
                    "name": "research_pack",
                    "display_name": "Research Pack",
                    "description": "Research defaults"
                }),
            },
        )
        .await
        .unwrap();

        let caps = normalize_capability_refs(
            &db,
            DEFAULT_ORG_ID,
            vec![AgentCapabilityConfig::new("research_pack")],
        )
        .await
        .unwrap();

        assert_eq!(caps[0].capability_id(), "declarative:research_pack");
        validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unknown_builtin_capability_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::new("nonexistent_capability")];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Capability not found");
    }

    #[tokio::test]
    async fn invalid_builtin_capability_config_is_bad_request() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::with_config(
            "gpt_image_gen",
            serde_json::json!({ "partial_images": 4 }),
        )];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert!(err.downcast_ref::<BadRequestError>().is_some());
        assert!(err.to_string().contains("Invalid capability config"));
    }

    async fn create_plugin(db: &StorageBackend, name: &str) -> String {
        let public_id = PluginInstallId::new().to_string();
        db.create_plugin_install(
            DEFAULT_ORG_ID,
            CreatePluginInstallRow {
                public_id: public_id.clone(),
                name: name.to_string(),
                marketplace_id: None,
                source: serde_json::json!({}),
                version: Some("1.0.0".to_string()),
                pinned_sha: None,
                manifest: serde_json::json!({"name": name}),
                definition: serde_json::json!({
                    "name": name,
                    "description": "Test plugin"
                }),
                warnings: serde_json::json!([]),
            },
        )
        .await
        .unwrap();
        plugin_capability_id(&public_id)
    }

    #[tokio::test]
    async fn active_plugin_install_ref_passes() {
        let db = Arc::new(StorageBackend::in_memory());
        let capability_ref = create_plugin(&db, "resend").await;

        validate_capability_refs(
            &db,
            DEFAULT_ORG_ID,
            &[AgentCapabilityConfig::new(capability_ref)],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn plugin_name_is_not_a_capability_identity() {
        let db = Arc::new(StorageBackend::in_memory());
        create_plugin(&db, "resend").await;

        let err = validate_capability_refs(
            &db,
            DEFAULT_ORG_ID,
            &[AgentCapabilityConfig::new("plugin:resend")],
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("unavailable"));
    }

    #[tokio::test]
    async fn disabled_or_uninstalled_plugin_ref_is_actionable() {
        let db = Arc::new(StorageBackend::in_memory());
        let capability_ref = create_plugin(&db, "resend").await;
        let public_id = parse_plugin_capability_id(&capability_ref).unwrap();
        let plugin = db
            .get_plugin_install_by_public_id(DEFAULT_ORG_ID, public_id)
            .await
            .unwrap()
            .unwrap();
        db.delete_plugin_install(DEFAULT_ORG_ID, plugin.id)
            .await
            .unwrap();

        let err = validate_capability_refs(
            &db,
            DEFAULT_ORG_ID,
            &[AgentCapabilityConfig::new(capability_ref.as_str())],
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains(&capability_ref));
        assert!(err.to_string().contains("reinstall"));
    }

    #[tokio::test]
    async fn valid_mcp_ref_passes() {
        let db = Arc::new(StorageBackend::in_memory());
        let server = db
            .create_mcp_server(
                DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: "test-server".to_string(),
                    description: None,
                    url: "http://localhost:3000".to_string(),
                    transport_type: "sse".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        let cap_id = format!("mcp:{}", server.id.uuid());
        let caps = vec![AgentCapabilityConfig::new(cap_id)];

        validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn nonexistent_mcp_ref_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let cap_id = format!("mcp:{}", Uuid::new_v4());
        let caps = vec![AgentCapabilityConfig::new(cap_id)];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "MCP server not found");
    }

    #[tokio::test]
    async fn invalid_mcp_uuid_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::new("mcp:not-a-uuid")];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("Invalid MCP capability reference"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn valid_skill_ref_passes() {
        let db = Arc::new(StorageBackend::in_memory());
        let skill = db
            .create_skill(
                DEFAULT_ORG_ID,
                CreateSkillRow {
                    public_id: format!("skill_{}", Uuid::new_v4().simple()),
                    name: "test-skill".to_string(),
                    description: "A test skill".to_string(),
                    license: None,
                    compatibility: None,
                    metadata: serde_json::json!({}),
                    allowed_tools: None,
                    instructions: "Test instructions".to_string(),
                    source_type: "registry".to_string(),
                    archive_data: None,
                    version: "1.0.0".to_string(),
                },
            )
            .await
            .unwrap();
        let cap_id = format!("skill:{}", skill.id.uuid());
        let caps = vec![AgentCapabilityConfig::new(cap_id)];

        validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn nonexistent_skill_ref_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let cap_id = format!("skill:{}", Uuid::new_v4());
        let caps = vec![AgentCapabilityConfig::new(cap_id)];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Skill not found");
    }

    #[tokio::test]
    async fn invalid_skill_uuid_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::new("skill:not-a-uuid")];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("Invalid skill capability reference"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn empty_capabilities_passes() {
        let db = Arc::new(StorageBackend::in_memory());

        validate_capability_refs(&db, DEFAULT_ORG_ID, &[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mcp_ref_from_other_org_rejected() {
        let db = Arc::new(StorageBackend::in_memory());
        let other_org_id = db
            .create_organization_with_id(
                2,
                crate::storage::CreateOrganizationRow {
                    public_id: "org_2".to_string(),
                    name: "Org 2".to_string(),
                    created_by: None,
                },
            )
            .await
            .unwrap()
            .unwrap()
            .org_id;

        let server = db
            .create_mcp_server(
                other_org_id,
                CreateMcpServerRow {
                    name: "other-server".to_string(),
                    description: None,
                    url: "http://localhost:3000".to_string(),
                    transport_type: "sse".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        let cap_id = format!("mcp:{}", server.id.uuid());
        let caps = vec![AgentCapabilityConfig::new(cap_id)];

        let err = validate_capability_refs(&db, DEFAULT_ORG_ID, &caps)
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "MCP server not found");
    }
}
