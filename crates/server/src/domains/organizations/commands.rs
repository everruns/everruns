use super::queries as q;
use super::types::{ListOrganizationsResponse, OrganizationResponse};
use crate::domains::common::*;
use everruns_core::validate_org_public_id;
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

#[cfg(test)]
mod tests {
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use serde_json::json;
    use std::sync::Arc;

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
                user_id: Some(everruns_core::ANONYMOUS_USER_ID),
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
