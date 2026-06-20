// Project domain commands.
//
// Registered via `inventory::submit!` so they surface through the command
// dispatch table and the MCP catalog (discover/query/execute), mirroring how
// organizations are exposed. The REST adapter (api/projects.rs) runs these.

use super::types::{DeleteProjectResponse, ProjectResponse};
use crate::api::common::ListResponse;
use crate::domains::common::*;
use crate::storage::models::{CreateProjectRow, UpdateProject as UpdateProjectRow};
use everruns_core::{generate_project_public_id, validate_project_public_id};
use serde::Deserialize;
use utoipa::ToSchema;

/// Maximum projects per organization (guards against runaway creation).
const MAX_PROJECTS_PER_ORG: usize = 100;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListProjects {}

impl Command for ListProjects {
    type Output = ListResponse<ProjectResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_projects",
            category: "projects",
            description: "List projects in the active organization.",
            method: "GET",
            path: "/v1/projects",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let rows = ctx
            .db
            .list_projects(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        Ok(ListResponse::new(
            rows.into_iter().map(ProjectResponse::from).collect(),
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<ListProjects>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetProject {
    /// Project public ID (proj_<32-hex>).
    pub project: String,
}

impl Command for GetProject {
    type Output = ProjectResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_project",
            category: "projects",
            description: "Get a project by public ID.",
            method: "GET",
            path: "/v1/projects/{project}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("project")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = ctx
            .db
            .get_project_by_public_id(ctx.org_id(), &self.project)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Project"))?;
        Ok(row.into())
    }
}

inventory::submit! { CommandDescriptor::of::<GetProject>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProject {
    /// Display name (unique per org, case-insensitive).
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

impl Command for CreateProject {
    type Output = ProjectResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_project",
            category: "projects",
            description: "Create a project in the active organization.",
            method: "POST",
            path: "/v1/projects",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(CommandError::bad_request("Project name cannot be empty"));
        }
        if name.len() > 255 {
            return Err(CommandError::bad_request(
                "Project name cannot exceed 255 characters",
            ));
        }

        let existing = ctx
            .db
            .list_projects(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        if existing.len() >= MAX_PROJECTS_PER_ORG {
            return Err(CommandError::conflict(format!(
                "Project limit reached (max {MAX_PROJECTS_PER_ORG})"
            )));
        }
        if existing.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            return Err(CommandError::conflict(
                "A project with this name already exists",
            ));
        }

        let row = ctx
            .db
            .create_project(CreateProjectRow {
                public_id: generate_project_public_id(),
                org_id: ctx.org_id(),
                name,
                description: self.description.map(|d| d.trim().to_string()),
                is_default: false,
            })
            .await
            .map_err(classify_anyhow)?;
        Ok(row.into())
    }
}

inventory::submit! { CommandDescriptor::of::<CreateProject>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProject {
    /// Project public ID to update.
    pub project: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Command for UpdateProject {
    type Output = ProjectResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_project",
            category: "projects",
            description: "Update a project's name or description.",
            method: "PATCH",
            path: "/v1/projects/{project}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("project")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let current = ctx
            .db
            .get_project_by_public_id(ctx.org_id(), &self.project)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Project"))?;

        let name = match self.name {
            Some(n) => {
                let n = n.trim().to_string();
                if n.is_empty() {
                    return Err(CommandError::bad_request("Project name cannot be empty"));
                }
                let clash = ctx
                    .db
                    .list_projects(ctx.org_id())
                    .await
                    .map_err(classify_anyhow)?
                    .into_iter()
                    .any(|p| p.project_id != current.project_id && p.name.eq_ignore_ascii_case(&n));
                if clash {
                    return Err(CommandError::conflict(
                        "A project with this name already exists",
                    ));
                }
                Some(n)
            }
            None => None,
        };

        let row = ctx
            .db
            .update_project(
                ctx.org_id(),
                current.project_id,
                UpdateProjectRow {
                    name,
                    description: self.description.map(|d| d.trim().to_string()),
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Project"))?;
        Ok(row.into())
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateProject>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteProject {
    /// Project public ID to delete (must be non-default and empty).
    pub project: String,
}

impl Command for DeleteProject {
    type Output = DeleteProjectResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_project",
            category: "projects",
            description: "Delete a non-default, empty project.",
            method: "DELETE",
            path: "/v1/projects/{project}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("project")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let current = ctx
            .db
            .get_project_by_public_id(ctx.org_id(), &self.project)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Project"))?;

        if !validate_project_public_id(&self.project) {
            return Err(CommandError::not_found("Project"));
        }
        if current.is_default {
            return Err(CommandError::conflict(
                "The default project cannot be deleted",
            ));
        }

        // The DB FK rejects deletion while resources still reference the project.
        let deleted = ctx
            .db
            .delete_project(ctx.org_id(), current.project_id)
            .await
            .map_err(|_| {
                CommandError::conflict("Project is not empty — move or delete its resources first.")
            })?;

        Ok(DeleteProjectResponse { success: deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteProject>() }

#[cfg(test)]
mod tests {
    use crate::domains::common::catalog_entries;

    /// Project commands must be in the command catalog so they surface through
    /// the dispatch table and MCP discover/query/execute (parity with orgs).
    #[test]
    fn project_commands_are_registered_for_mcp() {
        let names: Vec<&str> = catalog_entries().iter().map(|m| m.name).collect();
        for expected in [
            "list_projects",
            "get_project",
            "create_project",
            "update_project",
            "delete_project",
        ] {
            assert!(
                names.contains(&expected),
                "command `{expected}` missing from catalog (not MCP-discoverable)"
            );
        }
    }
}
