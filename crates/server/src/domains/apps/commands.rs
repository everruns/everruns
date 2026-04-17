// App commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    AddChannelRequest, CreateAppChannelRow, CreateAppRequest, CreateAppRow, UpdateApp,
    UpdateAppChannel, UpdateAppRequest, UpdateChannelRequest,
};
use super::{APP_DANGEROUS, APP_MANAGE, APP_VIEW};
use crate::domains::common::*;
use crate::errors::ResourceNotFoundError;
use chrono::Utc;
use everruns_core::typed_id::{AgentId, AppChannelId, HarnessId};
use everruns_core::{App, AppChannel, AppId, Policy};
use everruns_durable::UpdateField;
use serde::Deserialize;
use uuid::Uuid;

// ============================================================================
// Validation helpers
// ============================================================================

async fn validate_harness(ctx: &Ctx, harness_id: HarnessId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_harness(ctx.org_id(), harness_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Harness").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted harnesses cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

async fn validate_agent(ctx: &Ctx, agent_id: &AgentId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Agent").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agents cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

async fn validate_agent_identity(
    ctx: &Ctx,
    identity_id: everruns_core::typed_id::AgentIdentityId,
) -> Result<Uuid, CommandError> {
    let identity = ctx
        .db
        .get_agent_identity(ctx.org_id(), identity_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent identity"))?;
    if identity.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agent identities cannot be assigned"
        )));
    }
    Ok(identity.id.uuid())
}

// ============================================================================
// CreateApp
// ============================================================================

/// Create a new app with an agent, harness, and initial channel.
#[derive(Debug, Deserialize)]
pub struct CreateApp(pub CreateAppRequest);

impl Command for CreateApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_app",
            category: "apps",
            description: "Create a new app with an agent, harness, and initial channel.",
            method: "POST",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let req = self.0;
        let encryption = ctx.encryption.as_ref();

        // Validate references
        let harness_uuid = validate_harness(ctx, req.harness_id).await?;
        let agent_uuid = validate_agent(ctx, &req.agent_id).await?;
        let agent_identity_uuid = if let Some(identity_id) = req.agent_identity_id {
            Some(validate_agent_identity(ctx, identity_id).await?)
        } else {
            None
        };

        // Prepare channel config
        let channel_config = req.channel_config.clone().unwrap_or_default();
        let (stored_plaintext, channel_config_encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        // Persist app
        let internal_uuid = Uuid::now_v7();
        let public_id = AppId::from_uuid(internal_uuid);
        let input = CreateAppRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            harness_id: harness_uuid,
            agent_id: agent_uuid,
            agent_identity_id: agent_identity_uuid,
            channel_type: req.channel_type.to_string(),
            channel_config: stored_plaintext.clone(),
            channel_config_encrypted: channel_config_encrypted.clone(),
        };
        let row = ctx
            .db
            .create_app(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;

        // Create the initial channel
        let channel_uuid = Uuid::now_v7();
        let channel_public_id = AppChannelId::from_uuid(channel_uuid);
        let channel_input = CreateAppChannelRow {
            public_id: channel_public_id.to_string(),
            channel_type: req.channel_type.to_string(),
            channel_config: stored_plaintext,
            channel_config_encrypted,
            enabled: true,
        };
        ctx.db
            .create_app_channel(row.id, channel_input)
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateApp>() }

// ============================================================================
// ListApps
// ============================================================================

/// List apps. Supports search and include_archived.
#[derive(Debug, Deserialize)]
pub struct ListApps {
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

impl Command for ListApps {
    type Output = Vec<App>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_apps",
            category: "apps",
            description: "List all apps. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<App>, CommandError> {
        let encryption = ctx.encryption.as_ref();
        let rows = ctx
            .db
            .list_apps(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        q::load_apps_list(&ctx.db, encryption, rows, ctx.org_id())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListApps>() }

// ============================================================================
// GetApp
// ============================================================================

/// Get a single app by ID.
#[derive(Debug, Deserialize)]
pub struct GetApp {
    pub id: String,
}

impl Command for GetApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_app",
            category: "apps",
            description: "Get a single app by ID.",
            method: "GET",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        q::get_by_public_id(&ctx.db, encryption, ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetApp>() }

// ============================================================================
// UpdateApp
// ============================================================================

/// Update an app. Only provided fields are changed.
#[derive(Debug, Deserialize)]
pub struct UpdateAppCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAppRequest,
}

impl Command for UpdateAppCmd {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app",
            category: "apps",
            description: "Update an app. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let req = self.req;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(existing.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        // Validate optional references
        let harness_id = if let Some(hid) = req.harness_id {
            Some(validate_harness(ctx, hid).await?)
        } else {
            None
        };

        let agent_id = if let Some(aid) = req.agent_id {
            Some(validate_agent(ctx, &aid).await?)
        } else {
            None
        };

