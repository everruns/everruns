use super::types::{
    CreateMemoryRequest, CreateMemoryRow, CreateMemorySourceRequest, ListMemoriesQuery,
    MemoryResponse, UpdateMemory, UpdateMemoryRequest, memory_response,
};
use super::{MEMORY_MANAGE, MEMORY_VIEW};
use crate::domains::common::*;
use crate::storage::UpdateField;
use everruns_contracts::typed_id::MemoryId;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request("Memory name cannot be empty"));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Memory name must be at most 255 characters",
        ));
    }
    Ok(trimmed.to_string())
}

/// Memory v2 retired source-backed memories: synced documents belong in
/// Knowledge, and memory is what agents and people write. Existing GitHub and
/// Git memories were archived by the `memory_v2` migration.
fn create_memory_source(
    source: Option<CreateMemorySourceRequest>,
) -> Result<(String, Value, bool, String), CommandError> {
    match source {
        None => Ok(("manual".to_string(), json!({}), false, "idle".to_string())),
        Some(_) => Err(CommandError::bad_request(
            "source-backed memories were retired; sync documents into Knowledge instead",
        )),
    }
}

fn parse_memory_id(memory_id: &str) -> Result<MemoryId, CommandError> {
    memory_id
        .parse()
        .map_err(|_| CommandError::not_found("Memory"))
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListMemories {
    /// Only memories whose name matches this text.
    #[serde(default)]
    pub search: Option<String>,
    /// Include archived memories.
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListMemoriesQuery> for ListMemories {
    fn from(query: ListMemoriesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: query.include_archived,
        }
    }
}

#[command(
    name = "list_memories",
    category = "memories",
    description = "List workspace memories in the current organization.",
    method = "GET",
    path = "/v1/platform/memories",
    policy = MEMORY_VIEW,
    cli = CliRoute::new(&["memories"], "list").with_examples(&[CliExample::new("Find memories by name when you do not know the id", "everruns memories list --search design",)]),
)]
impl Command for ListMemories {
    type Output = Vec<MemoryResponse>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<MemoryResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MemoryResponse>, CommandError> {
        let rows = ctx
            .db
            .list_memories(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await?;
        rows.into_iter()
            .map(|row| memory_response(row).map_err(classify_anyhow))
            .collect()
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateMemory {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Optional sync source, e.g. `{"type":"github","repository":"acme/design-docs"}` or `{"type":"git","url":...}`.
    #[serde(default)]
    pub source: Option<CreateMemorySourceRequest>,
}

impl From<CreateMemoryRequest> for CreateMemory {
    fn from(request: CreateMemoryRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            source: request.source,
        }
    }
}

#[command(
    name = "create_memory",
    category = "memories",
    description = "Create a workspace memory in the current organization.",
    method = "POST",
    path = "/v1/platform/memories",
    policy = MEMORY_MANAGE,
    cli = CliRoute::new(&["memories"], "create").with_examples(&[CliExample::new("Create a named memory a coordinator and its workers share", "everruns memories create --name everruns-maintenance --description 'Decisions and rules for repository maintenance' --reason 'Housekeeping and Developer share decisions'",)]),
)]
impl Command for CreateMemory {
    type Output = MemoryResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let name = validate_name(&self.name)?;
        // A named memory is attached by name and mounted at /memory/<name>.
        everruns_contracts::runtime::memory::validate_store_name(&name)
            .map_err(|error| CommandError::bad_request(error.to_string()))?;
        let (source_type, source_config, is_readonly, sync_status) =
            create_memory_source(self.source)?;
        let input = CreateMemoryRow {
            public_id: MemoryId::new().to_string(),
            name,
            description: self.description,
            scope: "org".to_string(),
            owner_agent_id: None,
            owner_user_id: None,
            source_type,
            source_config,
            is_readonly,
            sync_status,
            owner_principal_id: None,
            resolved_owner_user_id: ctx.caller.user_id,
        };
        let row = ctx.db.create_memory(ctx.org_id(), input).await?;
        memory_response(row).map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetMemory {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

#[command(
    name = "get_memory",
    category = "memories",
    description = "Get a workspace memory by ID.",
    method = "GET",
    path = "/v1/platform/memories/{memory_id}",
    policy = MEMORY_VIEW,
    positional = "memory_id",
    cli = CliRoute::new(&["memories"], "get").with_args(&[CliArg::new("memory_id").at(1)]).with_examples(&[CliExample::new("Check a memory's source and sync state", "everruns memories get mem_01h9",)]),
)]
impl Command for GetMemory {
    type Output = MemoryResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let row = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateMemoryCmd {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
    #[serde(flatten)]
    pub request: UpdateMemoryRequest,
}

#[command(
    name = "update_memory",
    category = "memories",
    description = "Update a workspace memory.",
    method = "PATCH",
    path = "/v1/platform/memories/{memory_id}",
    policy = MEMORY_MANAGE,
    cli = CliRoute::new(&["memories"], "update").with_examples(&[CliExample::new("Rename a memory or change its description", "everruns memories update --memory-id mem_01h9 --description 'Design docs, main branch' --reason 'Clarify scope'",)]),
)]
impl Command for UpdateMemoryCmd {
    type Output = MemoryResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        let name = self
            .request
            .name
            .as_deref()
            .map(|name| {
                let name = validate_name(name)?;
                everruns_contracts::runtime::memory::validate_store_name(&name)
                    .map_err(|error| CommandError::bad_request(error.to_string()))?;
                Ok::<_, CommandError>(name)
            })
            .transpose()?;
        let source_update = self
            .request
            .source
            .map(|source| create_memory_source(Some(source)))
            .transpose()?;
        let row = ctx
            .db
            .update_memory(
                ctx.org_id(),
                existing.id,
                UpdateMemory {
                    name,
                    description: match self.request.description {
                        UpdateField::Set(description) => Some(Some(description)),
                        UpdateField::Clear => Some(None),
                        UpdateField::Unchanged => None,
                    },
                    status: None,
                    source_type: source_update
                        .as_ref()
                        .map(|(source_type, _, _, _)| source_type.clone()),
                    source_config: source_update
                        .as_ref()
                        .map(|(_, source_config, _, _)| source_config.clone()),
                    is_readonly: source_update
                        .as_ref()
                        .map(|(_, _, is_readonly, _)| *is_readonly),
                    sync_status: source_update
                        .as_ref()
                        .map(|(_, _, _, sync_status)| sync_status.clone()),
                    last_synced_at: source_update.as_ref().map(|_| None),
                    last_sync_error: source_update.as_ref().map(|_| None),
                    // THREAT[TM-AUTHZ-011]: a source edit must not keep syncing
                    // with the previous owner's GitHub connection. Rebind the
                    // sync owner to the caller who chose the new source, as
                    // Knowledge Indexes do; metadata edits leave it alone.
                    resolved_owner_user_id: source_update.as_ref().map(|_| ctx.caller.user_id),
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SyncMemoryNow {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

#[command(
    name = "sync_memory_now",
    category = "memories",
    description = "Queue an immediate sync for a source-backed workspace memory.",
    method = "POST",
    path = "/v1/platform/memories/{memory_id}/sync",
    policy = MEMORY_MANAGE,
    cli = CliRoute::new(&["memories"], "sync").with_examples(&[CliExample::new("Pull the latest source changes without waiting for the schedule", "everruns memories sync --memory-id mem_01h9 --reason 'Docs just changed'",)]),
)]
impl Command for SyncMemoryNow {
    type Output = MemoryResponse;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        if existing.source_type == "manual" {
            return Err(CommandError::bad_request(
                "manual memories do not have a source to sync",
            ));
        }
        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "only active source-backed memories can be synced",
            ));
        }
        let row = ctx
            .db
            .update_memory(
                ctx.org_id(),
                existing.id,
                UpdateMemory {
                    sync_status: Some("pending".to_string()),
                    last_sync_error: Some(None),
                    ..Default::default()
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteMemory {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

#[command(
    name = "delete_memory",
    category = "memories",
    description = "Archive a workspace memory.",
    method = "DELETE",
    path = "/v1/platform/memories/{memory_id}",
    policy = MEMORY_MANAGE,
    cli = CliRoute::new(&["memories"], "delete").with_examples(&[CliExample::new("Archive a memory agents should stop reading", "everruns memories delete --memory-id mem_01h9 --reason 'Docs moved to a new repo'",)]),
)]
impl Command for DeleteMemory {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        let archived = ctx.db.archive_memory(ctx.org_id(), existing.id).await?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("Memory"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
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
                read_only: false,
            },
            Arc::new(StorageBackend::test_database()),
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
                read_only: false,
            },
            db,
            None,
        )
    }

    #[tokio::test]
    async fn memory_lifecycle_round_trip() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateMemory {
            name: "team-memory".into(),
            description: Some("Shared working files".into()),
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create memory");

        assert!(created.id.to_string().starts_with("mem_"));
        assert_eq!(created.name, "team-memory");
        assert_eq!(created.status, "active");
        assert_eq!(created.source_type, "manual");
        assert!(!created.is_readonly);
        assert_eq!(created.sync_status, "idle");

        let listed = ListMemories {
            search: Some("memory".into()),
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list memories");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let updated = UpdateMemoryCmd {
            memory_id: created.id.to_string(),
            request: UpdateMemoryRequest {
                name: Some("team-files".into()),
                description: UpdateField::Set("Shared files".into()),
                source: None,
            },
        }
        .run(&ctx)
        .await
        .expect("update memory");
        assert_eq!(updated.name, "team-files");
        assert_eq!(updated.description.as_deref(), Some("Shared files"));

        let cleared = UpdateMemoryCmd {
            memory_id: created.id.to_string(),
            request: UpdateMemoryRequest {
                name: None,
                description: UpdateField::Clear,
                source: None,
            },
        }
        .run(&ctx)
        .await
        .expect("clear memory description");
        assert_eq!(cleared.name, "team-files");
        assert_eq!(cleared.description, None);

        DeleteMemory {
            memory_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive memory");

        let active = ListMemories {
            search: None,
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list active memories");
        assert!(active.is_empty());

        let archived = ListMemories {
            search: None,
            include_archived: Some(true),
        }
        .run(&ctx)
        .await
        .expect("list archived memories");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, "archived");
    }

    #[tokio::test]
    async fn memory_ids_do_not_cross_orgs() {
        let db = Arc::new(StorageBackend::test_database());
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateMemory {
            name: "private".into(),
            description: None,
            source: None,
        }
        .run(&org_one)
        .await
        .expect("create memory");

        let err = GetMemory {
            memory_id: created.id.to_string(),
        }
        .run(&org_two)
        .await
        .expect_err("other org should not read memory");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::NotFound(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn duplicate_active_memory_names_conflict() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        CreateMemory {
            name: "research".into(),
            description: None,
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create memory");

        let err = CreateMemory {
            name: "research".into(),
            description: None,
            source: None,
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
    async fn source_backed_and_unnamed_memories_are_refused() {
        let ctx = ctx_for_org(1);
        let source = CreateMemory {
            name: "design-docs".to_string(),
            description: None,
            source: Some(CreateMemorySourceRequest::Github(
                super::super::types::GitHubMemorySourceRequest {
                    repository: "acme/design-docs".to_string(),
                    branch: None,
                    root_folder: None,
                    sync_interval_secs: None,
                },
            )),
        }
        .execute(&ctx)
        .await
        .expect_err("sources are retired");
        assert!(source.to_string().contains("retired"), "{source}");

        let bad_name = CreateMemory {
            name: "Design Docs".to_string(),
            description: None,
            source: None,
        }
        .execute(&ctx)
        .await
        .expect_err("a named memory has a store name");
        assert!(bad_name.to_string().contains("not a memory"), "{bad_name}");
    }
}
