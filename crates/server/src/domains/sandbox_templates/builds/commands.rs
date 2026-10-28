use super::record::*;
use crate::domains::common::*;
use crate::domains::harnesses::{HARNESS_MANAGE, HARNESS_VIEW};
use crate::domains::sandbox_templates::record::{SandboxTargetKind, SandboxTemplate};
use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
use everruns_contracts::typed_id::{SandboxTemplateBuildId, SandboxTemplateId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartSandboxTemplateBuild {
    pub id: SandboxTemplateId,
    #[serde(flatten)]
    pub req: SandboxBuildRequest,
}
#[command(name="start_sandbox_template_build", http=plain, request_body(SandboxBuildRequest), tag="sandbox-templates", category="sandbox-templates", summary="Build a Sandbox Template snapshot", description="Run an install script in fresh isolated compute and bake a reusable provider snapshot. Does not revise the template.", method="POST", path="/v1/sandbox-templates/{id}/builds", policy=HARNESS_MANAGE, cli=CliRoute::new(&["sandbox-templates","builds"],"start").with_examples(&[CliExample::new("Start a Sandbox Template build", "everruns sandbox-templates builds start --id sbxtpl_01933b5a000070008000000000000001 --source-revision-id sbxtplrev_01933b5a000070008000000000000001 --install-script 'echo ready'")]))]
impl Command for StartSandboxTemplateBuild {
    type Output = SandboxTemplateBuild;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        self.req.validate().map_err(CommandError::bad_request)?;
        if !ctx.feature_flags.is_connector_enabled("daytona") {
            return Err(CommandError::not_found("Daytona connector"));
        }
        if ctx.encryption.is_none() {
            return Err(CommandError::unavailable(
                "Builds require configured credential encryption",
            ));
        }
        let template = ctx
            .db
            .get_sandbox_template(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template"))?;
        if template.is_managed || template.status != "active" {
            return Err(CommandError::conflict(
                "Managed or archived templates cannot be built",
            ));
        }
        if template.current_revision.public_id != self.req.source_revision_id {
            return Err(CommandError::conflict(
                "The source revision changed; reload the template",
            ));
        }
        let spec = template.current_revision.spec;
        if spec.target.kind != SandboxTargetKind::Managed
            || spec.target.provider.as_deref() != Some("daytona")
        {
            return Err(CommandError::bad_request(
                "Snapshot builds currently support Daytona targets",
            ));
        }
        if spec.target.credential.source != SessionSandboxCredentialSource::Organization {
            return Err(CommandError::bad_request(
                "Select a named Organization provider account before building",
            ));
        }
        let connection =
            spec.target.credential.connection_id.ok_or_else(|| {
                CommandError::bad_request("Select an Organization provider account")
            })?;
        ctx.db
            .get_organization_connection(ctx.org_id(), connection)
            .await?
            .filter(|a| a.provider == "daytona" && a.access_token_encrypted.is_some())
            .ok_or_else(|| {
                CommandError::bad_request("The Daytona organization account is unavailable")
            })?;
        let mut containment = crate::domains::sandbox_templates::resolution::resolve_spec(&spec)
            .map_err(CommandError::bad_request)?
            .containment;
        if let Some(network) = &self.req.build_network {
            containment.network = network.clone()
        }
        let mut build_spec = spec.clone();
        build_spec.containment = Some(containment);
        crate::domains::sandbox_templates::resolution::resolve_spec(&build_spec)
            .map_err(CommandError::bad_request)?;
        let id = SandboxTemplateBuildId::new();
        let now = chrono::Utc::now();
        let build = SandboxTemplateBuild {
            id,
            sandbox_template_id: self.id,
            request: self.req,
            source_spec: spec,
            status: SandboxBuildStatus::Queued,
            step: "queued".into(),
            logs: String::new(),
            error: None,
            software_notes: None,
            snapshot_name: format!("everruns-build-{}", id.uuid().simple()),
            snapshot_id: None,
            snapshot_requested_at: None,
            builder_id: None,
            verification_sandbox_id: None,
            cancel_requested: false,
            cleanup_pending: false,
            applied_revision_id: None,
            created_at: now,
            updated_at: now,
        };
        if !ctx.db.insert_sandbox_build(ctx.org_id(), &build).await? {
            return Err(CommandError::conflict(
                "An organization build is already running",
            ));
        }
        Ok(build)
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListSandboxTemplateBuilds {
    pub id: SandboxTemplateId,
    #[serde(default)]
    pub offset: u32,
}
#[command(name="list_sandbox_template_builds", http=plain, tag="sandbox-templates",category="sandbox-templates",summary="List Sandbox Template builds",description="List twenty build attempts at an optional offset, bounded logs, and captured software notes.",method="GET",path="/v1/sandbox-templates/{id}/builds",policy=HARNESS_VIEW,cli=CliRoute::new(&["sandbox-templates","builds"],"list").with_examples(&[CliExample::new("List a Sandbox Template build", "everruns sandbox-templates builds list --id sbxtpl_01933b5a000070008000000000000001")]))]
impl Command for ListSandboxTemplateBuilds {
    type Output = Vec<SandboxTemplateBuild>;
    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<SandboxTemplateBuild>())
    }
    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        Ok(ctx
            .db
            .list_sandbox_builds(ctx.org_id(), self.id, self.offset)
            .await?)
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetSandboxTemplateBuild {
    pub id: SandboxTemplateBuildId,
}
#[command(name="get_sandbox_template_build",http=plain,tag="sandbox-templates",category="sandbox-templates",summary="Get Sandbox Template build",description="Inspect a build attempt in the authenticated organization.",method="GET",path="/v1/sandbox-template-builds/{id}",policy=HARNESS_VIEW,cli=CliRoute::new(&["sandbox-templates","builds"],"get").with_examples(&[CliExample::new("Get a Sandbox Template build", "everruns sandbox-templates builds get --id sbxbuild_01933b5a000070008000000000000001")]))]
impl Command for GetSandboxTemplateBuild {
    type Output = SandboxTemplateBuild;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        ctx.db
            .get_sandbox_build(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CancelSandboxTemplateBuild {
    pub id: SandboxTemplateBuildId,
}
#[command(name="cancel_sandbox_template_build",http=plain,tag="sandbox-templates",category="sandbox-templates",summary="Cancel Sandbox Template build",description="Cancel queued or running build work and clean up its disposable resources.",method="POST",path="/v1/sandbox-template-builds/{id}/cancel",policy=HARNESS_MANAGE,cli=CliRoute::new(&["sandbox-templates","builds"],"cancel").with_examples(&[CliExample::new("Cancel a Sandbox Template build", "everruns sandbox-templates builds cancel --id sbxbuild_01933b5a000070008000000000000001")]))]
impl Command for CancelSandboxTemplateBuild {
    type Output = SandboxTemplateBuild;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if !ctx.db.cancel_sandbox_build(ctx.org_id(), self.id).await? {
            return Err(CommandError::conflict("Build is not queued or running"));
        }
        ctx.db
            .get_sandbox_build(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApplySandboxTemplateBuild {
    pub id: SandboxTemplateBuildId,
}
#[command(name="apply_sandbox_template_build",http=plain,tag="sandbox-templates",category="sandbox-templates",summary="Save built snapshot as new revision",description="Atomically apply a ready snapshot to its unchanged source template. Existing Agent and Session pins do not move.",method="POST",path="/v1/sandbox-template-builds/{id}/apply",policy=HARNESS_MANAGE,cli=CliRoute::new(&["sandbox-templates","builds"],"apply").with_examples(&[CliExample::new("Apply a Sandbox Template build", "everruns sandbox-templates builds apply --id sbxbuild_01933b5a000070008000000000000001")]))]
impl Command for ApplySandboxTemplateBuild {
    type Output = SandboxTemplate;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let build = ctx
            .db
            .get_sandbox_build(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))?;
        let connection = build
            .source_spec
            .target
            .credential
            .connection_id
            .ok_or_else(|| CommandError::conflict("Build account is unavailable"))?;
        if ctx
            .db
            .get_organization_connection(ctx.org_id(), connection)
            .await?
            .is_none()
        {
            return Err(CommandError::conflict("Build account is unavailable"));
        }
        if build.status != SandboxBuildStatus::Applied {
            let encryption = ctx
                .encryption
                .as_ref()
                .ok_or_else(|| CommandError::unavailable("Credential encryption unavailable"))?;
            let token = super::runner::current_token(&ctx.db, encryption, ctx.org_id(), &build)
                .await
                .map_err(|_| CommandError::conflict("Build account is unavailable"))?;
            let snapshot = build
                .snapshot_id
                .as_deref()
                .ok_or_else(|| CommandError::conflict("Build has no snapshot"))?;
            let snapshot = everruns_integrations::daytona::client::DaytonaClient::new(token)
                .get_snapshot(snapshot)
                .await
                .map_err(|_| CommandError::conflict("Provider snapshot is unavailable"))?;
            if snapshot.state != "active" {
                return Err(CommandError::conflict("Provider snapshot is not active"));
            }
        }
        ctx.db
            .apply_sandbox_build(ctx.org_id(), self.id)
            .await
            .map_err(build_error)?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DiscardSandboxTemplateBuild {
    pub id: SandboxTemplateBuildId,
}
#[command(name="discard_sandbox_template_build",http=plain,tag="sandbox-templates",category="sandbox-templates",summary="Discard unused build artifact",description="Remove an unused build artifact. Applied artifacts remain pinned and cannot be discarded.",method="POST",path="/v1/sandbox-template-builds/{id}/discard",policy=HARNESS_MANAGE,cli=CliRoute::new(&["sandbox-templates","builds"],"discard").with_examples(&[CliExample::new("Discard a Sandbox Template build", "everruns sandbox-templates builds discard --id sbxbuild_01933b5a000070008000000000000001")]))]
impl Command for DiscardSandboxTemplateBuild {
    type Output = SandboxTemplateBuild;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        ctx.db
            .get_sandbox_build(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))?;
        if !ctx
            .db
            .discard_sandbox_build(ctx.org_id(), self.id)
            .await
            .map_err(build_error)?
        {
            return Err(CommandError::conflict(
                "Only unapplied finished builds may be discarded",
            ));
        }
        ctx.db
            .get_sandbox_build(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Sandbox Template build"))
    }
}

fn build_error(error: anyhow::Error) -> CommandError {
    if let Some(conflict) = error.downcast_ref::<SandboxBuildConflict>() {
        CommandError::conflict(conflict.to_string())
    } else {
        CommandError::from(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattened_command_rejects_unknown_secret_inputs() {
        let mut value = serde_json::json!({
            "id": SandboxTemplateId::new(),
            "source_revision_id": everruns_contracts::typed_id::SandboxTemplateRevisionId::new(),
            "install_script": "echo ready"
        });
        assert!(serde_json::from_value::<StartSandboxTemplateBuild>(value.clone()).is_ok());
        value["credentials"] = serde_json::json!({"TOKEN": "must-not-be-accepted"});
        assert!(serde_json::from_value::<StartSandboxTemplateBuild>(value).is_err());
    }
}
