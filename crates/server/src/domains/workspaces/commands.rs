// Workspace CRUD command implementations.
//
// These commands implement the unified Command framework so they're
// surfaced in the MCP catalog, the platform_management capability tools,
// the everruns dev plugin sync, and the OpenAPI docs through a single
// registration site.

use super::types::{
    CreateWorkspaceRequest, ListWorkspacesQuery, UpdateWorkspaceRequest, WorkspaceResponse,
    workspace_response,
};
use super::{WORKSPACE_MANAGE, WORKSPACE_VIEW};
use crate::domains::common::*;
use crate::storage::models::{CreateWorkspaceRow, UpdateWorkspace};
use everruns_core::Policy;
use everruns_provider::typed_id::WorkspaceId;
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request("Workspace name cannot be empty"));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Workspace name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_workspace_id(workspace_id: &str) -> Result<WorkspaceId, CommandError> {
    workspace_id
        .parse()
        .map_err(|_| CommandError::not_found("Workspace"))
}

fn validate_status(status: &str) -> Result<(), CommandError> {
    match status {
        "active" | "archived" | "deleted" => Ok(()),
        other => Err(CommandError::bad_request(format!(
            "Invalid workspace status: {other} (expected one of: active, archived, deleted)"
        ))),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListWorkspaces {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListWorkspacesQuery> for ListWorkspaces {
    fn from(query: ListWorkspacesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: Some(query.include_archived),
        }
    }
}

impl Command for ListWorkspaces {
    type Output = Vec<WorkspaceResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_workspaces",
            category: "workspaces",
            description: "List Workspaces in the current organization.",
            method: "GET",
            path: "/v1/workspaces",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&WORKSPACE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<WorkspaceResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<WorkspaceResponse>, CommandError> {
        let rows = ctx
            .db
            .list_workspaces(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(workspace_response).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListWorkspaces>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    /// Human-readable workspace name. Must be unique per org.
    pub name: String,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

impl From<CreateWorkspaceRequest> for CreateWorkspace {
    fn from(request: CreateWorkspaceRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
        }
    }
}

impl Command for CreateWorkspace {
    type Output = WorkspaceResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_workspace",
            category: "workspaces",
            description: "Create a new Workspace in the current organization.",
            method: "POST",
            path: "/v1/workspaces",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&WORKSPACE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<WorkspaceResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<WorkspaceResponse, CommandError> {
        let name = validate_name(&self.name)?;
        let public_id = WorkspaceId::new().to_string();
        let row = ctx
            .db
            .create_workspace(
                ctx.org_id(),
                CreateWorkspaceRow {
                    id: None,
                    public_id,
                    name,
                    description: self.description,
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                },
            )
            .await
            .map_err(|e| {
                if e.to_string().contains("already exists") {
                    CommandError::conflict("Workspace name already exists")
                } else {
                    classify_anyhow(e)
                }
            })?;
        Ok(workspace_response(row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateWorkspace>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetWorkspace {
    /// Workspace ID (wsp_<32-hex>).
    pub workspace_id: String,
}

impl Command for GetWorkspace {
    type Output = WorkspaceResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_workspace",
            category: "workspaces",
            description: "Fetch a Workspace by its public ID.",
            method: "GET",
            path: "/v1/workspaces/{workspace_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&WORKSPACE_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("workspace_id")
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<WorkspaceResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<WorkspaceResponse, CommandError> {
        let id = parse_workspace_id(&self.workspace_id)?;
        let row = ctx
            .db
            .get_workspace(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Workspace"))?;
        Ok(workspace_response(row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetWorkspace>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWorkspaceCmd {
    /// Workspace ID (wsp_<32-hex>).
    pub workspace_id: String,
    #[serde(flatten)]
    pub request: UpdateWorkspaceRequest,
}

impl Command for UpdateWorkspaceCmd {
    type Output = WorkspaceResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_workspace",
            category: "workspaces",
            description: "Update a Workspace's mutable fields.",
            method: "PATCH",
            path: "/v1/workspaces/{workspace_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&WORKSPACE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<WorkspaceResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<WorkspaceResponse, CommandError> {
        let id = parse_workspace_id(&self.workspace_id)?;
        let existing = ctx
            .db
            .get_workspace(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Workspace"))?;

        let name = self.request.name.map(|n| validate_name(&n)).transpose()?;
        if let Some(status) = self.request.status.as_deref() {
            validate_status(status)?;
        }

        let row = ctx
            .db
            .update_workspace(
                ctx.org_id(),
                existing.id,
                UpdateWorkspace {
                    name,
                    description: self.request.description,
                    status: self.request.status,
                },
            )
            .await
            .map_err(|e| {
                if e.to_string().contains("already exists") {
                    CommandError::conflict("Workspace name already exists")
                } else {
                    classify_anyhow(e)
                }
            })?
            .ok_or_else(|| CommandError::not_found("Workspace"))?;
        Ok(workspace_response(row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateWorkspaceCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteWorkspace {
    /// Workspace ID (wsp_<32-hex>).
    pub workspace_id: String,
}

impl Command for DeleteWorkspace {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_workspace",
            category: "workspaces",
            description: "Archive a Workspace (soft-delete).",
            method: "DELETE",
            path: "/v1/workspaces/{workspace_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&WORKSPACE_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("workspace_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_workspace_id(&self.workspace_id)?;
        let existing = ctx
            .db
            .get_workspace(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Workspace"))?;
        ctx.db
            .archive_workspace(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        Ok(())
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteWorkspace>() }
