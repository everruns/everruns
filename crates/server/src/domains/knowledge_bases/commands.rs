use super::types::{
    CreateKnowledgeBaseRequest, CreateKnowledgeBaseRow, CreateKnowledgeEntryRequest,
    CreateKnowledgeEntryRow, KnowledgeBaseResponse, KnowledgeEntryResponse,
    ListKnowledgeBasesQuery, ListKnowledgeEntriesQuery, UpdateKnowledgeBase,
    UpdateKnowledgeBaseRequest, UpdateKnowledgeEntry, UpdateKnowledgeEntryRequest,
    knowledge_base_response, knowledge_entry_response,
};
use super::{
    ENTRY_KINDS, KNOWLEDGE_BASE_MANAGE, KNOWLEDGE_BASE_VIEW, MAX_ENTRY_BODY_BYTES, MAX_ENTRY_TAGS,
};
use crate::domains::common::*;
use crate::kernel_imports::{
    Policy, everruns_provider::driver_registry::ServiceKind, everruns_provider::provider::DriverId,
};
use everruns_durable::UpdateField;
use everruns_provider::typed_id::{KnowledgeBaseId, KnowledgeEntryId};
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request(
            "Knowledge base name cannot be empty",
        ));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Knowledge base name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_kb_id(kb_id: &str) -> Result<KnowledgeBaseId, CommandError> {
    kb_id
        .parse()
        .map_err(|_| CommandError::not_found("KnowledgeBase"))
}

fn parse_entry_id(entry_id: &str) -> Result<KnowledgeEntryId, CommandError> {
    entry_id
        .parse()
        .map_err(|_| CommandError::not_found("KnowledgeEntry"))
}

fn validate_kind(kind: Option<&str>) -> Result<Option<String>, CommandError> {
    let Some(kind) = kind else {
        return Ok(None);
    };
    let kind = kind.trim();
    if !ENTRY_KINDS.contains(&kind) {
        return Err(CommandError::bad_request(format!(
            "Invalid kind '{}'; must be one of: {}",
            kind,
            ENTRY_KINDS.join(", ")
        )));
    }
    Ok(Some(kind.to_string()))
}

fn validate_tags(tags: Option<Vec<String>>) -> Result<Option<Vec<String>>, CommandError> {
    let Some(tags) = tags else { return Ok(None) };
    if tags.len() > MAX_ENTRY_TAGS {
        return Err(CommandError::bad_request(format!(
            "Entry tags must be at most {MAX_ENTRY_TAGS}"
        )));
    }
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let trimmed = tag.trim().to_lowercase();
        if trimmed.is_empty() {
            return Err(CommandError::bad_request("Tags cannot be empty"));
        }
        if trimmed.chars().count() > 64 {
            return Err(CommandError::bad_request(
                "Tags must be at most 64 characters",
            ));
        }
        if !out.iter().any(|t| t == &trimmed) {
            out.push(trimmed);
        }
    }
    Ok(Some(out))
}

fn validate_title(title: &str) -> Result<String, CommandError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request("Entry title cannot be empty"));
    }
    if trimmed.chars().count() > 512 {
        return Err(CommandError::bad_request(
            "Entry title must be at most 512 characters",
        ));
    }
    Ok(trimmed.to_string())
}

/// Max length of an OKF `resource` URI. Bounds storage and response size; OKF
/// does not constrain the value, so we only guard against pathological input.
const MAX_RESOURCE_LEN: usize = 2048;

