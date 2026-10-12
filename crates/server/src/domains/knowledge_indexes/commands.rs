use super::types::{
    CreateKnowledgeIndexRequest, CreateKnowledgeIndexRow, KnowledgeIndexDocumentResponse,
    KnowledgeIndexResponse, ListKnowledgeIndexesQuery, UpdateKnowledgeIndex,
    UpdateKnowledgeIndexRequest, knowledge_index_document_response, knowledge_index_response,
};
use super::{DEFAULT_SOURCE_TYPE, KNOWLEDGE_INDEX_MANAGE, KNOWLEDGE_INDEX_VIEW, SOURCE_TYPES};
use crate::domains::common::*;
use crate::domains::git_sources::normalize_github_repository;
use crate::kernel_imports::{
    contracts::driver_registry::ServiceKind, contracts::provider::DriverId,
};
use everruns_contracts::typed_id::KnowledgeIndexId;
use everruns_contracts::vector_store::index_namespace;
use serde::Deserialize;
use utoipa::ToSchema;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request(
            "Knowledge index name cannot be empty",
        ));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Knowledge index name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

fn parse_index_id(index_id: &str) -> Result<KnowledgeIndexId, CommandError> {
    index_id
        .parse()
        .map_err(|_| CommandError::not_found("KnowledgeIndex"))
}

fn validate_source_type(source_type: Option<&str>) -> Result<String, CommandError> {
    let source_type = source_type.map(str::trim).unwrap_or(DEFAULT_SOURCE_TYPE);
    if !SOURCE_TYPES.contains(&source_type) {
        return Err(CommandError::bad_request(format!(
            "Invalid source_type '{}'; must be one of: {}",
            source_type,
            SOURCE_TYPES.join(", ")
        )));
    }
    Ok(source_type.to_string())
}

fn normalize_source_config(
    source_type: &str,
    source_config: Option<serde_json::Value>,
) -> Result<serde_json::Value, CommandError> {
    let Some(mut config) = source_config else {
        return Ok(serde_json::json!({}));
    };
    if source_type != "github" {
        return Ok(config);
    }
    let object = config.as_object_mut().ok_or_else(|| {
        CommandError::bad_request("GitHub source_config must be an object with a repository field")
    })?;
    if let Some(field) = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "provider" | "repository" | "branch" | "root_folder"
        )
    }) {
        return Err(CommandError::bad_request(format!(
            "Unsupported GitHub source field '{field}'; credentials must use a GitHub connection"
        )));
    }
    if object
        .get("provider")
        .is_some_and(|provider| provider.as_str() != Some("github"))
    {
        return Err(CommandError::bad_request(
            "GitHub source provider must be 'github'",
        ));
    }
    let repository = object
        .get("repository")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CommandError::bad_request("GitHub repository is required"))?;
    let normalized = normalize_github_repository(repository).map_err(CommandError::bad_request)?;
    object.insert(
        "repository".to_string(),
        serde_json::Value::String(normalized),
    );
    Ok(config)
}

const INVALID_EMBEDDING_MODEL: &str =
    "Embedding model is unavailable or does not support embeddings";

/// Validate the complete org-scoped model → provider → service binding.
/// Cross-org, missing, disabled, and incompatible references deliberately use
/// one response so the API cannot be used as a model/provider existence oracle.
async fn require_embedding_model(
    ctx: &Ctx,
    model_id: everruns_contracts::typed_id::ModelId,
) -> Result<everruns_contracts::typed_id::ModelId, CommandError> {
    // THREAT[TM-AUTHZ-015]: resolve both resources inside the caller's org and
    // collapse every invalid binding into one non-enumerable response.
    let model = ctx
        .db
        .get_model(ctx.org_id(), model_id.uuid())
        .await?
        .ok_or_else(|| CommandError::bad_request(INVALID_EMBEDDING_MODEL))?;
    let capabilities: Vec<String> = serde_json::from_value(model.capabilities).unwrap_or_default();
    if !model.enabled
        || !capabilities
            .iter()
            .any(|capability| capability.eq_ignore_ascii_case("embeddings"))
    {
        return Err(CommandError::bad_request(INVALID_EMBEDDING_MODEL));
    }
    let provider = ctx
        .db
        .get_provider(ctx.org_id(), model.provider_id.uuid())
        .await?
        .ok_or_else(|| CommandError::bad_request(INVALID_EMBEDDING_MODEL))?;
    let provider_type: DriverId = provider
        .provider_type
        .parse()
        .expect("DriverId::from_str is infallible");
    if provider.status != "active"
        || !ctx
            .driver_registry
            .supports(&provider_type, ServiceKind::Embeddings)
    {
        return Err(CommandError::bad_request(INVALID_EMBEDDING_MODEL));
    }
    Ok(model_id)
}

