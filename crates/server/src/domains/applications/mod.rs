//! Applications own publication, audience and navigation. Agents still own
//! execution; Rooms and Memory keep their existing storage/runtime semantics.
use crate::domains::common::*;
use everruns_contracts::typed_id::ApplicationId;
use everruns_core::{Permission, Policy, Rule};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
pub mod types;
pub use types::*;
pub const APPLICATION_VIEW: Policy = Policy {
    id: "application.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};
pub const APPLICATION_MANAGE: Policy = Policy {
    id: "application.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListApplications {
    /// Include archived Applications so their owners can restore them as drafts.
    #[serde(default)]
    pub include_archived: bool,
}
#[command(name="list_applications",category="applications",description="List Applications in the current organization.",method="GET",path="/v1/platform/applications",policy=APPLICATION_VIEW,http=plain,cli=CliRoute::new(&["applications"],"list").with_examples(&[CliExample::new("Find active and archived workspaces","everruns applications list --include-archived")]))]
impl Command for ListApplications {
    type Output = Vec<Application>;
    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<Application>())
    }
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        Ok(ctx
            .db
            .list_applications_including_archived(ctx.org_id(), self.include_archived)
            .await?)
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetApplication {
    /// Public ID of the Application in the current organization.
    pub application_id: ApplicationId,
}
#[command(name="get_application",category="applications",description="Get an Application's configuration and resources.",method="GET",path="/v1/platform/applications/{application_id}",policy=APPLICATION_VIEW,http=plain,cli=CliRoute::new(&["applications"],"get").with_examples(&[CliExample::new("Read the current resource bindings and version","everruns applications get --application-id application_01a213c3f773718e9d2b8c895eee5d36")]))]
impl Command for GetApplication {
    type Output = Application;
    fn output_schema() -> serde_json::Value {
        output_schema_for::<Application>()
    }
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = ctx
            .db
            .application(ctx.org_id(), self.application_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Application"))?;
        Ok(ctx.db.application_view(row).await?)
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateApplication {
    #[serde(flatten)]
    pub input: ApplicationInput,
}
#[command(name="create_application",category="applications",summary="Create Application",description="Create an Application with a permanently reserved stable name.",method="POST",path="/v1/platform/applications",policy=APPLICATION_MANAGE,http=created,cli=CliRoute::new(&["applications"],"create").with_examples(&[CliExample::new("Reserve a workspace name for organization users","everruns applications create --name 'Content Studio' --slug content-studio --config '{\"membership\":\"org_users\"}' --reason 'Create the editorial workspace'")]))]
impl Command for CreateApplication {
    type Output = Application;
    fn output_schema() -> serde_json::Value {
        output_schema_for::<Application>()
    }
    async fn execute(mut self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        self.input.prepare()?;
        ctx.db
            .save_application(ctx.org_id(), ApplicationId::new(), self.input, None)
            .await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateApplication {
    /// Public ID of the Application to update.
    pub application_id: ApplicationId,
    /// Current version from the latest read; stale edits are rejected.
    pub version: i64,
    #[serde(flatten)]
    pub input: ApplicationInput,
}
#[command(name="update_application",category="applications",summary="Update Application",description="Update an Application and its resource bindings, checking its version.",method="PATCH",path="/v1/platform/applications/{application_id}",policy=APPLICATION_MANAGE,http=plain,cli=CliRoute::new(&["applications"],"update").with_examples(&[CliExample::new("Rename an empty draft before attaching resources","everruns applications update --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 1 --name 'Editorial Studio' --slug content-studio --reason 'Name the new draft'")]))]
impl Command for UpdateApplication {
    type Output = Application;
    fn output_schema() -> serde_json::Value {
        output_schema_for::<Application>()
    }
    async fn execute(mut self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        self.input.prepare()?;
        ctx.db
            .save_application(
                ctx.org_id(),
                self.application_id,
                self.input,
                Some(self.version),
            )
            .await
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    Draft,
    Published,
    Archived,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetApplicationStatus {
    /// Public ID of the Application whose publication state should change.
    pub application_id: ApplicationId,
    /// Current version from the latest read.
    pub version: i64,
    /// Publish, unpublish as a draft, or archive while retaining the stable name.
    pub status: ApplicationStatus,
}
#[command(name="set_application_status",category="applications",description="Publish, unpublish or archive an Application. Archiving retains its stable name.",method="POST",path="/v1/platform/applications/{application_id}/status",policy=APPLICATION_MANAGE,http=plain,cli=CliRoute::new(&["applications"],"status").with_examples(&[CliExample::new("Publish the saved workspace and its audience","everruns applications status --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 3 --status published --reason 'Open the workspace to its audience'")]))]
impl Command for SetApplicationStatus {
    type Output = Application;
    fn output_schema() -> serde_json::Value {
        output_schema_for::<Application>()
    }
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        if matches!(self.status, ApplicationStatus::Published) {
            let app = GetApplication {
                application_id: self.application_id,
            }
            .execute(ctx)
            .await?;
            if app.resources.is_empty() {
                return Err(CommandError::bad_request(
                    "Add at least one Chat, Room or Memory before publishing",
                ));
            }
            if public_url(&app.slug).is_none() {
                return Err(CommandError::bad_request(
                    "Configure the deployment's CHAT_DOMAIN before publishing",
                ));
            }
            for resource in &app.resources {
                match resource.kind {
                    ApplicationResourceKind::Chat => {
                        let row = ctx
                            .db
                            .get_ingress_channel_by_public_id(&resource.resource_id)
                            .await?
                            .ok_or_else(|| CommandError::bad_request("Chat unavailable"))?;
                        if row.org_id != ctx.org_id()
                            || row.channel_status != "live"
                            || !row.enabled
                            || row.exposures_suspended
                            || row.agent_status != "active"
                        {
                            return Err(CommandError::bad_request(
                                "Publish every Chat before publishing the Application",
                            ));
                        }
                    }
                    ApplicationResourceKind::Room => {
                        let room = ctx
                            .db
                            .get_room(
                                ctx.org_id(),
                                resource
                                    .resource_id
                                    .parse()
                                    .map_err(|_| CommandError::bad_request("Invalid room"))?,
                            )
                            .await?
                            .ok_or_else(|| CommandError::bad_request("Room unavailable"))?;
                        if room.archived_at.is_some() {
                            return Err(CommandError::bad_request("Room is archived"));
                        }
                    }
                    ApplicationResourceKind::Memory => {
                        let memory = ctx
                            .db
                            .get_memory(
                                ctx.org_id(),
                                resource
                                    .resource_id
                                    .parse()
                                    .map_err(|_| CommandError::bad_request("Invalid memory"))?,
                            )
                            .await?
                            .ok_or_else(|| CommandError::bad_request("Memory unavailable"))?;
                        if memory.status != "active"
                            || memory.scope != "org"
                            || (resource.writable && memory.is_readonly)
                        {
                            return Err(CommandError::bad_request(
                                "Memory unavailable or incompatible access",
                            ));
                        }
                    }
                }
            }
        }
        ctx.db
            .set_application_status(
                ctx.org_id(),
                self.application_id,
                self.version,
                match self.status {
                    ApplicationStatus::Draft => "draft",
                    ApplicationStatus::Published => "published",
                    ApplicationStatus::Archived => "archived",
                },
            )
            .await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateApplicationResource {
    /// Public ID of the Application that will own the resource binding.
    pub application_id: ApplicationId,
    /// Current Application version; creation and binding are one transaction.
    pub version: i64,
    /// Create a Work Chat, Room, or organization Memory.
    pub kind: ApplicationResourceKind,
    /// Navigation label and initial name for a new Room or Memory.
    pub name: String,
    /// Unique navigation path within the Application.
    pub slug: String,
    /// Existing Agent to use for a new Chat; required only for Chat resources.
    pub agent_id: Option<String>,
}
#[command(name="create_application_resource",category="applications",summary="Create Application resource",description="Create and attach a Chat, Room or Memory atomically to an Application.",method="POST",path="/v1/platform/applications/{application_id}/resources",policy=APPLICATION_MANAGE,http=plain,cli=CliRoute::new(&["applications","resources"],"create").with_examples(&[CliExample::new("Create a Chat using an existing Agent","everruns applications resources create --application-id application_01a213c3f773718e9d2b8c895eee5d36 --version 1 --kind chat --name Writer --slug writer --agent-id agent_01a213c7b3cf76de8ec474d1c5b7fb27 --reason 'Add the writing assistant'")]))]
impl Command for CreateApplicationResource {
    type Output = Application;
    fn output_schema() -> serde_json::Value {
        output_schema_for::<Application>()
    }
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let application = GetApplication {
            application_id: self.application_id,
        }
        .execute(ctx)
        .await?;
        if application.version != self.version || application.status == "archived" {
            return Err(CommandError::conflict(
                "Application changed; reload before adding resources",
            ));
        }
        // The outer command transaction includes creation, publication and
        // binding: a new Chat is never briefly exposed to an open audience.
        let resource_id = match self.kind {
            ApplicationResourceKind::Chat => {
                use crate::domains::agent_channels::{CreateAgentChannel, PublishAgentChannel};
                let agent_id = self
                    .agent_id
                    .ok_or_else(|| CommandError::bad_request("Select an Agent"))?;
                let channel = CreateAgentChannel {
                    agent_id: agent_id.clone(),
                    req: crate::domains::agent_channels::types::CreateAgentChannelRequest {
                        channel_type: crate::domains::agent_channels::record::ChannelType::WorkChat,
                        // The Application governs admission while bound. If
                        // detached later, a newly created Chat stays org-only
                        // until its author explicitly chooses another audience.
                        channel_config: serde_json::json!({"interface_mode":"multi_chat","membership":"org_users"}),
                        enabled: true,
                    },
                }
                .run(ctx)
                .await?;
                PublishAgentChannel {
                    agent_id,
                    channel_id: channel.public_id.to_string(),
                }
                .run(ctx)
                .await?;
                channel.public_id.to_string()
            }
            ApplicationResourceKind::Room => {
                let command: crate::domains::rooms::CreateRoom =
                    serde_json::from_value(serde_json::json!({"name":self.name}))
                        .map_err(anyhow::Error::from)?;
                command.run(ctx).await?.id
            }
            ApplicationResourceKind::Memory => crate::domains::memory::CreateMemory {
                name: self.name.clone(),
                description: None,
                source: None,
            }
            .run(ctx)
            .await?
            .id
            .to_string(),
        };
        let mut input = ApplicationInput {
            name: application.name,
            slug: application.slug,
            description: application.description,
            config: application.config,
            resources: application.resources,
        };
        input.resources.push(ApplicationResource {
            kind: self.kind,
            name: self.name,
            slug: self.slug,
            resource_id,
            writable: false,
        });
        input.prepare()?;
        ctx.db
            .save_application(ctx.org_id(), self.application_id, input, Some(self.version))
            .await
    }
}
