use super::types::{
    CreateVolumeRequest, CreateVolumeRow, CreateVolumeSourceRequest, GitHubVolumeSourceRequest,
    GitVolumeSourceRequest, ListVolumesQuery, UpdateVolume, UpdateVolumeRequest, VolumeResponse,
    volume_response,
};
use super::{VOLUME_MANAGE, VOLUME_VIEW};
use crate::domains::common::*;
use everruns_core::Policy;
use everruns_core::typed_id::VolumeId;
use everruns_core::url_validation::validate_safe_url;
use everruns_durable::UpdateField;
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

const DEFAULT_GIT_BRANCH: &str = "main";
const MIN_SYNC_INTERVAL_SECS: u32 = 300;
const MAX_SYNC_INTERVAL_SECS: u32 = 7 * 24 * 60 * 60;

fn validate_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::bad_request("Volume name cannot be empty"));
    }
    if trimmed.chars().count() > 255 {
        return Err(CommandError::bad_request(
            "Volume name must be at most 255 characters",
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

fn normalize_github_repository(repository: &str) -> Result<String, CommandError> {
    let trimmed = repository.trim().trim_end_matches(".git");
    let repo_path = if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path
    } else {
        trimmed
    };
    let repo_path = repo_path.trim_matches('/');
    let parts: Vec<&str> = repo_path.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || !part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
        })
    {
        return Err(CommandError::bad_request(
            "GitHub repository must be owner/repo or a github.com repository URL",
        ));
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
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

fn github_source_config(request: GitHubVolumeSourceRequest) -> Result<Value, CommandError> {
    let repository = normalize_github_repository(&request.repository)?;
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

fn git_source_config(request: GitVolumeSourceRequest) -> Result<Value, CommandError> {
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

fn create_volume_source(
    source: Option<CreateVolumeSourceRequest>,
) -> Result<(String, Value, bool, String), CommandError> {
    match source {
        None => Ok(("manual".to_string(), json!({}), false, "idle".to_string())),
        Some(CreateVolumeSourceRequest::Github(request)) => Ok((
            "github".to_string(),
            github_source_config(request)?,
            true,
            "pending".to_string(),
        )),
        Some(CreateVolumeSourceRequest::Git(request)) => Ok((
            "git".to_string(),
            git_source_config(request)?,
            true,
            "pending".to_string(),
        )),
    }
}

fn parse_volume_id(volume_id: &str) -> Result<VolumeId, CommandError> {
    volume_id
        .parse()
        .map_err(|_| CommandError::not_found("Volume"))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListVolumes {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl From<ListVolumesQuery> for ListVolumes {
    fn from(query: ListVolumesQuery) -> Self {
        Self {
            search: query.search,
            include_archived: query.include_archived,
        }
    }
}

impl Command for ListVolumes {
    type Output = Vec<VolumeResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_volumes",
            category: "volumes",
            description: "List workspace volumes in the current organization.",
            method: "GET",
            path: "/v1/volumes",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<VolumeResponse>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<VolumeResponse>, CommandError> {
        let rows = ctx
            .db
            .list_volumes(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived.unwrap_or(false),
            )
            .await
            .map_err(classify_anyhow)?;
        rows.into_iter()
            .map(|row| volume_response(row).map_err(classify_anyhow))
            .collect()
    }
}

inventory::submit! { CommandDescriptor::of::<ListVolumes>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVolume {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub source: Option<CreateVolumeSourceRequest>,
}

impl From<CreateVolumeRequest> for CreateVolume {
    fn from(request: CreateVolumeRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            source: request.source,
        }
    }
}

impl Command for CreateVolume {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_volume",
            category: "volumes",
            description: "Create a workspace volume in the current organization.",
            method: "POST",
            path: "/v1/volumes",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let name = validate_name(&self.name)?;
        let (source_type, source_config, is_readonly, sync_status) =
            create_volume_source(self.source)?;
        let input = CreateVolumeRow {
            public_id: VolumeId::new().to_string(),
            name,
            description: self.description,
            source_type,
            source_config,
            is_readonly,
            sync_status,
            owner_principal_id: None,
            resolved_owner_user_id: ctx.caller.user_id,
        };
        let row = ctx
            .db
            .create_volume(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateVolume>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetVolume {
    pub volume_id: String,
}

impl Command for GetVolume {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_volume",
            category: "volumes",
            description: "Get a workspace volume by ID.",
            method: "GET",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("volume_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let row = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetVolume>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateVolumeCmd {
    pub volume_id: String,
    #[serde(flatten)]
    pub request: UpdateVolumeRequest,
}

impl Command for UpdateVolumeCmd {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_volume",
            category: "volumes",
            description: "Update a workspace volume.",
            method: "PATCH",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let existing = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        let name = self
            .request
            .name
            .as_deref()
            .map(validate_name)
            .transpose()?;
        let source_update = self
            .request
            .source
            .map(|source| create_volume_source(Some(source)))
            .transpose()?;
        let row = ctx
            .db
            .update_volume(
                ctx.org_id(),
                existing.id,
                UpdateVolume {
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
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateVolumeCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncVolumeNow {
    pub volume_id: String,
}

impl Command for SyncVolumeNow {
    type Output = VolumeResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "sync_volume_now",
            category: "volumes",
            description: "Queue an immediate sync for a source-backed workspace volume.",
            method: "POST",
            path: "/v1/volumes/{volume_id}/sync",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        output_schema_for::<VolumeResponse>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<VolumeResponse, CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let existing = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        if existing.source_type == "manual" {
            return Err(CommandError::bad_request(
                "manual volumes do not have a source to sync",
            ));
        }
        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "only active source-backed volumes can be synced",
            ));
        }
        let row = ctx
            .db
            .update_volume(
                ctx.org_id(),
                existing.id,
                UpdateVolume {
                    sync_status: Some("pending".to_string()),
                    last_sync_error: Some(None),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        volume_response(row).map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<SyncVolumeNow>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteVolume {
    pub volume_id: String,
}

impl Command for DeleteVolume {
    type Output = ();

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_volume",
            category: "volumes",
            description: "Archive a workspace volume.",
            method: "DELETE",
            path: "/v1/volumes/{volume_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&VOLUME_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let id = parse_volume_id(&self.volume_id)?;
        let existing = ctx
            .db
            .get_volume(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Volume"))?;
        let archived = ctx
            .db
            .archive_volume(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;
        if archived {
            Ok(())
        } else {
            Err(CommandError::not_found("Volume"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteVolume>() }

#[cfg(test)]
mod tests {
    use super::super::types::VolumeSourceResponse;
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
    async fn volume_lifecycle_round_trip() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateVolume {
            name: "Team Memory".into(),
            description: Some("Shared working files".into()),
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create volume");

        assert!(created.id.to_string().starts_with("vol_"));
        assert_eq!(created.name, "Team Memory");
        assert_eq!(created.status, "active");
        assert_eq!(created.source_type, "manual");
        assert!(!created.is_readonly);
        assert_eq!(created.sync_status, "idle");

        let listed = ListVolumes {
            search: Some("memory".into()),
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list volumes");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let updated = UpdateVolumeCmd {
            volume_id: created.id.to_string(),
            request: UpdateVolumeRequest {
                name: Some("Team Files".into()),
                description: UpdateField::Set("Shared files".into()),
                source: None,
            },
        }
        .run(&ctx)
        .await
        .expect("update volume");
        assert_eq!(updated.name, "Team Files");
        assert_eq!(updated.description.as_deref(), Some("Shared files"));

        let cleared = UpdateVolumeCmd {
            volume_id: created.id.to_string(),
            request: UpdateVolumeRequest {
                name: None,
                description: UpdateField::Clear,
                source: None,
            },
        }
        .run(&ctx)
        .await
        .expect("clear volume description");
        assert_eq!(cleared.name, "Team Files");
        assert_eq!(cleared.description, None);

        DeleteVolume {
            volume_id: created.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("archive volume");

        let active = ListVolumes {
            search: None,
            include_archived: None,
        }
        .run(&ctx)
        .await
        .expect("list active volumes");
        assert!(active.is_empty());

        let archived = ListVolumes {
            search: None,
            include_archived: Some(true),
        }
        .run(&ctx)
        .await
        .expect("list archived volumes");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].status, "archived");
    }

    #[tokio::test]
    async fn volume_ids_do_not_cross_orgs() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_one = ctx_with_db(1, db.clone());
        let org_two = ctx_with_db(2, db);

        let created = CreateVolume {
            name: "Private".into(),
            description: None,
            source: None,
        }
        .run(&org_one)
        .await
        .expect("create volume");

        let err = GetVolume {
            volume_id: created.id.to_string(),
        }
        .run(&org_two)
        .await
        .expect_err("other org should not read volume");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::NotFound(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn duplicate_active_volume_names_conflict() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);
        CreateVolume {
            name: "Research".into(),
            description: None,
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create volume");

        let err = CreateVolume {
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
    async fn github_volume_is_readonly_and_pending_sync() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        let created = CreateVolume {
            name: "Repo Docs".into(),
            description: None,
            source: Some(CreateVolumeSourceRequest::Github(
                GitHubVolumeSourceRequest {
                    repository: "https://github.com/everruns/everruns.git".into(),
                    branch: Some("docs".into()),
                    root_folder: Some("/specs/".into()),
                    sync_interval_secs: Some(3600),
                },
            )),
        }
        .run(&ctx)
        .await
        .expect("create github volume");

        assert_eq!(created.source_type, "github");
        assert!(created.is_readonly);
        assert_eq!(created.sync_status, "pending");
        match created.source {
            VolumeSourceResponse::Github(source) => {
                assert_eq!(source.repository, "everruns/everruns");
                assert_eq!(source.branch, "docs");
                assert_eq!(source.root_folder.as_deref(), Some("specs"));
                assert_eq!(source.sync_interval_secs, Some(3600));
            }
            other => panic!("expected github source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_source_backed_volume_requeues_sync() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let created = CreateVolume {
            name: "Repo Docs".into(),
            description: None,
            source: Some(CreateVolumeSourceRequest::Github(
                GitHubVolumeSourceRequest {
                    repository: "everruns/everruns".into(),
                    branch: Some("main".into()),
                    root_folder: None,
                    sync_interval_secs: None,
                },
            )),
        }
        .run(&ctx)
        .await
        .expect("create github volume");
        let claimed = db
            .claim_next_volume_sync()
            .await
            .expect("claim sync")
            .expect("syncable volume");
        db.complete_volume_sync(claimed.id, claimed.updated_at, Vec::new())
            .await
            .expect("complete sync");

        let updated = UpdateVolumeCmd {
            volume_id: created.id.to_string(),
            request: UpdateVolumeRequest {
                name: None,
                description: UpdateField::Unchanged,
                source: Some(CreateVolumeSourceRequest::Git(GitVolumeSourceRequest {
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
            VolumeSourceResponse::Git(source) => {
                assert_eq!(source.branch, "release");
                assert_eq!(source.root_folder.as_deref(), Some("docs"));
                assert_eq!(source.sync_interval_secs, Some(900));
            }
            other => panic!("expected git source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sync_volume_now_requeues_source_volume() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_db(DEFAULT_ORG_ID, db.clone());

        let manual = CreateVolume {
            name: "Manual".into(),
            description: None,
            source: None,
        }
        .run(&ctx)
        .await
        .expect("create manual volume");
        let manual_err = SyncVolumeNow {
            volume_id: manual.id.to_string(),
        }
        .run(&ctx)
        .await
        .expect_err("manual volume cannot sync");
        assert!(matches!(
            manual_err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ));

        let created = CreateVolume {
            name: "Repo".into(),
            description: None,
            source: Some(CreateVolumeSourceRequest::Git(GitVolumeSourceRequest {
                url: "https://example.com/org/repo.git".into(),
                branch: None,
                root_folder: None,
                sync_interval_secs: None,
            })),
        }
        .run(&ctx)
        .await
        .expect("create git volume");
        let claimed = db
            .claim_next_volume_sync()
            .await
            .expect("claim sync")
            .expect("syncable volume");
        db.complete_volume_sync(claimed.id, claimed.updated_at, Vec::new())
            .await
            .expect("complete sync");

        let queued = SyncVolumeNow {
            volume_id: created.id.to_string(),
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

        let created = CreateVolume {
            name: "Scheduled Repo".into(),
            description: None,
            source: Some(CreateVolumeSourceRequest::Git(GitVolumeSourceRequest {
                url: "https://example.com/org/repo.git".into(),
                branch: None,
                root_folder: None,
                sync_interval_secs: Some(300),
            })),
        }
        .run(&ctx)
        .await
        .expect("create git volume");
        db.update_volume(
            DEFAULT_ORG_ID,
            created.internal_id,
            UpdateVolume {
                sync_status: Some("synced".to_string()),
                last_synced_at: Some(Some(Utc::now() - Duration::seconds(301))),
                ..Default::default()
            },
        )
        .await
        .expect("mark old sync");

        let claimed = db
            .claim_next_volume_sync()
            .await
            .expect("claim scheduled sync")
            .expect("due volume should be claimed");
        assert_eq!(claimed.id, created.internal_id);
        assert_eq!(claimed.sync_status, "syncing");
    }

    #[tokio::test]
    async fn git_volume_rejects_secret_bearing_urls() {
        let ctx = ctx_for_org(DEFAULT_ORG_ID);

        for url in [
            "https://token@example.com/org/repo.git",
            "https://example.com/org/repo.git?token=secret",
            "https://example.com/org/repo.git#token=secret",
            "git@example.com:org/repo.git?token=secret",
        ] {
            let err = CreateVolume {
                name: format!("Unsafe Repo {url}"),
                description: None,
                source: Some(CreateVolumeSourceRequest::Git(GitVolumeSourceRequest {
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
    async fn git_volume_rejects_non_public_hosts_and_non_https_schemes() {
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
            let err = CreateVolume {
                name: format!("Unsafe Host Repo {url}"),
                description: None,
                source: Some(CreateVolumeSourceRequest::Git(GitVolumeSourceRequest {
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
