// Validate capability references on write paths.
//
// Ensures that capability refs persisted on agents, harnesses, and sessions
// actually resolve: built-in IDs must exist in the registry, `mcp:{uuid}`
// refs must point to an MCP server in the caller's org, and `skill:{uuid}`
// refs must point to a skill in the caller's org.
//
// See EVE-154 for context.

use crate::errors::ResourceNotFoundError;
use crate::storage::StorageBackend;
use anyhow::Result;
use everruns_core::capabilities::{
    AgentCapabilityConfig, CapabilityRegistry, is_mcp_capability, is_skill_capability,
    parse_mcp_capability_id, parse_skill_capability_id,
};

/// Validate that all capability references in `capabilities` resolve.
///
/// - Built-in IDs are checked against a static `CapabilityRegistry::with_builtins()`.
/// - `mcp:{uuid}` refs are resolved against the org's MCP servers.
/// - `skill:{uuid}` refs are resolved against the org's skills.
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
        } else {
            // Built-in capability — check registry
            let reg = registry.get_or_insert_with(CapabilityRegistry::with_builtins);
            if !reg.has(cap_id) {
                return Err(ResourceNotFoundError::new("Capability").into());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{CreateMcpServerRow, CreateSkillRow};
    use everruns_core::DEFAULT_ORG_ID;
    use std::sync::Arc;
    use uuid::Uuid;

    #[tokio::test]
    async fn valid_builtin_capability_passes() {
        let db = Arc::new(StorageBackend::in_memory());
        let caps = vec![AgentCapabilityConfig::new("current_time")];

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
