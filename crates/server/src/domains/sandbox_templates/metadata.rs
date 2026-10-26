//! Editable template metadata. Execution specifications never enter Chat history.
use crate::domains::common::*;
use crate::domains::harnesses::{HARNESS_MANAGE, HARNESS_VIEW};
use everruns_contracts::typed_id::SandboxTemplateId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateMetadata {
    pub id: SandboxTemplateId,
    pub display_name: String,
    pub description: Option<String>,
}
fn metadata(value: super::record::SandboxTemplate) -> TemplateMetadata {
    TemplateMetadata {
        id: value.public_id,
        display_name: value.display_name,
        description: value.description,
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetSandboxTemplateMetadata {
    pub id: SandboxTemplateId,
}
#[command(name = "get_sandbox_template_metadata", category = "sandbox-templates", description = "Read editable Sandbox Template metadata, excluding execution configuration and environment values.", method = "GET", path = "/v1/sandbox-template-metadata/{id}", policy = HARNESS_VIEW, cli = CliRoute::new(&["sandbox-template-metadata"], "get").with_examples(&[CliExample::new("Inspect a template display name without reading its execution configuration", "everruns sandbox-template-metadata get --id sbxtpl_01h9")]))]
impl Command for GetSandboxTemplateMetadata {
    type Output = TemplateMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        Ok(metadata(
            ctx.db
                .get_sandbox_template(ctx.org_id(), self.id)
                .await?
                .ok_or_else(|| CommandError::not_found("Sandbox Template"))?,
        ))
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateSandboxTemplateMetadata {
    pub id: SandboxTemplateId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<Option<String>>,
}
fn double_option<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(de).map(Some)
}
#[command(name = "update_sandbox_template_metadata", category = "sandbox-templates", description = "Update Sandbox Template metadata while preserving its current execution specification.", method = "PATCH", path = "/v1/sandbox-template-metadata/{id}", policy = HARNESS_MANAGE, cli = CliRoute::new(&["sandbox-template-metadata"], "update").with_examples(&[CliExample::new("Clarify a template label while preserving the current sandbox specification", "everruns sandbox-template-metadata update --id sbxtpl_01h9 --display-name 'Northstar review sandbox' --reason 'Clarify the template purpose'")]))]
impl Command for UpdateSandboxTemplateMetadata {
    type Output = TemplateMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if self
            .display_name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty() || name.len() > 200)
        {
            return Err(CommandError::bad_request(
                "display_name must be 1 to 200 bytes",
            ));
        }
        if self
            .description
            .as_ref()
            .and_then(|value| value.as_ref())
            .is_some_and(|value| value.len() > 10240)
        {
            return Err(CommandError::bad_request(
                "description must be at most 10240 bytes",
            ));
        }
        let current = ctx
            .db
            .get_sandbox_template(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template"))?;
        if current.is_managed || current.status != "active" {
            return Err(CommandError::conflict(
                "Managed or archived Sandbox Templates cannot be revised",
            ));
        }
        let updated = ctx
            .db
            .revise_sandbox_template(
                ctx.org_id(),
                self.id,
                &current.current_revision.spec,
                self.display_name.as_deref().map(str::trim),
                self.description.as_ref().map(|value| value.as_deref()),
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template"))?;
        Ok(metadata(updated))
    }
}
