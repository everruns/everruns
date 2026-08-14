use super::queries as q;
use super::types::{ListOrganizationsResponse, OrganizationResponse, ResolveOrgResponse};
use crate::domains::common::*;
use crate::domains::org_resolver;
use everruns_platform::validate_org_public_id;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListOrgs {}

impl Command for ListOrgs {
    type Output = ListOrganizationsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_orgs",
            category: "organizations",
            description: "List organizations for the current user.",
            method: "GET",
            path: "/v1/orgs",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListOrganizationsResponse, CommandError> {
        let memberships = q::list_user_organizations(ctx).await?;
        let mut orgs = Vec::with_capacity(memberships.len());

        for membership in memberships {
            if let Some(row) = ctx
                .db
                .get_organization(membership.org_id)
                .await
                .map_err(classify_anyhow)?
            {
                orgs.push(q::build_organization_response(&ctx.db, membership.org_id, row).await?);
            }
        }

        Ok(crate::api::common::ListResponse::new(orgs))
    }
}

inventory::submit! { CommandDescriptor::of::<ListOrgs>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetOrg {
    pub org: String,
}

impl Command for GetOrg {
    type Output = OrganizationResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_org",
            category: "organizations",
            description: "Get organization details.",
            method: "GET",
            path: "/v1/orgs/{org}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("org")
    }

    async fn execute(self, ctx: &Ctx) -> Result<OrganizationResponse, CommandError> {
        if !validate_org_public_id(&self.org) {
            return Err(CommandError::not_found("Organization"));
        }

        let membership = q::verify_membership(ctx, &self.org).await?;
        let row = ctx
            .db
            .get_organization_by_public_id(&self.org)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Organization"))?;

        q::build_organization_response(&ctx.db, membership.org_id, row).await
    }
}

inventory::submit! { CommandDescriptor::of::<GetOrg>() }

// SECURITY: this is the only domain command that may reveal an `org_id` the
// caller's active session is not currently scoped to. The shared helper
// `org_resolver::resolve_owning_org_for_user` enforces the membership gate
// (THREAT[TM-TENANT-010]); we return NotFound for every failure mode to
// preserve the org-enumeration guarantee. See knowledge/security/multitenancy.md
// (Cross-Org Resource Resolution).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveOrg {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for ResolveOrg {
    type Output = ResolveOrgResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "resolve_org",
            category: "organizations",
            description: "Resolve the owning organization for a prefixed entity id (agent, session, harness, app, skill, mcp server, identity, eval). Requires an authenticated user; returns NotFound when the caller is not a member of the owning org or the id does not resolve.",
            method: "GET",
            path: "/v1/resolve-org",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ResolveOrgResponse, CommandError> {
        let user_id = q::require_user_id(ctx)?;

        let resolved = org_resolver::resolve_owning_org_for_user(&ctx.db, user_id, &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Resource"))?;

        Ok(ResolveOrgResponse {
            org_id: resolved.0,
            org_name: resolved.1,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ResolveOrg>() }

#[cfg(test)]
mod tests {
    use crate::domains::common::{CommandError, CommandErrorKind, Ctx};
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateAgentRow;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    fn seed_caller(user_id: Uuid) -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(user_id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        }
    }

    #[tokio::test]
    async fn resolve_org_returns_owning_org_when_caller_is_member() {
        let db = Arc::new(StorageBackend::in_memory());
        crate::seed::seed_all(
            &db,
            everruns_core::DeploymentGrade::Dev,
            &crate::seed::SeedAuthContext::default(),
        )
        .await
        .expect("seed test data");

        let agent_public_id = "agent_00000000000000000000000000000077".to_string();
        db.create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: agent_public_id.clone(),
                name: "resolve-test".to_string(),
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
            },
        )
        .await
        .expect("create agent");

        let ctx =
            Ctx::minimal_for_test(seed_caller(everruns_platform::ANONYMOUS_USER_ID), db, None);

        let json =
            crate::domains::common::dispatch("resolve_org", json!({ "id": agent_public_id }), &ctx)
                .await
                .expect("dispatch resolve_org");
        let response: serde_json::Value =
            serde_json::from_str(&json).expect("deserialize resolve_org response");
        assert_eq!(
            response.get("org_id").and_then(|v| v.as_str()),
            Some("org_00000000000000000000000000000001")
        );
        assert!(response.get("org_name").and_then(|v| v.as_str()).is_some());
    }

    #[tokio::test]
    async fn resolve_org_returns_not_found_for_non_member() {
        let db = Arc::new(StorageBackend::in_memory());

        // Other-org agent: created in org 42 with no membership for the caller.
        let agent_public_id = "agent_00000000000000000000000000000088".to_string();
        db.create_agent(
            42,
            CreateAgentRow {
                public_id: agent_public_id.clone(),
                name: "other-org-agent".to_string(),
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
            },
        )
        .await
        .expect("create agent");

        let ctx = Ctx::minimal_for_test(seed_caller(Uuid::new_v4()), db, None);

        let err =
            crate::domains::common::dispatch("resolve_org", json!({ "id": agent_public_id }), &ctx)
                .await
                .expect_err("dispatch resolve_org should fail for non-member");
        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::NotFound(_),
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn resolve_org_returns_not_found_for_unknown_prefix() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx =
            Ctx::minimal_for_test(seed_caller(everruns_platform::ANONYMOUS_USER_ID), db, None);

        let err = crate::domains::common::dispatch(
            "resolve_org",
            json!({ "id": "totallybogus_00000000000000000000000000000001" }),
            &ctx,
        )
        .await
        .expect_err("unknown prefix should not resolve");
        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::NotFound(_),
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_orgs_dispatch_accepts_empty_object_params() {
        let db = Arc::new(StorageBackend::in_memory());
        crate::seed::seed_all(
            &db,
            everruns_core::DeploymentGrade::Dev,
            &crate::seed::SeedAuthContext::default(),
        )
        .await
        .expect("seed test data");

        let ctx = Ctx::minimal_for_test(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: "org_00000000000000000000000000000001".to_string(),
                user_id: Some(everruns_platform::ANONYMOUS_USER_ID),
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            None,
        );

        let json = crate::domains::common::dispatch("list_orgs", json!({}), &ctx)
            .await
            .expect("dispatch list_orgs");
        let response: serde_json::Value =
            serde_json::from_str(&json).expect("deserialize list_orgs response");
        assert!(
            response
                .get("data")
                .and_then(|value| value.as_array())
                .is_some_and(|data| !data.is_empty())
        );
    }
}
