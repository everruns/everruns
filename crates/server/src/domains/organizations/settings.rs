//! Shared organization definition update. REST, history restore and commands
//! use the same validation and transaction; audit remains on the HTTP adapter.
use super::{
    queries as q,
    types::{OrganizationResponse, UpdateOrganizationRequest},
};
use crate::domains::common::*;
use crate::storage::{UpdateField, UpdateOrganization, UpdateOrganizationSettings};
use everruns_core::{DEFAULT_ORG_ID, OrgRole, Permission, Policy, Rule};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetOrgDecisionSettings {
    pub org: String,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct OrgDecisionSettings {
    pub model_id: Option<String>,
}
#[command(name = "get_org_decision_settings", category = "organizations", description = "Read the active organization's decision-model selection without model credentials.", method = "GET", path = "/v1/orgs/{org}/decision-settings", policy = crate::domains::models::LLM_MODEL_VIEW, cli = CliRoute::new(&["orgs", "decision-settings"], "get").with_examples(&[CliExample::new("Check the active organization decision-model selection", "everruns orgs decision-settings get --org org_01h9")]))]
impl Command for GetOrgDecisionSettings {
    type Output = OrgDecisionSettings;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        Ok(OrgDecisionSettings {
            model_id: ctx
                .db
                .get_decision_default(ctx.org_id())
                .await?
                .map(|id| everruns_contracts::typed_id::ModelId::from_uuid(id).to_string()),
        })
    }
}

pub const ORGANIZATION_VIEW: Policy = Policy {
    id: "organization.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};
pub const ORGANIZATION_MANAGE: Policy = Policy {
    id: "organization.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateOrg {
    pub org: String,
    #[serde(flatten)]
    pub request: UpdateOrganizationRequest,
}
#[command(name = "update_org", category = "organizations", description = "Update the active organization's name and defaults.", method = "PATCH", path = "/v1/orgs/{org}", policy = ORGANIZATION_MANAGE, cli = CliRoute::new(&["orgs"], "update").with_examples(&[CliExample::new("Rename the active organization while preserving its model and harness defaults", "everruns orgs update --org org_01h9 --name 'Northstar Support' --reason 'Use the team name'")]))]
impl Command for UpdateOrg {
    type Output = OrganizationResponse;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self.org != ctx.caller.org_public_id {
            return Err(CommandError::not_found("Organization"));
        }
        let membership = q::verify_membership(ctx, &self.org).await?;
        if !membership
            .role
            .parse::<OrgRole>()
            .ok()
            .is_some_and(|role| role.has_permission(OrgRole::Admin))
        {
            return Err(CommandError::forbidden(
                "Only organization admins can update organization settings",
            ));
        }
        let row = ctx
            .db
            .get_organization_by_public_id(&self.org)
            .await?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        let UpdateOrganizationRequest {
            name,
            default_model_id,
            small_model_id,
            mut default_harness_id,
            default_harness_name,
            base_harness_id,
            default_provider_per_service,
            system_decisions,
            agentid_agents_per_owner,
            web_search_defaults,
        } = self.request;
        if let Some(name) = &name {
            if name.trim().is_empty() || name.len() > 255 {
                return Err(CommandError::bad_request(
                    "Organization name must be 1 to 255 characters",
                ));
            }
            if row.org_id == DEFAULT_ORG_ID && name != &row.name {
                return Err(CommandError::bad_request(
                    "Cannot update default organization",
                ));
            }
        }
        if agentid_agents_per_owner
            .flatten()
            .is_some_and(|cap| cap < 0)
        {
            return Err(CommandError::bad_request(
                "agentid_agents_per_owner cannot be negative",
            ));
        }
        if default_harness_id.is_some() && default_harness_name.is_some() {
            return Err(CommandError::bad_request(
                "Cannot specify both default_harness_id and default_harness_name",
            ));
        }
        if let Some(name) = &default_harness_name {
            crate::domains::agents::record::validate_addressable_name(name)
                .map_err(CommandError::bad_request)?;
            default_harness_id = Some(Some(
                ctx.db
                    .get_harness_by_name(row.org_id, name)
                    .await?
                    .ok_or_else(|| CommandError::not_found("Harness"))?
                    .id,
            ));
        }
        for (selection, label) in [
            (default_model_id.as_ref(), "Default model"),
            (small_model_id.as_ref(), "Small model"),
        ] {
            if let Some(Some(id)) = selection {
                let model = ctx
                    .db
                    .get_model_with_provider(row.org_id, id.uuid())
                    .await?
                    .ok_or_else(|| CommandError::bad_request("Model not found"))?;
                crate::domains::models::catalog::require_chat(model.provider_metadata.as_ref())
                    .map_err(|_| {
                        CommandError::bad_request(format!("{label} must be a chat model"))
                    })?;
                if !model.enabled {
                    return Err(CommandError::bad_request(format!(
                        "{label} must be an enabled model"
                    )));
                }
                if model.provider_type == "chatgpt" {
                    return Err(CommandError::bad_request(
                        "Personal providers cannot be organization defaults",
                    ));
                }
            }
        }
        for id in [default_harness_id.flatten(), base_harness_id.flatten()]
            .into_iter()
            .flatten()
        {
            ctx.db
                .get_harness(row.org_id, id)
                .await?
                .ok_or_else(|| CommandError::not_found("Harness"))?;
        }
        if let Some(defaults) = &default_provider_per_service {
            for id in defaults.values() {
                let provider = ctx
                    .db
                    .get_provider(row.org_id, id.uuid())
                    .await?
                    .ok_or_else(|| CommandError::bad_request(format!("Provider {id} not found")))?;
                if provider.provider_type == "chatgpt" {
                    return Err(CommandError::bad_request(
                        "Personal providers cannot be organization defaults",
                    ));
                }
            }
        }
        let web_search_defaults = match web_search_defaults {
            Some(defaults) => Some(
                super::web_search::validate_defaults(&ctx.db, row.org_id, defaults)
                    .await
                    .map_err(|error| match error {
                        Ok(message) => CommandError::bad_request(message),
                        Err(error) => CommandError::internal(error),
                    })?,
            ),
            None => None,
        };
        let updated = ctx
            .db
            .update_organization(row.org_id, UpdateOrganization { name })
            .await?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        if default_model_id.is_some()
            || small_model_id.is_some()
            || default_harness_id.is_some()
            || base_harness_id.is_some()
            || default_provider_per_service.is_some()
            || system_decisions.is_some()
        {
            let model_field = |value| match value {
                None => UpdateField::Unchanged,
                Some(None) => UpdateField::Clear,
                Some(Some(id)) => UpdateField::Set(id),
            };
            ctx.db
                .patch_organization_settings(
                    row.org_id,
                    UpdateOrganizationSettings {
                        default_model_id: model_field(default_model_id),
                        small_model_id: model_field(small_model_id),
                        default_harness_id: default_harness_id
                            .map_or(UpdateField::Unchanged, UpdateField::from_option),
                        base_harness_id: base_harness_id
                            .map_or(UpdateField::Unchanged, UpdateField::from_option),
                        default_provider_per_service: default_provider_per_service
                            .map_or(UpdateField::Unchanged, UpdateField::Set),
                        system_decisions,
                    },
                )
                .await?;
        }
        if let Some(defaults) = web_search_defaults {
            ctx.db
                .set_org_web_search_defaults(row.org_id, &defaults)
                .await?;
        }
        if let Some(cap) = agentid_agents_per_owner {
            ctx.db.set_agentid_agents_per_owner(row.org_id, cap).await?;
        }
        q::build_organization_response(&ctx.db, row.org_id, updated).await
    }
}
