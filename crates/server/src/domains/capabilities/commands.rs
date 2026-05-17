// Capability commands — user-facing read-only operations.
//
// Capabilities are a bounded registry (~30-50 items). No policy checks
// needed — these are public read endpoints.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    CapabilityInfo, CreateDeclarativeCapabilityRequest, CreateDeclarativeCapabilityRow,
    DeclarativeCapability, UpdateDeclarativeCapability, UpdateDeclarativeCapabilityRequest,
};
use super::{CAPABILITY_DANGEROUS, CAPABILITY_MANAGE, CAPABILITY_VIEW};
use crate::domains::common::*;
use everruns_core::{
    CapabilityId, DeclarativeCapabilityDefinition, DeclarativeCapabilityId, Policy,
    validate_declarative_capability_definition,
};
use serde::Deserialize;
use utoipa::ToSchema;

// Capabilities are a bounded set (~30-50 items), so default to showing all.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 200;

// ============================================================================
// ListCapabilities
// ============================================================================

/// List available capabilities with optional search and pagination.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListCapabilities {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl Command for ListCapabilities {
    type Output = Paginated<CapabilityInfo>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_capabilities",
            category: "capabilities",
            description: "List available capabilities. Use search for name/description filtering. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/capabilities",
        }
    }

    fn output_schema() -> serde_json::Value {
        paginated_output_schema(output_schema_for::<CapabilityInfo>())
    }

    fn output_shape() -> &'static str {
        "paginated"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Paginated<CapabilityInfo>, CommandError> {
        let mut capabilities = ctx
            .capability_service
            .list_all(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;

        if let Some(ref search) = self.search {
            q::filter_by_search(&mut capabilities, search);
        }

        let total = capabilities.len() as u32;
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        let data: Vec<CapabilityInfo> = capabilities
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();

        Ok(Paginated {
            data,
            total,
            offset,
            limit,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListCapabilities>() }

// ============================================================================
// GetCapability
// ============================================================================

/// Get a specific capability by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetCapability {
    pub id: String,
}

impl Command for GetCapability {
    type Output = CapabilityInfo;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_capability",
            category: "capabilities",
            description: "Get a specific capability by ID.",
            method: "GET",
            path: "/v1/capabilities/{id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<CapabilityInfo, CommandError> {
        let cap_id = CapabilityId::new(&self.id);

        ctx.capability_service
            .get(ctx.org_id(), &cap_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Capability"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetCapability>() }

// ============================================================================
// Declarative Capability CRUD
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateDeclarativeCapability(pub CreateDeclarativeCapabilityRequest);

impl CommandSchema for CreateDeclarativeCapability {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateDeclarativeCapabilityRequest>()
    }
}

impl Command for CreateDeclarativeCapability {
    type Output = DeclarativeCapability;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_declarative_capability",
            category: "capabilities",
            description: "Create a persisted declarative capability.",
            method: "POST",
            path: "/v1/capabilities",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeclarativeCapability, CommandError> {
        let definition = normalize_definition(self.0.definition)?;
        validate_definition(&definition)?;

        if ctx
            .db
            .get_declarative_capability_by_name(ctx.org_id(), &definition.name)
            .await
            .map_err(classify_anyhow)?
            .is_some()
        {
            return Err(CommandError::conflict(format!(
                "Declarative capability with name '{}' already exists",
                definition.name
            )));
        }

        let row = ctx
            .db
            .create_declarative_capability(
                ctx.org_id(),
                CreateDeclarativeCapabilityRow {
                    public_id: DeclarativeCapabilityId::new().to_string(),
                    name: definition.name.clone(),
                    display_name: definition.display_name.clone(),
                    description: definition.description.clone(),
                    definition: serde_json::to_value(&definition)
                        .map_err(|error| CommandError::internal(error.into()))?,
                },
            )
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_declarative_capability(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateDeclarativeCapability>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListDeclarativeCapabilities {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListDeclarativeCapabilities {
    type Output = Vec<DeclarativeCapability>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_declarative_capabilities",
            category: "capabilities",
            description: "List persisted declarative capabilities.",
            method: "GET",
            path: "/v1/capabilities/declarative",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<DeclarativeCapability>, CommandError> {
        let rows = ctx
            .db
            .list_declarative_capabilities(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived,
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_declarative_capability).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListDeclarativeCapabilities>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetDeclarativeCapability {
    pub id: String,
}

impl Command for GetDeclarativeCapability {
    type Output = DeclarativeCapability;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_declarative_capability",
            category: "capabilities",
            description: "Get a persisted declarative capability.",
            method: "GET",
            path: "/v1/capabilities/declarative/{id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeclarativeCapability, CommandError> {
        let row = get_declarative_capability_by_public_id(ctx, &self.id).await?;
        Ok(q::row_to_declarative_capability(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetDeclarativeCapability>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDeclarativeCapabilityCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateDeclarativeCapabilityRequest,
}

impl Command for UpdateDeclarativeCapabilityCmd {
    type Output = DeclarativeCapability;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_declarative_capability",
            category: "capabilities",
            description: "Update a persisted declarative capability.",
            method: "PATCH",
            path: "/v1/capabilities/declarative/{id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeclarativeCapability, CommandError> {
        let existing = get_declarative_capability_by_public_id(ctx, &self.id).await?;
        if matches!(self.req.status.as_deref(), Some("deleted")) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

        let mut input = UpdateDeclarativeCapability::default();
        if let Some(definition) = self.req.definition {
            let definition = normalize_definition(definition)?;
            validate_definition(&definition)?;
            if definition.name != existing.name
                && ctx
                    .db
                    .get_declarative_capability_by_name(ctx.org_id(), &definition.name)
                    .await
                    .map_err(classify_anyhow)?
                    .is_some()
            {
                return Err(CommandError::conflict(format!(
                    "Declarative capability with name '{}' already exists",
                    definition.name
                )));
            }
            input.name = Some(definition.name.clone());
            input.display_name = definition.display_name.clone();
            input.description = Some(definition.description.clone());
            input.definition = Some(
                serde_json::to_value(&definition)
                    .map_err(|error| CommandError::internal(error.into()))?,
            );
        }
        if let Some(status) = self.req.status {
            validate_status(&status)?;
            input.status = Some(status);
        }

        let row = ctx
            .db
            .update_declarative_capability(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Declarative capability"))?;
        Ok(q::row_to_declarative_capability(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateDeclarativeCapabilityCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteDeclarativeCapability {
    pub id: String,
}

impl Command for DeleteDeclarativeCapability {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_declarative_capability",
            category: "capabilities",
            description: "Archive a persisted declarative capability.",
            method: "DELETE",
            path: "/v1/capabilities/declarative/{id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let row = get_declarative_capability_by_public_id(ctx, &self.id).await?;
        let deleted = ctx
            .db
            .delete_declarative_capability(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Declarative capability"));
        }
        Ok(serde_json::json!({ "deleted": true }))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteDeclarativeCapability>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyDeclarativeCapability {
    pub id: String,
}

impl Command for DestroyDeclarativeCapability {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_declarative_capability",
            category: "capabilities",
            description: "Permanently delete an archived declarative capability.",
            method: "POST",
            path: "/v1/capabilities/declarative/{id}/delete",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let row = get_declarative_capability_by_public_id(ctx, &self.id).await?;
        let deleted = ctx
            .db
            .destroy_declarative_capability(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Declarative capability"));
        }
        Ok(serde_json::json!({ "destroyed": true }))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyDeclarativeCapability>() }

fn normalize_definition(
    mut definition: DeclarativeCapabilityDefinition,
) -> Result<DeclarativeCapabilityDefinition, CommandError> {
    if definition.icon.is_none() {
        definition.icon = Some("puzzle".to_string());
    }
    if definition.category.is_none() {
        definition.category = Some("Declarative".to_string());
    }
    Ok(definition)
}

fn validate_definition(definition: &DeclarativeCapabilityDefinition) -> Result<(), CommandError> {
    validate_declarative_capability_definition(definition)
        .map_err(|message| CommandError::bad_request(format!("Invalid definition: {message}")))?;
    if let Some(servers) = &definition.mcp_servers {
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(servers)
            .map_err(|error| CommandError::bad_request(format!("Invalid MCP servers: {error}")))?;
    }
    Ok(())
}

fn validate_status(status: &str) -> Result<(), CommandError> {
    match status {
        "active" | "disabled" | "archived" => Ok(()),
        _ => Err(CommandError::bad_request(
            "status must be active, disabled, or archived",
        )),
    }
}

fn parse_declarative_public_id(id: &str) -> Result<DeclarativeCapabilityId, CommandError> {
    id.parse::<DeclarativeCapabilityId>().map_err(|error| {
        CommandError::bad_request(format!("Invalid declarative capability ID: {error}"))
    })
}

async fn get_declarative_capability_by_public_id(
    ctx: &Ctx,
    id: &str,
) -> Result<crate::storage::models::DeclarativeCapabilityRow, CommandError> {
    let public_id = parse_declarative_public_id(id)?;
    ctx.db
        .get_declarative_capability_by_public_id(ctx.org_id(), &public_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Declarative capability"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Regression for EVE-324: bashkit's flag parser emits string values when
    // the tool schema declares no per-property types (as inventory commands
    // currently do). Before the lenient deserializer, `--limit 5` deserialized
    // as `{"limit": "5"}` and failed with "invalid type: string".
    #[test]
    fn list_capabilities_accepts_string_numeric_limit() {
        let cmd: ListCapabilities =
            serde_json::from_value(json!({ "limit": "5", "offset": "10" })).unwrap();
        assert_eq!(cmd.limit, Some(5));
        assert_eq!(cmd.offset, Some(10));
    }

    #[test]
    fn list_capabilities_accepts_native_numeric_limit() {
        let cmd: ListCapabilities =
            serde_json::from_value(json!({ "limit": 5, "offset": 10 })).unwrap();
        assert_eq!(cmd.limit, Some(5));
        assert_eq!(cmd.offset, Some(10));
    }
}
