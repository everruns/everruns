// Virtual user commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    CreateVirtualUserRequest, CreateVirtualUserRow, UpdateVirtualUser, UpdateVirtualUserRequest,
};
use super::{VIRTUAL_USER_DANGEROUS, VIRTUAL_USER_MANAGE, VIRTUAL_USER_VIEW};
use crate::domains::common::*;
use crate::domains::users::PrincipalService;
use crate::domains::users::record::PrincipalStatus;
use crate::kernel_imports::{VirtualUser, contracts::typed_id::VirtualUserId};
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// CreateVirtualUser
// ============================================================================

/// Create a new virtual user (org-scoped runtime account for a consumer or service).
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateVirtualUser(pub CreateVirtualUserRequest);

impl CommandSchema for CreateVirtualUser {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateVirtualUserRequest>()
    }
}

#[command(
    name = "create_virtual_user",
    category = "virtual_users",
    description = "Create a new virtual user (org-scoped runtime account for a consumer or service).",
    method = "POST",
    path = "/v1/virtual-users",
    policy = VIRTUAL_USER_MANAGE,
)]
impl Command for CreateVirtualUser {
    type Output = VirtualUser;

    async fn execute(self, ctx: &Ctx) -> Result<VirtualUser, CommandError> {
        let req = self.0;

        // Validate locale/timezone
        if let Some(ref locale) = req.locale {
            q::validate_locale(locale)?;
        }
        if let Some(ref tz) = req.timezone {
            q::validate_timezone(tz)?;
        }

        // Persist
        let row = ctx
            .db
            .create_virtual_user(CreateVirtualUserRow {
                usage: req.usage.to_string(),
                org_id: ctx.org_id(),
                id: VirtualUserId::new(),
                name: req.name,
                description: req.description,
                avatar_url: req.avatar_url,
                locale: req.locale,
                timezone: req.timezone,
            })
            .await?;
        let parent = PrincipalService::new(ctx.db.clone())
            .default_owner_principal(&ctx.caller, None)
            .await?;
        PrincipalService::new(ctx.db.clone())
            .ensure_virtual_user_principal(ctx.org_id(), row.id, parent.id)
            .await?;

        q::row_to_identity(&ctx.db, ctx.org_id(), row)
            .await
            .map_err(classify_anyhow)
    }
}

// ============================================================================
// ListVirtualUsers
// ============================================================================

/// List virtual users. Supports search and include_archived.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListVirtualUsers {
    /// Immutable account purpose filter.
    #[schema(example = "service")]
    pub usage: Option<crate::domains::virtual_users::record::VirtualUserUsage>,
    /// Search runtime names and descriptions.
    #[schema(example = "Alex")]
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    /// Include archived runtime accounts.
    #[schema(example = false)]
    pub include_archived: bool,
    /// Zero-based page offset.
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    #[schema(example = 0)]
    pub offset: Option<u32>,
    /// Maximum results per page, capped at the platform pagination limit.
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    #[schema(example = 20)]
    pub limit: Option<u32>,
}

#[command(
    name = "list_virtual_users",
    category = "virtual_users",
    description = "List virtual users by usage and search. Supports pagination (limit/offset) and include_archived.",
    method = "GET",
    path = "/v1/virtual-users",
    policy = VIRTUAL_USER_VIEW,
)]
impl Command for ListVirtualUsers {
    type Output = Paginated<VirtualUser>;

    fn output_schema() -> serde_json::Value {
        paginated_output_schema(output_schema_for::<VirtualUser>())
    }
    fn output_shape() -> &'static str {
        "paginated"
    }
    async fn execute(self, ctx: &Ctx) -> Result<Paginated<VirtualUser>, CommandError> {
        let pg = pagination(self.offset, self.limit);
        let usage = self.usage.map(|u| u.to_string());
        let (rows, total) = ctx
            .db
            .list_virtual_users(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived,
                usage.as_deref(),
                pg,
            )
            .await?;
        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            data.push(q::row_to_identity(&ctx.db, ctx.org_id(), row).await?);
        }
        Ok(Paginated {
            data,
            total,
            offset: pg.offset,
            limit: pg.limit,
        })
    }
}

// ============================================================================
// GetVirtualUser
// ============================================================================

/// Get a single virtual user by ID.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetVirtualUser {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_virtual_user",
    category = "virtual_users",
    description = "Get a single virtual user by ID.",
    method = "GET",
    path = "/v1/virtual-users/{identity_id}",
    policy = VIRTUAL_USER_VIEW,
    positional = "id",
)]
impl Command for GetVirtualUser {
    type Output = VirtualUser;

    async fn execute(self, ctx: &Ctx) -> Result<VirtualUser, CommandError> {
        let identity_id: VirtualUserId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        q::get_by_id(&ctx.db, ctx.org_id(), identity_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Virtual user"))
    }
}

// ============================================================================
// UpdateVirtualUser
// ============================================================================

/// Update a virtual user. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateVirtualUserCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateVirtualUserRequest,
}

