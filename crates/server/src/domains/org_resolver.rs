// Cross-org resource resolution.
//
// Every top-level entity that has a dedicated UI detail route MUST register a
// resolver here so the UI can auto-switch the caller's active org when they
// follow a direct link to a resource owned by a different (but still member)
// org. The API keeps returning 404 for cross-org access — only the UI calls
// the gated /v1/resolve-org endpoint to recover. See
// specs/multitenancy.md (Cross-Org Resource Resolution).
//
// Adding a new top entity: implement a storage method
// `get_<entity>_organization_id(public_id: &str) -> Result<Option<i64>>` and
// add an `inventory::submit!` block below. The HTTP endpoint dispatches by
// the prefix component of the ID (`session_...`, `agent_...`, etc.) so no
// other wiring is required.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use everruns_core::typed_id::SessionId;

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
}