fn validate_resource(resource: Option<String>) -> Result<Option<String>, CommandError> {
    let Some(resource) = resource else {
        return Ok(None);
    };
    let trimmed = resource.trim();
    if trimmed.is_empty() {
        // Treat an empty/whitespace resource as "not set" rather than storing "".
        return Ok(None);
    }
    if trimmed.len() > MAX_RESOURCE_LEN {
        return Err(CommandError::bad_request(format!(
            "Entry resource must be at most {MAX_RESOURCE_LEN} characters"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_body(body: &str) -> Result<String, CommandError> {
    if body.len() > MAX_ENTRY_BODY_BYTES {
        return Err(CommandError::bad_request(format!(
            "Entry body must be at most {MAX_ENTRY_BODY_BYTES} bytes"
        )));
    }
    Ok(body.to_string())
}

async fn validate_embedding_model_id(
    ctx: &Ctx,
    model_id: everruns_provider::typed_id::ModelId,
) -> Result<everruns_provider::typed_id::ModelId, CommandError> {
    let model = ctx
        .db
        .get_model(ctx.org_id(), model_id.uuid())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::bad_request("Embedding model not found"))?;
    let provider = ctx
        .db
        .get_provider(ctx.org_id(), model.provider_id.uuid())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::bad_request("Embedding provider not found"))?;
    let provider_type: DriverId = provider
        .provider_type
        .parse()
        .expect("DriverId::from_str is infallible");
    let registry = everruns_worker::create_driver_registry();
    if !registry.supports(&provider_type, ServiceKind::Embeddings) {
        return Err(CommandError::bad_request(format!(
            "Embedding model provider '{}' does not support embeddings",
            provider.provider_type
        )));
    }
    Ok(model_id)
}

// ============================================
// Knowledge Base CRUD
// ============================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListKnowledgeBases {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListKnowledgeBasesQuery> for ListKnowledgeBases {
    fn from(query: ListKnowledgeBasesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: query.include_archived,
        }
    }
}

impl Command for ListKnowledgeBases {
    type Output = Vec<KnowledgeBaseResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_knowledge_bases",
            category: "knowledge_bases",
            description: "List knowledge bases in the current organization.",
            method: "GET",
            path: "/v1/knowledge-bases",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<KnowledgeBaseResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<KnowledgeBaseResponse>, CommandError> {
        let rows = ctx
            .db
            .list_knowledge_bases(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await
            .map_err(classify_anyhow)?;
        rows.into_iter()
            .map(|row| knowledge_base_response(row).map_err(classify_anyhow))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListKnowledgeBases>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateKnowledgeBase {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    /// Optional embedding model for hybrid retrieval.
    pub embedding_model_id: Option<everruns_provider::typed_id::ModelId>,
}

impl From<CreateKnowledgeBaseRequest> for CreateKnowledgeBase {
    fn from(request: CreateKnowledgeBaseRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            embedding_model_id: request.embedding_model_id,
        }
    }
}

impl Command for CreateKnowledgeBase {
    type Output = KnowledgeBaseResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_knowledge_base",
            category: "knowledge_bases",
            description: "Create a knowledge base in the current organization.",
            method: "POST",
            path: "/v1/knowledge-bases",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeBaseResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeBaseResponse, CommandError> {
        let name = validate_name(&self.name)?;
        let embedding_model_id = if let Some(model_id) = self.embedding_model_id {
            Some(validate_embedding_model_id(ctx, model_id).await?)
        } else {
            None
        };
        let input = CreateKnowledgeBaseRow {
            public_id: KnowledgeBaseId::new().to_string(),
            name,
            description: self.description,
            owner_principal_id: None,
            resolved_owner_user_id: ctx.caller.user_id,
            embedding_model_id,
        };
        let row = ctx
            .db
            .create_knowledge_base(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        knowledge_base_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateKnowledgeBase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetKnowledgeBase {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
}

impl Command for GetKnowledgeBase {
    type Output = KnowledgeBaseResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_knowledge_base",
            category: "knowledge_bases",
            description: "Get a knowledge base by ID.",
            method: "GET",
            path: "/v1/knowledge-bases/{kb_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("kb_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeBaseResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeBaseResponse, CommandError> {
        let id = parse_kb_id(&self.kb_id)?;
        let row = ctx
            .db
            .get_knowledge_base(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeBase"))?;
        knowledge_base_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetKnowledgeBase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateKnowledgeBaseCmd {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    #[serde(flatten)]
    pub request: UpdateKnowledgeBaseRequest,
}

impl Command for UpdateKnowledgeBaseCmd {
    type Output = KnowledgeBaseResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_knowledge_base",
            category: "knowledge_bases",
            description: "Update a knowledge base.",
            method: "PATCH",
            path: "/v1/knowledge-bases/{kb_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeBaseResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeBaseResponse, CommandError> {
        let id = parse_kb_id(&self.kb_id)?;
        let existing = ctx
            .db
            .get_knowledge_base(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeBase"))?;
        // Archived KBs are read-only per knowledge/foundations/models.md lifecycle contract.
        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "Knowledge base is archived; restore it before updating",
            ));
        }
        let name = self
            .request
            .name
            .as_deref()
            .map(validate_name)
            .transpose()?;
        // Validate the embedding model if it's being updated.
        let embedding_model_id = match self.request.embedding_model_id {
            UpdateField::Set(model_id) => Some(UpdateField::Set(
                validate_embedding_model_id(ctx, model_id).await?,
            )),
            UpdateField::Clear => Some(UpdateField::Clear),
            UpdateField::Unchanged => None,
        };
        let row = ctx
            .db
            .update_knowledge_base(
                ctx.org_id(),
                existing.id,
                UpdateKnowledgeBase {
                    name,
                    description: match self.request.description {
                        UpdateField::Set(description) => Some(Some(description)),
                        UpdateField::Clear => Some(None),
                        UpdateField::Unchanged => None,
                    },
                    status: None,
                    embedding_model_id,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeBase"))?;
        knowledge_base_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateKnowledgeBaseCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteKnowledgeBase {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
}

impl Command for DeleteKnowledgeBase {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_knowledge_base",
            category: "knowledge_bases",
            description: "Archive a knowledge base.",
            method: "DELETE",
            path: "/v1/knowledge-bases/{kb_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_kb_id(&self.kb_id)?;
        let existing = ctx
            .db
            .get_knowledge_base(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeBase"))?;
        let archived = ctx
            .db
            .archive_knowledge_base(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("KnowledgeBase"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteKnowledgeBase>() }

// ============================================
// Knowledge Entry CRUD (scoped to a KB)
// ============================================

/// Resolve a KB's internal UUID. If `require_active` is true, archived KBs
/// reject the call so write paths honor the lifecycle contract from
/// `knowledge/foundations/models.md` (archived = read-only).
pub(super) async fn resolve_kb_internal_id(
    ctx: &Ctx,
    kb_id: &str,
    require_active: bool,
) -> Result<uuid::Uuid, CommandError> {
    let id = parse_kb_id(kb_id)?;
    let kb = ctx
        .db
        .get_knowledge_base(ctx.org_id(), id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("KnowledgeBase"))?;
    if require_active && kb.status != "active" {
        return Err(CommandError::bad_request(
            "Knowledge base is archived; restore it before modifying entries",
        ));
    }
    Ok(kb.id)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListKnowledgeEntries {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource.
    pub kind: Option<String>,
}

impl ListKnowledgeEntries {
    pub fn from_path_query(kb_id: String, query: ListKnowledgeEntriesQuery) -> Self {
        Self {
            kb_id,
            search: query.search,
            kind: query.kind,
        }
    }
}

impl Command for ListKnowledgeEntries {
    type Output = Vec<KnowledgeEntryResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_knowledge_entries",
            category: "knowledge_bases",
            description: "List entries inside a knowledge base.",
            method: "GET",
            path: "/v1/knowledge-bases/{kb_id}/entries",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<KnowledgeEntryResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<KnowledgeEntryResponse>, CommandError> {
        let kb_internal_id = resolve_kb_internal_id(ctx, &self.kb_id, false).await?;
        let kind = validate_kind(self.kind.as_deref())?;
        let rows = ctx
            .db
            .list_knowledge_entries(kb_internal_id, self.search.as_deref(), kind.as_deref())
            .await
            .map_err(classify_anyhow)?;
        rows.into_iter()
            .map(|row| knowledge_entry_response(row, &self.kb_id).map_err(classify_anyhow))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListKnowledgeEntries>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateKnowledgeEntry {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    /// Human-readable title. Safe to render in user-facing messages.
    pub title: String,
    pub body: String,
    #[serde(default)]
    /// Discriminator selecting the variant of this resource.
    pub kind: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    /// Optional OKF resource URI identifying the underlying asset.
    pub resource: Option<String>,
}

impl CreateKnowledgeEntry {
    pub fn from_request(kb_id: String, request: CreateKnowledgeEntryRequest) -> Self {
        Self {
            kb_id,
            title: request.title,
            body: request.body,
            kind: request.kind,
            tags: request.tags,
            resource: request.resource,
        }
    }
}

impl Command for CreateKnowledgeEntry {
    type Output = KnowledgeEntryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_knowledge_entry",
            category: "knowledge_bases",
            description: "Create an entry inside a knowledge base.",
            method: "POST",
            path: "/v1/knowledge-bases/{kb_id}/entries",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeEntryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeEntryResponse, CommandError> {
        let kb_internal_id = resolve_kb_internal_id(ctx, &self.kb_id, true).await?;
        let title = validate_title(&self.title)?;
        let body = validate_body(&self.body)?;
        let kind = validate_kind(self.kind.as_deref())?.unwrap_or_else(|| "note".to_string());
        let tags = validate_tags(self.tags)?.unwrap_or_default();
        let resource = validate_resource(self.resource)?;
        let input = CreateKnowledgeEntryRow {
            public_id: KnowledgeEntryId::new().to_string(),
            title,
            body,
            kind,
            tags,
            resource,
        };
        let row = ctx
            .db
            .create_knowledge_entry(kb_internal_id, input)
            .await
            .map_err(classify_anyhow)?;
        knowledge_entry_response(row, &self.kb_id).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateKnowledgeEntry>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetKnowledgeEntry {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    /// Knowledge base entry's prefixed public identifier.
    pub entry_id: String,
}

impl Command for GetKnowledgeEntry {
    type Output = KnowledgeEntryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_knowledge_entry",
            category: "knowledge_bases",
            description: "Get a knowledge entry by ID.",
            method: "GET",
            path: "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeEntryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeEntryResponse, CommandError> {
        let kb_internal_id = resolve_kb_internal_id(ctx, &self.kb_id, false).await?;
        let entry_id = parse_entry_id(&self.entry_id)?;
        let row = ctx
            .db
            .get_knowledge_entry(kb_internal_id, entry_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeEntry"))?;
        knowledge_entry_response(row, &self.kb_id).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetKnowledgeEntry>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateKnowledgeEntryCmd {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    /// Knowledge base entry's prefixed public identifier.
    pub entry_id: String,
    #[serde(flatten)]
    pub request: UpdateKnowledgeEntryRequest,
}

impl Command for UpdateKnowledgeEntryCmd {
    type Output = KnowledgeEntryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_knowledge_entry",
            category: "knowledge_bases",
            description: "Update a knowledge entry.",
            method: "PATCH",
            path: "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeEntryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeEntryResponse, CommandError> {
        let kb_internal_id = resolve_kb_internal_id(ctx, &self.kb_id, true).await?;
        let entry_id = parse_entry_id(&self.entry_id)?;
        let existing = ctx
            .db
            .get_knowledge_entry(kb_internal_id, entry_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeEntry"))?;
        let title = self
            .request
            .title
            .as_deref()
            .map(validate_title)
            .transpose()?;
        let body = self
            .request
            .body
            .as_deref()
            .map(validate_body)
            .transpose()?;
        let kind = validate_kind(self.request.kind.as_deref())?;
        let tags = validate_tags(self.request.tags)?;
        let resource = match self.request.resource {
            UpdateField::Set(resource) => Some(validate_resource(Some(resource))?),
            UpdateField::Clear => Some(None),
            UpdateField::Unchanged => None,
        };
        let row = ctx
            .db
            .update_knowledge_entry(
                kb_internal_id,
                existing.id,
                UpdateKnowledgeEntry {
                    title,
                    body,
                    kind,
                    tags,
                    resource,
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeEntry"))?;
        knowledge_entry_response(row, &self.kb_id).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateKnowledgeEntryCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteKnowledgeEntry {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    /// Knowledge base entry's prefixed public identifier.
    pub entry_id: String,
}

impl Command for DeleteKnowledgeEntry {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_knowledge_entry",
            category: "knowledge_bases",
            description: "Delete a knowledge entry.",
            method: "DELETE",
            path: "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&KNOWLEDGE_BASE_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let kb_internal_id = resolve_kb_internal_id(ctx, &self.kb_id, true).await?;
        let entry_id = parse_entry_id(&self.entry_id)?;
        let existing = ctx
            .db
            .get_knowledge_entry(kb_internal_id, entry_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("KnowledgeEntry"))?;
        let removed = ctx
            .db
            .delete_knowledge_entry(kb_internal_id, existing.id)
            .await
            .map_err(classify_anyhow)?;
        if removed {
            Ok(())
        } else {
            Err(CommandError::not_found("KnowledgeEntry"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteKnowledgeEntry>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::kernel_imports::{Caller, DEFAULT_ORG_ID, OrgRole};
    use crate::storage::StorageBackend;
    use crate::storage::models::{CreateModelRow, CreateProviderRow};
    use std::sync::Arc;

    fn ctx_for_org(org_id: i64) -> Ctx {
        Ctx::minimal_for_test(
            Caller {
                org_id,
                org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            Arc::new(StorageBackend::in_memory()),
            None,
        )
    }

    fn ctx_with_db(org_id: i64, db: Arc<StorageBackend>) -> Ctx {
        Ctx::minimal_for_test(
            Caller {
                org_id,
                org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
                user_id: None,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            None,
        )
    }

    async fn seed_model(
        db: &StorageBackend,
        org_id: i64,
        provider_type: &str,
        model_id: &str,
    ) -> everruns_provider::typed_id::ModelId {
        let provider = db
            .create_provider(
                org_id,
                CreateProviderRow {
                    name: format!("{provider_type} provider"),
                    provider_type: provider_type.to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .expect("create provider");
        db.create_model(
            org_id,
            CreateModelRow {
                provider_id: provider.id,
                model_id: model_id.to_string(),
                display_name: model_id.to_string(),
                capabilities: vec![],
                is_favorite: false,
                enabled: true,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .expect("create model")
        .id
    }

    #[tokio::test]
    async fn knowledge_base_lifecycle_round_trip() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateKnowledgeBase {
            name: "Sales Playbook".into(),
            description: Some("Curated sales knowledge".into()),
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect("create kb");

        assert!(created.id.to_string().starts_with("kb_"));
        assert_eq!(created.status, "active");

        let entry = CreateKnowledgeEntry {
            kb_id: created.id.to_string(),
            title: "Pipeline definition".into(),
            body: "Pipeline = sum of opportunities at stages 2-4.".into(),
            kind: Some("business".into()),
            tags: Some(vec!["pipeline".into(), "Pipeline".into()]),
            resource: None,
        }
        .run(&ctx)
        .await
        .expect("create entry");
        assert_eq!(entry.kind, "business");
        // Tags are normalized to lowercase and deduplicated.
        assert_eq!(entry.tags, vec!["pipeline".to_string()]);

        let listed = ListKnowledgeEntries {
            kb_id: created.id.to_string(),
            search: Some("pipeline".into()),
            kind: None,
        }
        .run(&ctx)
        .await
        .expect("list entries");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, entry.id);

        let updated = UpdateKnowledgeEntryCmd {
            kb_id: created.id.to_string(),
            entry_id: entry.id.to_string(),
            request: UpdateKnowledgeEntryRequest {
                title: Some("Pipeline (definition)".into()),
                body: None,
                kind: None,
                tags: None,
                resource: UpdateField::Unchanged,
            },
        }
        .run(&ctx)
        .await
        .expect("update entry");
        assert_eq!(updated.title, "Pipeline (definition)");

        DeleteKnowledgeEntry {
            kb_id: created.id.to_string(),
            entry_id: entry.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("delete entry");

        DeleteKnowledgeBase {
            kb_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive kb");

        let active = ListKnowledgeBases {
            search: None,
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list active kbs");
        assert!(active.is_empty());

        let archived = ListKnowledgeBases {
            search: None,
            include_archived: Some(true),
        }
        .run(&ctx)
        .await
        .expect("list archived kbs");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, "archived");
    }

    #[tokio::test]
    async fn rejects_invalid_kind_and_oversized_body() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        let kb = CreateKnowledgeBase {
            name: "K".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect("create kb");

        let err = CreateKnowledgeEntry {
            kb_id: kb.id.to_string(),
            title: "T".into(),
            body: "B".into(),
            kind: Some("nope".into()),
            tags: None,
            resource: None,
        }
        .run(&ctx)
        .await
        .expect_err("invalid kind should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let oversize = "x".repeat(MAX_ENTRY_BODY_BYTES + 1);
        let err = CreateKnowledgeEntry {
            kb_id: kb.id.to_string(),
            title: "T".into(),
            body: oversize,
            kind: None,
            tags: None,
            resource: None,
        }
        .run(&ctx)
        .await
        .expect_err("oversized body should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn knowledge_bases_do_not_cross_orgs() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateKnowledgeBase {
            name: "Private".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&org_one)
        .await
        .expect("create kb");

        let err = GetKnowledgeBase {
            kb_id: created.id.to_string(),
        }
        .run(&org_two)
        .await
        .expect_err("other org should not read kb");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::NotFound(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn duplicate_active_kb_names_conflict() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        CreateKnowledgeBase {
            name: "Research".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect("create kb");

        let err = CreateKnowledgeBase {
            name: "research".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect_err("duplicate name should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Conflict(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn embedding_model_must_use_embeddings_provider_on_create() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());
        let chat_only_model = seed_model(&db, DEFAULT_ORG_ID, "anthropic", "claude-sonnet").await;

        let err = CreateKnowledgeBase {
            name: "Hybrid KB".into(),
            description: None,
            embedding_model_id: Some(chat_only_model),
        }
        .run(&ctx)
        .await
        .expect_err("chat-only provider should be rejected");

        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn embedding_model_must_use_embeddings_provider_on_update() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());
        let chat_only_model = seed_model(&db, DEFAULT_ORG_ID, "anthropic", "claude-sonnet").await;
        let kb = CreateKnowledgeBase {
            name: "Hybrid KB".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect("create kb");

        let err = UpdateKnowledgeBaseCmd {
            kb_id: kb.id.to_string(),
            request: UpdateKnowledgeBaseRequest {
                name: None,
                description: UpdateField::Unchanged,
                embedding_model_id: UpdateField::Set(chat_only_model),
            },
        }
        .run(&ctx)
        .await
        .expect_err("chat-only provider should be rejected");

        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn accepts_embedding_model_from_embeddings_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());
        let embedding_model =
            seed_model(&db, DEFAULT_ORG_ID, "openai", "text-embedding-3-small").await;

        let kb = CreateKnowledgeBase {
            name: "Hybrid KB".into(),
            description: None,
            embedding_model_id: Some(embedding_model),
        }
        .run(&ctx)
        .await
        .expect("embedding provider should be accepted");

        assert_eq!(kb.embedding_model_id, Some(embedding_model));
    }

    #[tokio::test]
    async fn archived_kb_rejects_mutations_but_allows_reads() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        let kb = CreateKnowledgeBase {
            name: "Archive Me".into(),
            description: None,
            embedding_model_id: None,
        }
        .run(&ctx)
        .await
        .expect("create kb");

        // Seed an entry while still active.
        let entry = CreateKnowledgeEntry {
            kb_id: kb.id.to_string(),
            title: "Seed".into(),
            body: "body".into(),
            kind: None,
            tags: None,
            resource: None,
        }
        .run(&ctx)
        .await
        .expect("create entry");

        DeleteKnowledgeBase {
            kb_id: kb.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive kb");

        // Reads keep working against archived KBs.
        let list = ListKnowledgeEntries {
            kb_id: kb.id.to_string(),
            search: None,
            kind: None,
        }
        .run(&ctx)
        .await
        .expect("list entries on archived kb");
        assert_eq!(list.len(), 1);

        // Writes are rejected.
        let err = CreateKnowledgeEntry {
            kb_id: kb.id.to_string(),
            title: "Nope".into(),
            body: "body".into(),
            kind: None,
            tags: None,
            resource: None,
        }
        .run(&ctx)
        .await
        .expect_err("create on archived kb should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let err = UpdateKnowledgeEntryCmd {
            kb_id: kb.id.to_string(),
            entry_id: entry.id.to_string(),
            request: UpdateKnowledgeEntryRequest {
                title: Some("Nope".into()),
                body: None,
                kind: None,
                tags: None,
                resource: UpdateField::Unchanged,
            },
        }
        .run(&ctx)
        .await
        .expect_err("update on archived kb should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let err = DeleteKnowledgeEntry {
            kb_id: kb.id.to_string(),
            entry_id: entry.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect_err("delete on archived kb should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let err = UpdateKnowledgeBaseCmd {
            kb_id: kb.id.to_string(),
            request: UpdateKnowledgeBaseRequest {
                name: Some("Renamed".into()),
                description: UpdateField::Unchanged,
                embedding_model_id: UpdateField::Unchanged,
            },
        }
        .run(&ctx)
        .await
        .expect_err("update on archived kb should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }
}