#[command(
    name = "update_virtual_user",
    category = "virtual_users",
    description = "Update a virtual user. Only provided fields are changed.",
    method = "PATCH",
    path = "/v1/virtual-users/{identity_id}",
    policy = VIRTUAL_USER_MANAGE,
    positional = "id",
)]
impl Command for UpdateVirtualUserCmd {
    type Output = VirtualUser;

    async fn execute(self, ctx: &Ctx) -> Result<VirtualUser, CommandError> {
        let identity_id: VirtualUserId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        let req = self.req;

        // Validate locale/timezone if being set
        if let crate::storage::UpdateField::Set(ref locale) = req.locale {
            q::validate_locale(locale)?;
        }
        if let crate::storage::UpdateField::Set(ref tz) = req.timezone {
            q::validate_timezone(tz)?;
        }

        // Persist
        let row = ctx
            .db
            .update_virtual_user(
                ctx.org_id(),
                identity_id,
                UpdateVirtualUser {
                    name: req.name,
                    description: req.description,
                    avatar_url: req.avatar_url,
                    locale: req.locale,
                    timezone: req.timezone,
                    status: req.status.map(|s| s.to_string()),
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Virtual user"))?;
        let principal_service = PrincipalService::new(ctx.db.clone());
        let parent = match ctx
            .db
            .get_principal_by_subject(ctx.org_id(), "virtual_user", row.id.uuid())
            .await?
            .and_then(|principal| principal.parent_principal_id)
        {
            Some(parent_id) => parent_id,
            None => {
                principal_service
                    .default_owner_principal(&ctx.caller, None)
                    .await?
                    .id
            }
        };
        principal_service
            .ensure_virtual_user_principal(ctx.org_id(), row.id, parent)
            .await?;

        q::row_to_identity(&ctx.db, ctx.org_id(), row)
            .await
            .map_err(classify_anyhow)
    }
}

async fn ensure_no_live_references(
    ctx: &Ctx,
    identity_id: VirtualUserId,
) -> Result<(), CommandError> {
    crate::domains::apps::queries::ensure_no_app_references_to_virtual_user(
        &ctx.db,
        ctx.org_id(),
        identity_id.uuid(),
    )
    .await?;

    if ctx
        .db
        .has_agent_with_identity(ctx.org_id(), identity_id)
        .await?
    {
        return Err(CommandError::conflict(
            "Cannot archive or delete virtual user while agents still reference it",
        ));
    }

    Ok(())
}

// ============================================================================
// DeleteVirtualUser
// ============================================================================

/// Archive a virtual user (soft delete).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteVirtualUser {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "delete_virtual_user",
    category = "virtual_users",
    description = "Archive a virtual user (soft delete). Can be restored.",
    method = "DELETE",
    path = "/v1/virtual-users/{identity_id}",
    policy = VIRTUAL_USER_MANAGE,
    positional = "id",
)]
impl Command for DeleteVirtualUser {
    type Output = serde_json::Value;

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let identity_id: VirtualUserId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        ensure_no_live_references(ctx, identity_id).await?;

        let deleted = ctx
            .db
            .delete_virtual_user(ctx.org_id(), identity_id)
            .await?;

        if deleted {
            PrincipalService::new(ctx.db.clone())
                .sync_virtual_user_status(ctx.org_id(), identity_id, PrincipalStatus::Archived)
                .await?;
            Ok(serde_json::json!({"deleted": true}))
        } else {
            Err(CommandError::not_found("Virtual user"))
        }
    }
}

// ============================================================================
// DestroyVirtualUser (hard delete)
// ============================================================================

/// Permanently delete a virtual user.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DestroyVirtualUser {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "destroy_virtual_user",
    category = "virtual_users",
    description = "Permanently delete a virtual user.",
    method = "POST",
    path = "/v1/virtual-users/{identity_id}/delete",
    policy = VIRTUAL_USER_DANGEROUS,
    positional = "id",
    cli = CliRoute::new(&["virtual-users"], "destroy").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Permanently remove a virtual user", "everruns virtual-users destroy vu_01h9 --reason 'Integration decommissioned'",)]),
)]
impl Command for DestroyVirtualUser {
    type Output = serde_json::Value;

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let identity_id: VirtualUserId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        ensure_no_live_references(ctx, identity_id).await?;

        let destroyed = ctx
            .db
            .destroy_virtual_user(ctx.org_id(), identity_id)
            .await?;

        if destroyed {
            PrincipalService::new(ctx.db.clone())
                .sync_virtual_user_status(ctx.org_id(), identity_id, PrincipalStatus::Deleted)
                .await?;
            Ok(serde_json::json!({"destroyed": true}))
        } else {
            Err(CommandError::not_found("Virtual user"))
        }
    }
}