async fn response_with_document_count(
    ctx: &Ctx,
    row: crate::storage::KnowledgeIndexRow,
) -> Result<KnowledgeIndexResponse, CommandError> {
    let document_count = ctx
        .db
        .count_knowledge_index_documents(&[row.id])
        .await?
        .get(&row.id)
        .copied()
        .unwrap_or(0);
    knowledge_index_response(row, document_count).map_err(classify_anyhow)
}

// ============================================
// Knowledge Index CRUD
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListKnowledgeIndexes {
    /// Case-insensitive substring match on name or description.
    #[serde(default)]
    pub search: Option<String>,
    /// Also return archived items.
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListKnowledgeIndexesQuery> for ListKnowledgeIndexes {
    fn from(query: ListKnowledgeIndexesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: query.include_archived,
        }
    }
}

#[command(
    name = "list_knowledge_indexes",
    category = "knowledge_indexes",
    description = "List knowledge indexes in the current organization.",
    method = "GET",
    path = "/v1/knowledge-indexes",
    policy = KNOWLEDGE_INDEX_VIEW,
    cli = CliRoute::new(&["knowledge-indexes"], "list").with_examples(&[CliExample::new("Find a knowledge index by name when you do not know the id", "everruns knowledge-indexes list --search docs")]),
)]
impl Command for ListKnowledgeIndexes {
    type Output = Vec<KnowledgeIndexResponse>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<KnowledgeIndexResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<KnowledgeIndexResponse>, CommandError> {
        let rows = ctx
            .db
            .list_knowledge_indexes(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await?;
        let counts = ctx
            .db
            .count_knowledge_index_documents(&rows.iter().map(|row| row.id).collect::<Vec<_>>())
            .await?;
        rows.into_iter()
            .map(|row| {
                let document_count = counts.get(&row.id).copied().unwrap_or(0);
                knowledge_index_response(row, document_count).map_err(classify_anyhow)
            })
            .collect()
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateKnowledgeIndex {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    #[serde(default)]
    /// External source type. One of `github`, `git`. Defaults to `github`.
    pub source_type: Option<String>,
    #[serde(default)]
    /// Non-secret source coordinates.
    pub source_config: Option<serde_json::Value>,
    #[schema(value_type = String)]
    /// Embedding model used to embed chunks. Required.
    pub embedding_model_id: everruns_contracts::typed_id::ModelId,
}

impl From<CreateKnowledgeIndexRequest> for CreateKnowledgeIndex {
    fn from(request: CreateKnowledgeIndexRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            source_type: request.source_type,
            source_config: request.source_config,
            embedding_model_id: request.embedding_model_id,
        }
    }
}

#[command(
    name = "create_knowledge_index",
    category = "knowledge_indexes",
    description = "Create a knowledge index in the current organization.",
    method = "POST",
    path = "/v1/knowledge-indexes",
    policy = KNOWLEDGE_INDEX_MANAGE,
    cli = CliRoute::new(&["knowledge-indexes"], "create").with_examples(&[CliExample::new("Index a GitHub repository so agents can search its docs", "everruns knowledge-indexes create --name product-docs --embedding-model-id model_01h9 --source-type github --source-config '{\"provider\":\"github\",\"repository\":\"acme/docs\"}' --reason 'Give support agents the product docs'")]),
)]
impl Command for CreateKnowledgeIndex {
    type Output = KnowledgeIndexResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeIndexResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeIndexResponse, CommandError> {
        let name = validate_name(&self.name)?;
        let source_type = validate_source_type(self.source_type.as_deref())?;
        let source_config = normalize_source_config(&source_type, self.source_config)?;
        let embedding_model_id = require_embedding_model(ctx, self.embedding_model_id).await?;
        // Assign the vector-store namespace at creation; it is org-prefixed and
        // never reused. See knowledge/runtime-resources/knowledge-indexes.md#multitenancy-and-naming.
        let public_id = KnowledgeIndexId::new().to_string();
        let vector_namespace = index_namespace(ctx.org_id(), &public_id);
        let input = CreateKnowledgeIndexRow {
            public_id,
            name,
            description: self.description,
            source_type,
            source_config,
            embedding_model_id,
            vector_namespace,
            owner_principal_id: None,
            resolved_owner_user_id: ctx.caller.user_id,
        };
        let row = ctx.db.create_knowledge_index(ctx.org_id(), input).await?;
        knowledge_index_response(row, 0).map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetKnowledgeIndex {
    /// Knowledge index's prefixed public identifier.
    pub index_id: String,
}

#[command(
    name = "get_knowledge_index",
    category = "knowledge_indexes",
    description = "Get a knowledge index by ID.",
    method = "GET",
    path = "/v1/knowledge-indexes/{index_id}",
    policy = KNOWLEDGE_INDEX_VIEW,
    cli = CliRoute::new(&["knowledge-indexes"], "get").with_args(&[CliArg::new("index_id").at(1)]).with_examples(&[CliExample::new("Check a knowledge index's source and sync state", "everruns knowledge-indexes get kidx_01h9")]),
    positional = "index_id",
)]
impl Command for GetKnowledgeIndex {
    type Output = KnowledgeIndexResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeIndexResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeIndexResponse, CommandError> {
        let id = parse_index_id(&self.index_id)?;
        let row = ctx
            .db
            .get_knowledge_index(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        response_with_document_count(ctx, row).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateKnowledgeIndexCmd {
    /// Knowledge index's prefixed public identifier.
    pub index_id: String,
    #[serde(flatten)]
    pub request: UpdateKnowledgeIndexRequest,
}

#[command(
    name = "update_knowledge_index",
    category = "knowledge_indexes",
    description = "Update a knowledge index.",
    method = "PATCH",
    path = "/v1/knowledge-indexes/{index_id}",
    policy = KNOWLEDGE_INDEX_MANAGE,
    cli = CliRoute::new(&["knowledge-indexes"], "update").with_examples(&[CliExample::new("Point an index at a different branch or folder", "everruns knowledge-indexes update --index-id kidx_01h9 --source-config '{\"provider\":\"github\",\"repository\":\"acme/docs\",\"branch\":\"main\"}' --reason 'Track main instead of the release branch'")]),
)]
impl Command for UpdateKnowledgeIndexCmd {
    type Output = KnowledgeIndexResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeIndexResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeIndexResponse, CommandError> {
        let id = parse_index_id(&self.index_id)?;
        let existing = ctx
            .db
            .get_knowledge_index(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        // Archived indexes are read-only per knowledge/foundations/models.md lifecycle contract.
        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "Knowledge index is archived; restore it before updating",
            ));
        }
        let name = self
            .request
            .name
            .as_deref()
            .map(validate_name)
            .transpose()?;
        // The embedding model is required, so it can be changed but not cleared.
        let source_changed = self.request.source_config.is_some();
        let enqueue_sync = source_changed || self.request.embedding_model_id.is_some();
        let embedding_model_id = match self.request.embedding_model_id {
            Some(model_id) => Some(require_embedding_model(ctx, model_id).await?),
            None => None,
        };
        let source_config = self
            .request
            .source_config
            .map(|config| normalize_source_config(&existing.source_type, Some(config)))
            .transpose()?;
        let resolved_owner_user_id = if source_changed {
            // THREAT[TM-AUTHZ-011]: Source edits must not continue to use the
            // previous owner's external connection. Rebind the sync token owner
            // to the caller who selected the new source coordinates.
            crate::storage::UpdateField::from_option(ctx.caller.user_id)
        } else {
            crate::storage::UpdateField::Unchanged
        };
        let row = ctx
            .db
            .update_knowledge_index(
                ctx.org_id(),
                existing.id,
                UpdateKnowledgeIndex {
                    name,
                    description: match self.request.description {
                        crate::storage::UpdateField::Set(description) => Some(Some(description)),
                        crate::storage::UpdateField::Clear => Some(None),
                        crate::storage::UpdateField::Unchanged => None,
                    },
                    source_config,
                    resolved_owner_user_id,
                    embedding_model_id,
                    enqueue_sync,
                    status: None,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        response_with_document_count(ctx, row).await
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteKnowledgeIndex {
    /// Knowledge index's prefixed public identifier.
    pub index_id: String,
}

#[command(
    name = "delete_knowledge_index",
    category = "knowledge_indexes",
    description = "Archive a knowledge index.",
    method = "DELETE",
    path = "/v1/knowledge-indexes/{index_id}",
    policy = KNOWLEDGE_INDEX_MANAGE,
    cli = CliRoute::new(&["knowledge-indexes"], "delete").with_examples(&[CliExample::new("Archive a knowledge index that agents no longer need", "everruns knowledge-indexes delete --index-id kidx_01h9 --reason 'Docs moved to a new index'")]),
)]
impl Command for DeleteKnowledgeIndex {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_index_id(&self.index_id)?;
        let existing = ctx
            .db
            .get_knowledge_index(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        let archived = ctx
            .db
            .archive_knowledge_index(ctx.org_id(), existing.id)
            .await?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("KnowledgeIndex"))
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SyncKnowledgeIndex {
    /// Knowledge index's prefixed public identifier.
    pub index_id: String,
}

#[command(
    name = "sync_knowledge_index",
    category = "knowledge_indexes",
    description = "Enqueue a manual sync of a knowledge index.",
    method = "POST",
    path = "/v1/knowledge-indexes/{index_id}/sync",
    policy = KNOWLEDGE_INDEX_MANAGE,
    cli = CliRoute::new(&["knowledge-indexes"], "sync").with_args(&[CliArg::new("index_id").at(1)]).with_examples(&[CliExample::new("Re-index now after the source repository changed", "everruns knowledge-indexes sync kidx_01h9 --reason 'Docs updated upstream'")]),
    positional = "index_id",
)]
impl Command for SyncKnowledgeIndex {
    type Output = KnowledgeIndexResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<KnowledgeIndexResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<KnowledgeIndexResponse, CommandError> {
        let id = parse_index_id(&self.index_id)?;
        let existing = ctx
            .db
            .get_knowledge_index(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        // Archived/deleted indexes are not syncable per the lifecycle contract.
        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "Knowledge index is archived; restore it before syncing",
            ));
        }
        require_embedding_model(ctx, existing.embedding_model_id).await?;
        let row = ctx
            .db
            .enqueue_knowledge_index_sync(ctx.org_id(), existing.id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        response_with_document_count(ctx, row).await
    }
}

