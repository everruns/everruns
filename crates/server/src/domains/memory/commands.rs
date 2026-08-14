use super::types::{
    CreateMemoryRequest, CreateMemoryRow, CreateMemorySourceRequest, GitHubMemorySourceRequest,
    GitMemorySourceRequest, ListMemoriesQuery, MemoryResponse, UpdateMemory, UpdateMemoryRequest,
    memory_response,
};
use super::{MEMORY_MANAGE, MEMORY_VIEW};
use crate::domains::common::*;
use crate::domains::git_sources::normalize_github_repository;
use everruns_core::Policy;
use everruns_durable::UpdateField;
use everruns_provider::typed_id::MemoryId;
use everruns_provider::url_validation::validate_safe_url;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

const DEFAULT_GIT_BRANCH: &str = "main";
const MIN_SYNC_INTERVAL_SECS: u32 = 300;
const MAX_SYNC_INTERVAL_SECS: u32 = 7 * 24 * 60 * 60;

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

fn validate_optional_ref(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, CommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 255 || trimmed.contains('\0') {
        return Err(CommandError::bad_request(format!(
            "{field} must be a non-empty ref at most 255 characters"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_root_folder(value: Option<String>) -> Result<Option<String>, CommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim().trim_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Ok(None);
    }
    if trimmed.contains('\0')
        || trimmed.contains("//")
        || trimmed
            .split('/')
            .any(|segment| segment == ".." || segment.is_empty())
    {
        return Err(CommandError::bad_request(
            "root_folder must be a relative path without empty segments or '..'",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_sync_interval(value: Option<u32>) -> Result<Option<u32>, CommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value == 0 {
        return Ok(None);
    }
    if !(MIN_SYNC_INTERVAL_SECS..=MAX_SYNC_INTERVAL_SECS).contains(&value) {
        return Err(CommandError::bad_request(format!(
            "sync_interval_secs must be between {MIN_SYNC_INTERVAL_SECS} and {MAX_SYNC_INTERVAL_SECS}"
        )));
    }
    Ok(Some(value))
}

fn validate_git_url(url: &str) -> Result<String, CommandError> {
    let trimmed = url.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 || trimmed.contains(char::is_whitespace) {
        return Err(CommandError::bad_request(
            "Git URL must be a non-empty URL without whitespace",
        ));
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(CommandError::bad_request(
            "Git URL must not include query strings or fragments",
        ));
    }
    if let Ok(parsed) = url::Url::parse(trimmed) {
        if parsed.scheme() != "https" {
            return Err(CommandError::bad_request("Git URL must use https scheme"));
        }
        if parsed.password().is_some() || !parsed.username().is_empty() {
            return Err(CommandError::bad_request(
                "Git URL must not include inline credentials",
            ));
        }
        if validate_safe_url(trimmed).is_err() {
            return Err(CommandError::bad_request(
                "Git URL must target a public non-local host",
            ));
        }
        return Ok(trimmed.to_string());
    }
    Err(CommandError::bad_request(
        "Git URL must be an absolute https URL",
    ))
}

fn github_source_config(request: GitHubMemorySourceRequest) -> Result<Value, CommandError> {
    let repository =
        normalize_github_repository(&request.repository).map_err(CommandError::bad_request)?;
    let branch = validate_optional_ref(request.branch, "branch")?
        .unwrap_or_else(|| DEFAULT_GIT_BRANCH.to_string());
    let root_folder = normalize_root_folder(request.root_folder)?;
    let sync_interval_secs = validate_sync_interval(request.sync_interval_secs)?;
    Ok(json!({
        "provider": "github",
        "repository": repository,
        "branch": branch,
        "root_folder": root_folder,
        "sync_interval_secs": sync_interval_secs,
    }))
}

fn git_source_config(request: GitMemorySourceRequest) -> Result<Value, CommandError> {
    let url = validate_git_url(&request.url)?;
    let branch = validate_optional_ref(request.branch, "branch")?
        .unwrap_or_else(|| DEFAULT_GIT_BRANCH.to_string());
    let root_folder = normalize_root_folder(request.root_folder)?;
    let sync_interval_secs = validate_sync_interval(request.sync_interval_secs)?;
    Ok(json!({
        "provider": "git",
        "url": url,
        "branch": branch,
        "root_folder": root_folder,
        "sync_interval_secs": sync_interval_secs,
    }))
}

fn create_memory_source(
    source: Option<CreateMemorySourceRequest>,
) -> Result<(String, Value, bool, String), CommandError> {
    match source {
        None => Ok(("manual".to_string(), json!({}), false, "idle".to_string())),
        Some(CreateMemorySourceRequest::Github(request)) => Ok((
            "github".to_string(),
            github_source_config(request)?,
            true,
            "pending".to_string(),
        )),
        Some(CreateMemorySourceRequest::Git(request)) => Ok((
            "git".to_string(),
            git_source_config(request)?,
            true,
            "pending".to_string(),
        )),
    }
}

fn parse_memory_id(memory_id: &str) -> Result<MemoryId, CommandError> {
    memory_id
        .parse()
        .map_err(|_| CommandError::not_found("Memory"))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListMemories {
    #[serde(default)]
    pub search: Option<String>,
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

impl Command for ListMemories {
    type Output = Vec<MemoryResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_memories",
            category: "memories",
            description: "List workspace memories in the current organization.",
            method: "GET",
            path: "/v1/memories",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_VIEW)
    }

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
            .await
            .map_err(classify_anyhow)?;
        rows.into_iter()
            .map(|row| memory_response(row).map_err(classify_anyhow))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListMemories>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMemory {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
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

impl Command for CreateMemory {
    type Output = MemoryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_memory",
            category: "memories",
            description: "Create a workspace memory in the current organization.",
            method: "POST",
            path: "/v1/memories",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let name = validate_name(&self.name)?;
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
        let row = ctx
            .db
            .create_memory(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        memory_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateMemory>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetMemory {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

impl Command for GetMemory {
    type Output = MemoryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_memory",
            category: "memories",
            description: "Get a workspace memory by ID.",
            method: "GET",
            path: "/v1/memories/{memory_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("memory_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let row = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetMemory>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemoryCmd {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
    #[serde(flatten)]
    pub request: UpdateMemoryRequest,
}

impl Command for UpdateMemoryCmd {
    type Output = MemoryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_memory",
            category: "memories",
            description: "Update a workspace memory.",
            method: "PATCH",
            path: "/v1/memories/{memory_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        let name = self
            .request
            .name
            .as_deref()
            .map(validate_name)
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
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateMemoryCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncMemoryNow {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

impl Command for SyncMemoryNow {
    type Output = MemoryResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "sync_memory_now",
            category: "memories",
            description: "Queue an immediate sync for a source-backed workspace memory.",
            method: "POST",
            path: "/v1/memories/{memory_id}/sync",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryResponse, CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        memory_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<SyncMemoryNow>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteMemory {
    /// Workspace memory's prefixed public identifier.
    pub memory_id: String,
}

impl Command for DeleteMemory {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_memory",
            category: "memories",
            description: "Archive a workspace memory.",
            method: "DELETE",
            path: "/v1/memories/{memory_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MEMORY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_memory_id(&self.memory_id)?;
        let existing = ctx
            .db
            .get_memory(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Memory"))?;
        let archived = ctx
            .db
            .archive_memory(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("Memory"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteMemory>() }

#[cfg(test)]
mod tests {
    use super::super::types::MemorySourceResponse;
    use super::*;
    use crate::domains::common::Ctx;
    use crate::storage::StorageBackend;
    use chrono::{Duration, Utc};
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

    #[tokio::test]
    async fn memory_lifecycle_round_trip() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateMemory {
            name: "Team Memory".into(),
            description: Some("Shared working files".into()),
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create memory");

        assert!(created.id.to_string().starts_with("mem_"));
        assert_eq!(created.name, "Team Memory");
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
                name: Some("Team Files".into()),
                description: UpdateField::Set("Shared files".into()),
                source: None,
            },
        }
        .run(&ctx)
        .await
        .expect("update memory");
        assert_eq!(updated.name, "Team Files");
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
        assert_eq!(cleared.name, "Team Files");
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
        let db = Arc::new(StorageBackend::in_memory());
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateMemory {
            name: "Private".into(),
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
            name: "Research".into(),
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
    async fn github_memory_is_readonly_and_pending_sync() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateMemory {
            name: "Repo Docs".into(),
            description: None,
            source: Some(CreateMemorySourceRequest::Github(
                GitHubMemorySourceRequest {
                    repository: "https://github.com/everruns/everruns.git".into(),
                    branch: Some("docs".into()),
                    root_folder: Some("/specs/".into()),
                    sync_interval_secs: Some(3600),
                },
            )),
        }
        .run(&ctx)
        .await
        .expect("create github memory");

        assert_eq!(created.source_type, "github");
        assert!(created.is_readonly);
        assert_eq!(created.sync_status, "pending");
        match created.source {
            MemorySourceResponse::Github(source) => {
                assert_eq!(source.repository, "everruns/everruns");
                assert_eq!(source.branch, "docs");
                assert_eq!(source.root_folder.as_deref(), Some("specs"));
                assert_eq!(source.sync_interval_secs, Some(3600));
            }
            other => panic!("expected github source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_source_backed_memory_requeues_sync() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let created = CreateMemory {
            name: "Repo Docs".into(),
            description: None,
            source: Some(CreateMemorySourceRequest::Github(
                GitHubMemorySourceRequest {
                    repository: "everruns/everruns".into(),
                    branch: Some("main".into()),
                    root_folder: None,
                    sync_interval_secs: None,
                },
            )),
        }
        .run(&ctx)
        .await
        .expect("create github memory");
        let claimed = db
            .claim_next_memory_sync()
            .await
            .expect("claim sync")
            .expect("syncable memory");
        db.complete_memory_sync(claimed.id, claimed.updated_at, Vec::new())
            .await
            .expect("complete sync");

        let updated = UpdateMemoryCmd {
            memory_id: created.id.to_string(),
            request: UpdateMemoryRequest {
                name: None,
                description: UpdateField::Unchanged,
                source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                    url: "https://example.com/org/repo.git".into(),
                    branch: Some("release".into()),
                    root_folder: Some("docs".into()),
                    sync_interval_secs: Some(900),
                })),
            },
        }
        .run(&ctx)
        .await
        .expect("update source");

        assert_eq!(updated.source_type, "git");
        assert_eq!(updated.sync_status, "pending");
        assert_eq!(updated.last_synced_at, None);
        match updated.source {
            MemorySourceResponse::Git(source) => {
                assert_eq!(source.branch, "release");
                assert_eq!(source.root_folder.as_deref(), Some("docs"));
                assert_eq!(source.sync_interval_secs, Some(900));
            }
            other => panic!("expected git source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sync_memory_now_requeues_source_volume() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let manual = CreateMemory {
            name: "Manual".into(),
            description: None,
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create manual memory");
        let manual_err = SyncMemoryNow {
            memory_id: manual.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect_err("manual memory cannot sync");
        assert!(matches!(
            manual_err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let created = CreateMemory {
            name: "Repo".into(),
            description: None,
            source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                url: "https://example.com/org/repo.git".into(),
                branch: None,
                root_folder: None,
                sync_interval_secs: None,
            })),
        }
        .run(&ctx)
        .await
        .expect("create git memory");
        let claimed = db
            .claim_next_memory_sync()
            .await
            .expect("claim sync")
            .expect("syncable memory");
        db.complete_memory_sync(claimed.id, claimed.updated_at, Vec::new())
            .await
            .expect("complete sync");

        let queued = SyncMemoryNow {
            memory_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("queue sync");
        assert_eq!(queued.sync_status, "pending");
        assert_eq!(queued.last_sync_error, None);
    }

    #[tokio::test]
    async fn due_sync_interval_claims_source_volume() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let created = CreateMemory {
            name: "Scheduled Repo".into(),
            description: None,
            source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                url: "https://example.com/org/repo.git".into(),
                branch: None,
                root_folder: None,
                sync_interval_secs: Some(300),
            })),
        }
        .run(&ctx)
        .await
        .expect("create git memory");
        db.update_memory(
            DEFAULT_ORG_ID,
            created.internal_id,
            UpdateMemory {
                sync_status: Some("synced".to_string()),
                last_synced_at: Some(Some(Utc::now() - Duration::seconds(301))),
                ..Default::default()
            },
        )
        .await
        .expect("mark old sync");

        let claimed = db
            .claim_next_memory_sync()
            .await
            .expect("claim scheduled sync")
            .expect("due memory should be claimed");
        assert_eq!(claimed.id, created.internal_id);
        assert_eq!(claimed.sync_status, "syncing");
    }

    #[tokio::test]
    async fn git_memory_rejects_secret_bearing_urls() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        for url in [
            "https://token@example.com/org/repo.git",
            "https://example.com/org/repo.git?token=secret",
            "https://example.com/org/repo.git#token=secret",
            "git@example.com:org/repo.git?token=secret",
        ] {
            let err = CreateMemory {
                name: format!("Unsafe Repo {url}"),
                description: None,
                source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                    url: url.into(),
                    branch: None,
                    root_folder: None,
                    sync_interval_secs: None,
                })),
            }
            .run(&ctx)
            .await
            .expect_err("secret-bearing URL should fail");

            assert!(matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::BadRequest(_),
                    ..
                }
            ));
        }
    }

    #[tokio::test]
    async fn git_memory_rejects_non_public_hosts_and_non_https_schemes() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        for url in [
            "https://127.0.0.1/repo.git",
            "https://10.0.0.5/repo.git",
            "https://169.254.169.254/repo.git",
            "https://localhost/repo.git",
            "ssh://git@example.com/org/repo.git",
            "git://example.com/org/repo.git",
            "git@example.com:org/repo.git",
        ] {
            let err = CreateMemory {
                name: format!("Unsafe Host Repo {url}"),
                description: None,
                source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
                    url: url.into(),
                    branch: None,
                    root_folder: None,
                    sync_interval_secs: None,
                })),
            }
            .run(&ctx)
            .await
            .expect_err("unsafe git URL should fail");

            assert!(matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::BadRequest(_),
                    ..
                }
            ));
        }
    }
}
