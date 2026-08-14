// Cross-org resource resolution.
//
// Every top-level entity that has a dedicated UI detail route MUST register a
// resolver here so the UI can auto-switch the caller's active org when they
// follow a direct link to a resource owned by a different (but still member)
// org. The API keeps returning 404 for cross-org access — only the UI calls
// the gated /v1/resolve-org endpoint to recover. See
// knowledge/security/multitenancy.md (Cross-Org Resource Resolution).
//
// Adding a new top entity: implement a storage method
// `get_<entity>_organization_id(public_id: &str) -> Result<Option<i64>>` and
// add an `inventory::submit!` block below. The HTTP endpoint dispatches by
// the prefix component of the ID (`session_...`, `agent_...`, etc.) so no
// other wiring is required.

use std::future::Future;
use std::pin::Pin;

use crate::kernel_imports::{
    CapabilityId,
    everruns_provider::typed_id::{McpServerId, SessionId, SkillId},
};
use anyhow::Result;
use everruns_core::capabilities::SkillCapabilityIdExt;
use everruns_mcp::McpCapabilityIdExt;
use uuid::Uuid;

use crate::storage::StorageBackend;

/// Resolver function signature: given a storage handle and a prefixed
/// public ID, return the owning `org_id` without any caller-side scoping.
pub type ResolveFn = for<'a> fn(
    &'a StorageBackend,
    &'a str,
) -> Pin<Box<dyn Future<Output = Result<Option<i64>>> + Send + 'a>>;

/// Resolves the owning organization for a top-level entity ID.
///
/// The dispatch key is the prefix component of a prefixed public ID
/// (e.g. `session`, `agent`, `app`). At runtime the resolver endpoint
/// iterates `inventory::iter::<ResourceOrgResolver>` looking for a match.
pub struct ResourceOrgResolver {
    /// ID prefix without the trailing underscore (e.g. `"session"`).
    pub prefix: &'static str,
    /// Resolver function. MUST NOT apply caller-side scoping — the endpoint
    /// gates the result against the caller's org memberships.
    pub resolve: ResolveFn,
}

inventory::collect!(ResourceOrgResolver);

/// Resolve the owning org for a prefixed public ID, regardless of caller.
///
/// Returns `Ok(None)` if the prefix is unknown or the resource doesn't
/// exist. Callers MUST filter the returned `org_id` against the user's
/// memberships before revealing it to the client.
pub async fn resolve_resource_org(db: &StorageBackend, id: &str) -> Result<Option<i64>> {
    let capability_id = CapabilityId::new(id);
    if let Some(server_id) = capability_id.mcp_server_id() {
        let public_id = McpServerId::from_uuid(server_id).to_string();
        return db.get_mcp_server_organization_id(&public_id).await;
    }
    if let Some(skill_id) = capability_id.skill_id() {
        let public_id = SkillId::from_uuid(skill_id).to_string();
        return db.get_skill_organization_id(&public_id).await;
    }

    let prefix = match id.split_once('_') {
        Some((p, _)) if !p.is_empty() => p,
        _ => return Ok(None),
    };
    for entry in inventory::iter::<ResourceOrgResolver> {
        if entry.prefix == prefix {
            return (entry.resolve)(db, id).await;
        }
    }
    Ok(None)
}

/// Resolve and authorize the owning org for a prefixed entity id.
///
/// SECURITY: this is the only path that may reveal an org id outside the
/// caller's currently active org. It enforces `is_organization_member` and
/// returns `None` for any failure mode (unknown id, unknown prefix, empty
/// id, non-member, vanished org row) so callers cannot distinguish them —
/// preserving the org-enumeration guarantee documented in
/// knowledge/security/multitenancy.md (THREAT[TM-TENANT-010]).
///
/// Returns `(org_public_id, org_name)` on success, `None` otherwise.
/// Used by both `GET /v1/resolve-org` and the `resolve_org` domain command.
pub async fn resolve_owning_org_for_user(
    db: &StorageBackend,
    user_id: Uuid,
    id: &str,
) -> Result<Option<(String, String)>> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Some(owning_org_id) = resolve_resource_org(db, trimmed).await? else {
        return Ok(None);
    };

    if !db.is_organization_member(owning_org_id, user_id).await? {
        return Ok(None);
    }

    // Theoretically unreachable (membership implies the org exists), but
    // treat a vanished row as `None` rather than surfacing an error.
    let Some(org) = db.get_organization(owning_org_id).await? else {
        return Ok(None);
    };

    Ok(Some((org.public_id, org.name)))
}

// ============================================================================
// Registered resolvers (one per top-level entity with a detail UI route).
// ============================================================================

