// Capability commands — user-facing read-only operations.
//
// Capabilities are a bounded registry (~30-50 items). No policy checks
// needed — these are public read endpoints.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    CapabilityInfo, CreateDeclarativeCapabilityRequest, CreateDeclarativeCapabilityRow,
    DeclarativeCapability, GuardrailExample, GuardrailExamplesResponse, GuardrailsDryRunHit,
    GuardrailsDryRunRequest, GuardrailsDryRunResponse, UpdateDeclarativeCapability,
    UpdateDeclarativeCapabilityRequest,
};
use super::{CAPABILITY_DANGEROUS, CAPABILITY_MANAGE, CAPABILITY_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    CapabilityId, DeclarativeCapabilityDefinition, GuardrailsConfig, Policy,
    everruns_provider::typed_id::DeclarativeCapabilityId,
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
    /// Zero-based offset into the result set.
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Maximum number of items returned in this page.
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

        capabilities.retain(|capability| {
            ctx.feature_flags
                .is_capability_enabled(capability.id.as_str())
        });

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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

        if let Some(flag) =
            everruns_platform::FeatureFlags::required_for_capability(cap_id.as_str())
            && !ctx.feature_flags.is_enabled(flag)
        {
            return Err(CommandError::feature_not_enabled(flag));
        }

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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

// ============================================================================
// DryRunGuardrails
// ============================================================================

/// TM-DOS: bound dry-run input so check evaluation stays cheap.
const MAX_DRY_RUN_TEXT_BYTES: usize = 64 * 1024;

/// Evaluate a guardrails capability config against sample content without a
/// session. This is how checks are tuned (especially in advisory mode)
/// before being attached to an agent. Pure computation — nothing persisted.
#[derive(Debug, Deserialize)]
pub struct DryRunGuardrails(pub GuardrailsDryRunRequest);

impl CommandSchema for DryRunGuardrails {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<GuardrailsDryRunRequest>()
    }
}

impl Command for DryRunGuardrails {
    type Output = GuardrailsDryRunResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "dry_run_guardrails",
            category: "capabilities",
            description: "Evaluate a guardrails capability config against sample text without a session. Returns triggered checks and whether the content would be blocked.",
            method: "POST",
            path: "/v1/capabilities/guardrails/dry-run",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_VIEW)
    }

    async fn execute(self, _ctx: &Ctx) -> Result<GuardrailsDryRunResponse, CommandError> {
        let req = self.0;
        // TM-DOS-017: bound input and let compile enforce regex/entry limits
        // so an authenticated caller cannot wedge the request with a
        // pathological pattern or oversized config.
        if req.text.len() > MAX_DRY_RUN_TEXT_BYTES {
            return Err(CommandError::bad_request(format!(
                "text exceeds {MAX_DRY_RUN_TEXT_BYTES} bytes"
            )));
        }
        let compiled = GuardrailsConfig::from_value(&req.config)
            .and_then(|config| config.compile())
            .map_err(CommandError::bad_request)?;
        let hits: Vec<GuardrailsDryRunHit> = compiled
            .evaluate(req.stage, &req.text, req.tool_name.as_deref(), &|_| false)
            .into_iter()
            .map(|hit| GuardrailsDryRunHit {
                check_index: hit.check_index as u32,
                check_id: hit.check_label,
                stage: hit.stage,
                rule_type: hit.rule_type.to_string(),
                action: hit.action,
                reason_code: hit.reason_code,
                replacement: hit.replacement,
                matched: hit.matched,
            })
            .collect();
        let blocked = hits
            .iter()
            .any(|hit| hit.action == everruns_core::GuardrailAction::Block);
        Ok(GuardrailsDryRunResponse { hits, blocked })
    }
}

inventory::submit! { CommandDescriptor::of::<DryRunGuardrails>() }

// ============================================================================
// ListGuardrailExamples
// ============================================================================

/// List the read-only guardrail gallery: ready-made `GuardrailsConfig`
/// presets an author can adopt into an agent's `guardrails` capability config.
/// Pure computation over a static catalogue — nothing persisted, no I/O.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct ListGuardrailExamples {}

impl Command for ListGuardrailExamples {
    type Output = GuardrailExamplesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_guardrail_examples",
            category: "capabilities",
            description: "List adoptable guardrail presets (the guardrail gallery), each with its config and trust metadata (check-type composition, stages, data egress).",
            method: "GET",
            path: "/v1/capabilities/guardrails/examples",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&CAPABILITY_VIEW)
    }

    async fn execute(self, _ctx: &Ctx) -> Result<GuardrailExamplesResponse, CommandError> {
        let mut examples = Vec::new();
        for item in everruns_core::guardrail_gallery() {
            let config = serde_json::to_value(&item.config).map_err(|e| {
                CommandError::internal(anyhow::anyhow!(
                    "gallery preset '{}' failed to serialize: {e}",
                    item.name
                ))
            })?;
            examples.push(GuardrailExample {
                name: item.name.to_string(),
                display_name: item.display_name.to_string(),
                description: item.description.to_string(),
                tags: item.tags.iter().map(|t| t.to_string()).collect(),
                check_types: item.check_types().iter().map(|t| t.to_string()).collect(),
                stages: item.stages().iter().map(|s| s.to_string()).collect(),
                data_egress: item.data_egress().as_str().to_string(),
                config,
            });
        }
        Ok(GuardrailExamplesResponse { examples })
    }
}

inventory::submit! { CommandDescriptor::of::<ListGuardrailExamples>() }

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

    #[test]
    fn dry_run_guardrails_request_deserializes() {
        let cmd: DryRunGuardrails = serde_json::from_value(json!({
            "config": {
                "checks": [
                    {"stage": "output", "type": "blocklist", "words": ["x"]}
                ]
            },
            "stage": "output",
            "text": "x marks the spot"
        }))
        .unwrap();
        assert_eq!(cmd.0.stage, everruns_core::GuardrailStage::Output);
        assert_eq!(cmd.0.text, "x marks the spot");
        assert!(cmd.0.tool_name.is_none());
        // Sanity: the embedded config compiles and matches via the core engine.
        let compiled = GuardrailsConfig::from_value(&cmd.0.config)
            .unwrap()
            .compile()
            .unwrap();
        let hits = compiled.evaluate(cmd.0.stage, &cmd.0.text, None, &|_| false);
        assert_eq!(hits.len(), 1);
    }
}