// ============================================
// Knowledge Index Documents (read-only; populated by the Syncout worker)
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListKnowledgeIndexDocuments {
    /// Knowledge index's prefixed public identifier.
    pub index_id: String,
}

#[command(
    name = "list_knowledge_index_documents",
    category = "knowledge_indexes",
    description = "List documents inside a knowledge index.",
    method = "GET",
    path = "/v1/knowledge-indexes/{index_id}/documents",
    policy = KNOWLEDGE_INDEX_VIEW,
    cli = CliRoute::new(&["knowledge-indexes", "documents"], "list").with_args(&[CliArg::new("index_id").at(1)]).with_examples(&[CliExample::new("See which documents a sync has indexed", "everruns knowledge-indexes documents list kidx_01h9")]),
    positional = "index_id",
)]
impl Command for ListKnowledgeIndexDocuments {
    type Output = Vec<KnowledgeIndexDocumentResponse>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<KnowledgeIndexDocumentResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<KnowledgeIndexDocumentResponse>, CommandError> {
        let id = parse_index_id(&self.index_id)?;
        let index = ctx
            .db
            .get_knowledge_index(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("KnowledgeIndex"))?;
        let rows = ctx.db.list_knowledge_index_documents(index.id).await?;
        rows.into_iter()
            .map(|row| {
                knowledge_index_document_response(row, &self.index_id).map_err(classify_anyhow)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::kernel_imports::{Caller, DEFAULT_ORG_ID, ModelId, OrgRole};
    use crate::storage::StorageBackend;
    use everruns_contracts::vector_store::index_namespace;
    use std::sync::Arc;
    use uuid::Uuid;

    fn ctx_with_db(org_id: i64, db: Arc<StorageBackend>) -> Ctx {
        ctx_with_db_and_user(org_id, db, None)
    }

    fn ctx_with_db_and_user(org_id: i64, db: Arc<StorageBackend>, user_id: Option<Uuid>) -> Ctx {
        Ctx::minimal_for_test(
            Caller {
                org_id,
                org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
                user_id,
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            None,
        )
    }

    /// Seed an embeddings-capable model for the org so create validation passes.
    async fn seed_model(db: &StorageBackend, org_id: i64) -> ModelId {
        let provider = db
            .create_provider(
                org_id,
                crate::storage::CreateProviderRow {
                    name: "test-provider".into(),
                    provider_type: "openai".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .expect("create provider");
        let model = db
            .create_model(
                org_id,
                crate::storage::CreateModelRow {
                    provider_id: provider.id,
                    model_id: "text-embedding-3-small".into(),
                    display_name: "Embeddings".into(),
                    capabilities: vec!["embeddings".into()],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .expect("create model");
        model.id
    }

    #[tokio::test]
    async fn knowledge_index_lifecycle_round_trip() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let created = CreateKnowledgeIndex {
            name: "Product Docs".into(),
            description: Some("Synced docs".into()),
            source_type: None,
            source_config: Some(serde_json::json!({"repository": "owner/repo"})),
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");

        assert!(created.id.to_string().starts_with("kidx_"));
        assert_eq!(created.status, "active");
        assert_eq!(created.sync_status, "pending");
        assert_eq!(created.source_type, "github");

        // Vector namespace is assigned at create time and org-prefixed.
        let internal = ctx
            .db
            .get_knowledge_index_by_id(DEFAULT_ORG_ID, created.internal_id)
            .await
            .expect("get internal")
            .expect("row");
        assert_eq!(
            internal.vector_namespace.as_deref(),
            Some(index_namespace(DEFAULT_ORG_ID, &created.id.to_string()).as_str())
        );

        // Documents are empty until the Syncout worker claims the pending row.
        let docs = ListKnowledgeIndexDocuments {
            index_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("list documents");
        assert!(docs.is_empty());

        let updated = UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: Some("Product Docs v2".into()),
                description: crate::storage::UpdateField::Unchanged,
                source_config: None,
                embedding_model_id: None,
            },
        }
        .run(&ctx)
        .await
        .expect("update index");
        assert_eq!(updated.name, "Product Docs v2");

        DeleteKnowledgeIndex {
            index_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive index");

        let active = ListKnowledgeIndexes {
            search: None,
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list active");
        assert!(active.is_empty());

        let archived = ListKnowledgeIndexes {
            search: None,
            include_archived: Some(true),
        }
        .run(&ctx)
        .await
        .expect("list archived");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, "archived");
    }

    #[tokio::test]
    async fn github_source_urls_are_normalized_before_storage() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let created = CreateKnowledgeIndex {
            name: "Public Docs".into(),
            description: None,
            source_type: Some("github".into()),
            source_config: Some(serde_json::json!({
                "repository": "https://github.com/everruns/bashkit.git/",
                "branch": "main",
                "root_folder": "knowledge"
            })),
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");

        assert_eq!(
            created.source_config["repository"],
            serde_json::json!("everruns/bashkit")
        );
    }

    #[tokio::test]
    async fn github_source_rejects_unsupported_urls() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let error = CreateKnowledgeIndex {
            name: "Invalid Docs".into(),
            description: None,
            source_type: Some("github".into()),
            source_config: Some(serde_json::json!({
                "repository": "https://gitlab.com/everruns/bashkit"
            })),
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect_err("unsupported host should fail");

        assert!(matches!(error.kind, CommandErrorKind::BadRequest(_)));
        assert!(error.message().contains("https://github.com/owner/repo"));
    }

    #[tokio::test]
    async fn github_source_rejects_inline_credential_fields() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let error = CreateKnowledgeIndex {
            name: "Secret Docs".into(),
            description: None,
            source_type: Some("github".into()),
            source_config: Some(serde_json::json!({
                "repository": "everruns/bashkit",
                "token": "must-not-be-stored"
            })),
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect_err("inline credentials should fail");

        assert!(matches!(error.kind, CommandErrorKind::BadRequest(_)));
        assert!(
            error
                .message()
                .contains("credentials must use a GitHub connection")
        );
    }

    #[tokio::test]
    async fn source_config_update_rebinds_sync_owner_to_caller() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let creator_user_id = Uuid::new_v4();
        let updater_user_id = Uuid::new_v4();
        let creator = ctx_with_db_and_user(DEFAULT_ORG_ID, db.clone(), Some(creator_user_id));
        let updater = ctx_with_db_and_user(DEFAULT_ORG_ID, db, Some(updater_user_id));

        let created = CreateKnowledgeIndex {
            name: "Private Docs".into(),
            description: None,
            source_type: None,
            source_config: Some(serde_json::json!({"repository": "owner/original"})),
            embedding_model_id: model_id,
        }
        .run(&creator)
        .await
        .expect("create index");

        UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: None,
                description: crate::storage::UpdateField::Unchanged,
                source_config: Some(serde_json::json!({
                    "repository": "https://github.com/owner/rebound.git/"
                })),
                embedding_model_id: None,
            },
        }
        .run(&updater)
        .await
        .expect("update source config");

        let row = updater
            .db
            .get_knowledge_index_by_id(DEFAULT_ORG_ID, created.internal_id)
            .await
            .expect("get internal")
            .expect("row");
        assert_eq!(row.resolved_owner_user_id, Some(updater_user_id));
        assert_eq!(row.source_config["repository"], "owner/rebound");
    }

    #[tokio::test]
    async fn metadata_update_preserves_sync_owner() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let creator_user_id = Uuid::new_v4();
        let updater_user_id = Uuid::new_v4();
        let creator = ctx_with_db_and_user(DEFAULT_ORG_ID, db.clone(), Some(creator_user_id));
        let updater = ctx_with_db_and_user(DEFAULT_ORG_ID, db, Some(updater_user_id));

        let created = CreateKnowledgeIndex {
            name: "Stable Docs".into(),
            description: None,
            source_type: None,
            source_config: Some(serde_json::json!({"repository": "owner/original"})),
            embedding_model_id: model_id,
        }
        .run(&creator)
        .await
        .expect("create index");

        UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: Some("Stable Docs v2".into()),
                description: crate::storage::UpdateField::Unchanged,
                source_config: None,
                embedding_model_id: None,
            },
        }
        .run(&updater)
        .await
        .expect("update metadata");

        let row = updater
            .db
            .get_knowledge_index_by_id(DEFAULT_ORG_ID, created.internal_id)
            .await
            .expect("get internal")
            .expect("row");
        assert_eq!(row.resolved_owner_user_id, Some(creator_user_id));
    }

    #[tokio::test]
    async fn create_requires_existing_embedding_model() {
        let db = Arc::new(StorageBackend::test_database());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let err = CreateKnowledgeIndex {
            name: "No Model".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: ModelId::new(),
        }
        .run(&ctx)
        .await
        .expect_err("missing model should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
        assert_eq!(err.message(), INVALID_EMBEDDING_MODEL);
    }

    #[tokio::test]
    async fn create_rejects_chat_model_from_embeddings_capable_provider() {
        let db = Arc::new(StorageBackend::test_database());
        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                crate::storage::CreateProviderRow {
                    name: "OpenAI".into(),
                    provider_type: "openai".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .expect("create provider");
        let chat_model = db
            .create_model(
                DEFAULT_ORG_ID,
                crate::storage::CreateModelRow {
                    provider_id: provider.id,
                    model_id: "gpt-5".into(),
                    display_name: "GPT-5".into(),
                    capabilities: vec!["chat".into()],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .expect("create model");
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let err = CreateKnowledgeIndex {
            name: "Invalid model".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: chat_model.id,
        }
        .run(&ctx)
        .await
        .expect_err("chat model must fail");

        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
        assert_eq!(err.message(), INVALID_EMBEDDING_MODEL);
    }

    #[tokio::test]
    async fn create_rejects_embedding_tag_when_provider_lacks_service() {
        let db = Arc::new(StorageBackend::test_database());
        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                crate::storage::CreateProviderRow {
                    name: "Anthropic".into(),
                    provider_type: "anthropic".into(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .expect("create provider");
        let incorrectly_tagged_model = db
            .create_model(
                DEFAULT_ORG_ID,
                crate::storage::CreateModelRow {
                    provider_id: provider.id,
                    model_id: "claude-embedding-impostor".into(),
                    display_name: "Not Embeddings".into(),
                    capabilities: vec!["embeddings".into()],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .expect("create model");
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let err = CreateKnowledgeIndex {
            name: "Unsupported provider".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: incorrectly_tagged_model.id,
        }
        .run(&ctx)
        .await
        .expect_err("provider without embeddings service must fail");

        assert_eq!(err.message(), INVALID_EMBEDDING_MODEL);
    }

    #[tokio::test]
    async fn update_rejects_chat_model_and_preserves_valid_configuration() {
        let db = Arc::new(StorageBackend::test_database());
        let embedding_model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let embedding_model = db
            .get_model(DEFAULT_ORG_ID, embedding_model_id.uuid())
            .await
            .expect("get model")
            .expect("embedding model");
        let chat_model = db
            .create_model(
                DEFAULT_ORG_ID,
                crate::storage::CreateModelRow {
                    provider_id: embedding_model.provider_id,
                    model_id: "gpt-5".into(),
                    display_name: "GPT-5".into(),
                    capabilities: vec!["chat".into()],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .expect("create chat model");
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);
        let created = CreateKnowledgeIndex {
            name: "Valid index".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");

        UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: None,
                description: crate::storage::UpdateField::Unchanged,
                source_config: None,
                embedding_model_id: Some(chat_model.id),
            },
        }
        .run(&ctx)
        .await
        .expect_err("chat model update must fail");

        let stored = ctx
            .db
            .get_knowledge_index_by_id(DEFAULT_ORG_ID, created.internal_id)
            .await
            .expect("get index")
            .expect("stored index");
        assert_eq!(stored.embedding_model_id, embedding_model_id);
    }

    #[tokio::test]
    async fn valid_model_repair_requeues_failed_index_and_clears_error() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);
        let created = CreateKnowledgeIndex {
            name: "Repairable index".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");
        let claimed = ctx
            .db
            .claim_next_knowledge_index_sync()
            .await
            .expect("claim")
            .expect("pending index");
        ctx.db
            .fail_knowledge_index_sync(claimed.id, claimed.updated_at, "invalid configuration")
            .await
            .expect("fail sync")
            .expect("failed row");

        let repaired = UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: None,
                description: crate::storage::UpdateField::Unchanged,
                source_config: None,
                embedding_model_id: Some(model_id),
            },
        }
        .run(&ctx)
        .await
        .expect("repair model");

        assert_eq!(repaired.sync_status, "pending");
        assert_eq!(repaired.last_sync_error, None);
    }

    #[tokio::test]
    async fn rejects_invalid_source_type() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let err = CreateKnowledgeIndex {
            name: "Bad Source".into(),
            description: None,
            source_type: Some("dropbox".into()),
            source_config: None,
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect_err("invalid source_type should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn rejects_cross_org_embedding_model() {
        let db = Arc::new(StorageBackend::test_database());
        // Model belongs to org 2; org 1 must not be able to reference it, and the
        // error must not leak that it exists in another org.
        let foreign_model = seed_model(&db, 2).await;
        let ctx_one = ctx_with_db(1, db);

        let err = CreateKnowledgeIndex {
            name: "Cross Org".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: foreign_model,
        }
        .run(&ctx_one)
        .await
        .expect_err("cross-org model should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
        assert_eq!(err.message(), INVALID_EMBEDDING_MODEL);
    }

    #[tokio::test]
    async fn knowledge_indexes_do_not_cross_orgs() {
        let db = Arc::new(StorageBackend::test_database());
        let model_one = seed_model(&db, 1).await;
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateKnowledgeIndex {
            name: "Private".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: model_one,
        }
        .run(&org_one)
        .await
        .expect("create index");

        let err = GetKnowledgeIndex {
            index_id: created.id.to_string(),
        }
        .run(&org_two)
        .await
        .expect_err("other org should not read index");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::NotFound(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn duplicate_active_index_names_conflict() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        CreateKnowledgeIndex {
            name: "Research".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");

        let err = CreateKnowledgeIndex {
            name: "research".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: model_id,
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
    async fn archived_index_rejects_updates() {
        let db = Arc::new(StorageBackend::test_database());
        let model_id = seed_model(&db, DEFAULT_ORG_ID).await;
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db);

        let created = CreateKnowledgeIndex {
            name: "Archive Me".into(),
            description: None,
            source_type: None,
            source_config: None,
            embedding_model_id: model_id,
        }
        .run(&ctx)
        .await
        .expect("create index");

        DeleteKnowledgeIndex {
            index_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive index");

        let err = UpdateKnowledgeIndexCmd {
            index_id: created.id.to_string(),
            request: UpdateKnowledgeIndexRequest {
                name: Some("Renamed".into()),
                description: crate::storage::UpdateField::Unchanged,
                source_config: None,
                embedding_model_id: None,
            },
        }
        .run(&ctx)
        .await
        .expect_err("update on archived index should fail");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));
    }
}