inventory::submit! {
    ResourceOrgResolver {
        prefix: "session",
        resolve: |db, id| Box::pin(async move {
            let Ok(session_id) = id.parse::<SessionId>() else {
                return Ok(None);
            };
            db.get_session_organization_id(session_id).await
        }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "agent",
        resolve: |db, id| Box::pin(async move { db.get_agent_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "harness",
        resolve: |db, id| Box::pin(async move { db.get_harness_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "app",
        resolve: |db, id| Box::pin(async move { db.get_app_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "skill",
        resolve: |db, id| Box::pin(async move { db.get_skill_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "mcp",
        resolve: |db, id| Box::pin(async move { db.get_mcp_server_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "identity",
        resolve: |db, id| Box::pin(async move { db.get_agent_identity_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "eval",
        resolve: |db, id| Box::pin(async move { db.get_eval_organization_id(id).await }),
    }
}

inventory::submit! {
    ResourceOrgResolver {
        prefix: "mem",
        resolve: |db, id| Box::pin(async move { db.get_memory_organization_id(id).await }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_prefix_is_nonempty_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for entry in inventory::iter::<ResourceOrgResolver> {
            assert!(!entry.prefix.is_empty());
            assert!(!entry.prefix.contains('_'));
            assert!(
                seen.insert(entry.prefix),
                "duplicate resolver prefix: {}",
                entry.prefix
            );
        }
        // Guard against a missing `inventory::submit!` silently passing CI.
        // Must stay in sync with the registrations above.
        assert!(
            seen.len() >= 9,
            "expected at least 9 resolvers, found {}",
            seen.len()
        );
    }

    #[tokio::test]
    async fn resolve_rejects_missing_prefix() {
        let db = StorageBackend::in_memory();
        assert!(
            resolve_resource_org(&db, "not-an-id")
                .await
                .unwrap()
                .is_none()
        );
        assert!(resolve_resource_org(&db, "").await.unwrap().is_none());
        assert!(resolve_resource_org(&db, "_").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_returns_none_for_unknown_prefix() {
        let db = StorageBackend::in_memory();
        let result =
            resolve_resource_org(&db, "unknownprefix_00000000000000000000000000000001").await;
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_agent_returns_owning_org_id() {
        use crate::storage::models::CreateAgentRow;

        let db = StorageBackend::in_memory();

        // Seed an agent in org 1 and another in org 42. Mirrors the
        // cross-org situation: a user currently in one org follows a link
        // to a resource owned by another.
        let agent_one = CreateAgentRow {
            public_id: "agent_00000000000000000000000000000001".to_string(),
            name: "one".to_string(),
            display_name: None,
            description: None,
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!([]),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            is_built_in: false,
        };
        let mut agent_two = agent_one.clone();
        agent_two.public_id = "agent_00000000000000000000000000000042".to_string();
        agent_two.name = "two".to_string();

        db.create_agent(1, agent_one).await.unwrap();
        db.create_agent(42, agent_two).await.unwrap();

        // Known id → known owning org.
        let org = resolve_resource_org(&db, "agent_00000000000000000000000000000001")
            .await
            .unwrap();
        assert_eq!(org, Some(1));

        let org = resolve_resource_org(&db, "agent_00000000000000000000000000000042")
            .await
            .unwrap();
        assert_eq!(org, Some(42));

        // Same prefix, unknown id → None (never leaks a guessed org).
        let org = resolve_resource_org(&db, "agent_00000000000000000000000000000099")
            .await
            .unwrap();
        assert!(org.is_none());
    }

    #[tokio::test]
    async fn resolve_virtual_capability_returns_underlying_resource_org_id() {
        use crate::kernel_imports::{capabilities::skill_capability_id, typed_id::SkillId};
        use crate::storage::models::{CreateMcpServerRow, CreateSkillRow};
        use everruns_mcp::mcp_capability_id;

        let db = StorageBackend::in_memory();

        let mcp = db
            .create_mcp_server(
                42,
                CreateMcpServerRow {
                    name: "tools".to_string(),
                    description: None,
                    url: "https://mcp.example.test".to_string(),
                    transport_type: "streamable_http".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        let mcp_capability_id = mcp_capability_id(mcp.id.uuid());

        let skill_id = SkillId::new();
        db.create_skill(
            43,
            CreateSkillRow {
                public_id: skill_id.to_string(),
                name: "helper".to_string(),
                description: "Helper skill".to_string(),
                license: None,
                compatibility: None,
                metadata: serde_json::json!({}),
                allowed_tools: None,
                instructions: "Do useful things.".to_string(),
                source_type: "manual".to_string(),
                archive_data: None,
                version: "1.0.0".to_string(),
            },
        )
        .await
        .unwrap();
        let skill_capability_id = skill_capability_id(skill_id.uuid());

        let org = resolve_resource_org(&db, &mcp_capability_id).await.unwrap();
        assert_eq!(org, Some(42));

        let org = resolve_resource_org(&db, &skill_capability_id)
            .await
            .unwrap();
        assert_eq!(org, Some(43));
    }
}