        let agent_identity_id = match req.agent_identity_id {
            UpdateField::Set(identity_id) => {
                let uuid = validate_agent_identity(ctx, identity_id).await?;
                UpdateField::Set(uuid)
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };

        let input = UpdateApp {
            name: req.name,
            description: req.description,
            harness_id,
            agent_id,
            agent_identity_id,
            channel_type: None,
            channel_config: None,
            channel_config_encrypted: None,
            status: req.status.map(|s| s.to_string()),
            published_at: UpdateField::Unchanged,
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        Ok(q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAppCmd>() }

// ============================================================================
// DeleteApp
// ============================================================================

/// Archive an app (soft delete).
#[derive(Debug, Deserialize)]
pub struct DeleteApp {
    pub id: String,
}

impl Command for DeleteApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app",
            category: "apps",
            description: "Archive an app (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        ctx.db
            .delete_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteApp>() }

// ============================================================================
// DestroyApp (hard delete)
// ============================================================================

/// Permanently delete an archived app.
#[derive(Debug, Deserialize)]
pub struct DestroyApp {
    pub id: String,
}

impl Command for DestroyApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_app",
            category: "apps",
            description: "Permanently delete an archived app.",
            method: "POST",
            path: "/v1/apps/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "App must be archived before deletion",
            ));
        }

        ctx.db
            .destroy_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyApp>() }

// ============================================================================
// PublishApp
// ============================================================================

/// Publish an app (start accepting requests).
#[derive(Debug, Deserialize)]
pub struct PublishApp {
    pub id: String,
}

impl Command for PublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "publish_app",
            category: "apps",
            description: "Publish an app (start accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/publish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let input = UpdateApp {
            status: Some("published".to_string()),
            published_at: UpdateField::Set(Utc::now()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        Ok(q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await)
    }
}

inventory::submit! { CommandDescriptor::of::<PublishApp>() }

// ============================================================================
// UnpublishApp
// ============================================================================

/// Unpublish an app (stop accepting requests).
#[derive(Debug, Deserialize)]
pub struct UnpublishApp {
    pub id: String,
}

impl Command for UnpublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpublish_app",
            category: "apps",
            description: "Unpublish an app (stop accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/unpublish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let input = UpdateApp {
            status: Some("draft".to_string()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        Ok(q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await)
    }
}

inventory::submit! { CommandDescriptor::of::<UnpublishApp>() }

// ============================================================================
// AddChannel
// ============================================================================

/// Add a channel to an app.
#[derive(Debug, Deserialize)]
pub struct AddChannel {
    pub app_id: String,
    #[serde(flatten)]
    pub req: AddChannelRequest,
}

impl Command for AddChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_app_channel",
            category: "apps",
            description: "Add a channel to an app.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_config = self.req.channel_config.unwrap_or_default();
        let (stored_plaintext, encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        let channel_uuid = Uuid::now_v7();
        let channel_public_id = AppChannelId::from_uuid(channel_uuid);
        let input = CreateAppChannelRow {
            public_id: channel_public_id.to_string(),
            channel_type: self.req.channel_type.to_string(),
            channel_config: stored_plaintext,
            channel_config_encrypted: encrypted,
            enabled: self.req.enabled.unwrap_or(true),
        };

        let row = ctx
            .db
            .create_app_channel(app.id, input)
            .await
            .map_err(classify_anyhow)?;

        Ok(q::channel_row_to_channel(encryption, row))
    }
}

inventory::submit! { CommandDescriptor::of::<AddChannel>() }

// ============================================================================
// UpdateChannel
// ============================================================================

/// Update a channel on an app.
#[derive(Debug, Deserialize)]
pub struct UpdateChannelCmd {
    pub app_id: String,
    pub channel_id: String,
    #[serde(flatten)]
    pub req: UpdateChannelRequest,
}

impl Command for UpdateChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app_channel",
            category: "apps",
            description: "Update a channel on an app.",
            method: "PATCH",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        let (channel_config, channel_config_encrypted) =
            if let Some(config) = self.req.channel_config {
                let (stored, encrypted) =
                    q::prepare_channel_config(encryption, &config).map_err(classify_anyhow)?;
                (Some(stored), encrypted)
            } else {
                (None, None)
            };

        let input = UpdateAppChannel {
            channel_type: self.req.channel_type.map(|ct| ct.to_string()),
            channel_config,
            channel_config_encrypted,
            enabled: self.req.enabled,
        };

        let row = ctx
            .db
            .update_app_channel(channel_row.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        Ok(q::channel_row_to_channel(encryption, row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateChannelCmd>() }

// ============================================================================
// DeleteChannel
// ============================================================================

/// Remove a channel from an app.
#[derive(Debug, Deserialize)]
pub struct DeleteChannel {
    pub app_id: String,
    pub channel_id: String,
}

impl Command for DeleteChannel {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app_channel",
            category: "apps",
            description: "Remove a channel from an app.",
            method: "DELETE",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        ctx.db
            .delete_app_channel(channel_row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteChannel>() }
