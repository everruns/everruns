use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use everruns_core::traits::UserConnectionResolver;
use git2::{AutotagOption, Cred, FetchOptions, RemoteCallbacks, build::RepoBuilder};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::task;
use uuid::Uuid;

use crate::domains::volumes::types::{
    GitHubVolumeSourceResponse, GitVolumeSourceResponse, VolumeSourceResponse,
};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateVolumeFileRow, VolumeRow};

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_MAX_FILES: usize = 5_000;
const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SYNC_ERROR_LEN: usize = 512;

#[derive(Debug, Clone)]
pub struct VolumeSourceSyncConfig {
    pub poll_interval: Duration,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl VolumeSourceSyncConfig {
    pub fn from_env() -> Self {
        Self {
            poll_interval: Duration::from_secs(env_u64(
                "VOLUME_SOURCE_SYNC_INTERVAL_SECS",
                DEFAULT_POLL_INTERVAL_SECS,
            )),
            max_files: env_usize("VOLUME_SOURCE_SYNC_MAX_FILES", DEFAULT_MAX_FILES),
            max_file_bytes: env_u64("VOLUME_SOURCE_SYNC_MAX_FILE_BYTES", DEFAULT_MAX_FILE_BYTES),
            max_total_bytes: env_u64(
                "VOLUME_SOURCE_SYNC_MAX_TOTAL_BYTES",
                DEFAULT_MAX_TOTAL_BYTES,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeSyncOutcome {
    Synced { volume_id: Uuid, file_count: usize },
    Failed { volume_id: Uuid, error: String },
}

#[derive(Clone)]
pub struct VolumeSourceSyncService {
    db: Arc<StorageBackend>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    config: VolumeSourceSyncConfig,
}

impl VolumeSourceSyncService {
    pub fn new(
        db: Arc<StorageBackend>,
        connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
        config: VolumeSourceSyncConfig,
    ) -> Self {
        Self {
            db,
            connection_resolver,
            config,
        }
    }

    pub async fn run_once(&self) -> Result<Option<VolumeSyncOutcome>> {
        let Some(volume) = self.db.claim_next_volume_sync().await? else {
            return Ok(None);
        };

        let volume_id = volume.id;
        let claimed_at = volume.updated_at;
        match snapshot_volume_source(
            volume,
            self.connection_resolver.clone(),
            self.config.clone(),
        )
        .await
        {
            Ok(files) => {
                let file_count = files.len();
                match self
                    .db
                    .complete_volume_sync(volume_id, claimed_at, files)
                    .await
                {
                    Ok(Some(_)) => Ok(Some(VolumeSyncOutcome::Synced {
                        volume_id,
                        file_count,
                    })),
                    Ok(None) => Ok(None),
                    Err(error) => {
                        let message = sanitize_sync_error(&error);
                        let _ = self
                            .db
                            .fail_volume_sync(volume_id, claimed_at, &message)
                            .await;
                        Ok(Some(VolumeSyncOutcome::Failed {
                            volume_id,
                            error: message,
                        }))
                    }
                }
            }
            Err(error) => {
                let message = sanitize_sync_error(&error);
                match self
                    .db
                    .fail_volume_sync(volume_id, claimed_at, &message)
                    .await?
                {
                    Some(_) => Ok(Some(VolumeSyncOutcome::Failed {
                        volume_id,
                        error: message,
                    })),
                    None => Ok(None),
                }
            }
        }
    }
}

pub fn spawn_volume_source_sync_task(
    db: Arc<StorageBackend>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
) {
    if !env_bool("VOLUME_SOURCE_SYNC_ENABLED", true) {
        tracing::info!("Volume source sync background task disabled");
        return;
    }

    let config = VolumeSourceSyncConfig::from_env();
    if config.poll_interval.is_zero() {
        tracing::info!("Volume source sync background task disabled by zero interval");
        return;
    }

    let service = VolumeSourceSyncService::new(db, connection_resolver, config.clone());
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.poll_interval);
        tracing::info!(
            interval_secs = config.poll_interval.as_secs(),
            max_files = config.max_files,
            max_file_bytes = config.max_file_bytes,
            max_total_bytes = config.max_total_bytes,
            "Started volume source sync background task"
        );

        loop {
            interval.tick().await;
            match service.run_once().await {
                Ok(Some(VolumeSyncOutcome::Synced {
                    volume_id,
                    file_count,
                })) => {
                    tracing::info!(%volume_id, file_count, "Volume source sync completed");
                }
                Ok(Some(VolumeSyncOutcome::Failed { volume_id, error })) => {
                    tracing::warn!(%volume_id, %error, "Volume source sync failed");
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "Volume source sync task failed");
                }
            }
        }
    });
}

async fn snapshot_volume_source(
    volume: VolumeRow,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    config: VolumeSourceSyncConfig,
) -> Result<Vec<CreateVolumeFileRow>> {
    let source: VolumeSourceResponse =
        serde_json::from_value(volume.source_config).context("invalid volume source config")?;

    let source = match source {
        VolumeSourceResponse::Github(source) => {
            // THREAT[TM-API-018]: GitHub tokens must not be persisted in volume source config.
            // Mitigation: resolve the owner's short-lived connection token only at sync time.
            let auth_token = match (connection_resolver, volume.resolved_owner_user_id) {
                (Some(resolver), Some(user_id)) => resolver
                    .get_connection_token_for_user(user_id, "github")
                    .await
                    .map_err(|_| anyhow!("failed to resolve GitHub connection"))?,
                _ => None,
            };
            ResolvedGitSource::from_github(source, auth_token)
        }
        VolumeSourceResponse::Git(source) => ResolvedGitSource::from_git(source),
        VolumeSourceResponse::Manual(_) => bail!("manual volumes do not have a source to sync"),
    };

    task::spawn_blocking(move || snapshot_git_source(source, config))
        .await
        .map_err(|_| anyhow!("volume source sync worker was interrupted"))?
}

fn snapshot_git_source(
    source: ResolvedGitSource,
    config: VolumeSourceSyncConfig,
) -> Result<Vec<CreateVolumeFileRow>> {
    let temp_dir = TempDir::new().map_err(|_| anyhow!("failed to prepare repository checkout"))?;
    clone_repository(&source, temp_dir.path())?;

    let root = resolve_root_folder(temp_dir.path(), source.root_folder.as_deref())?;
    snapshot_directory(&root, &SnapshotLimits::from_config(&config))
}

fn clone_repository(source: &ResolvedGitSource, checkout_dir: &Path) -> Result<()> {
    let mut builder = RepoBuilder::new();
    let mut fetch_options = FetchOptions::new();
    fetch_options.download_tags(AutotagOption::None);
    fetch_options.depth(1);
    if let Some(token) = source.auth_token.clone() {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            Cred::userpass_plaintext("x-access-token", &token)
        });
        fetch_options.remote_callbacks(callbacks);
    }
    builder.fetch_options(fetch_options);

    builder
        .branch(&source.branch)
        .clone(&source.url, checkout_dir)
        .map(|_| ())
        .map_err(|_| anyhow!("failed to clone repository"))
}

fn resolve_root_folder(checkout_dir: &Path, root_folder: Option<&str>) -> Result<PathBuf> {
    let Some(root_folder) = root_folder.filter(|folder| !folder.trim().is_empty()) else {
        return Ok(checkout_dir.to_path_buf());
    };
    if root_folder.split('/').any(|segment| segment == ".git") {
        bail!("configured root folder cannot point inside repository metadata");
    }

    let root = checkout_dir.join(root_folder);
    let canonical_checkout = checkout_dir
        .canonicalize()
        .map_err(|_| anyhow!("failed to read repository checkout"))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| anyhow!("configured root folder does not exist"))?;

    if !canonical_root.starts_with(&canonical_checkout) || !canonical_root.is_dir() {
        bail!("configured root folder is not a directory inside the repository");
    }

    Ok(canonical_root)
}

#[derive(Debug, Clone)]
struct ResolvedGitSource {
    url: String,
    branch: String,
    root_folder: Option<String>,
    auth_token: Option<String>,
}

impl ResolvedGitSource {
    fn from_github(source: GitHubVolumeSourceResponse, auth_token: Option<String>) -> Self {
        Self {
            url: format!("https://github.com/{}.git", source.repository),
            branch: source.branch,
            root_folder: source.root_folder,
            auth_token,
        }
    }

    fn from_git(source: GitVolumeSourceResponse) -> Self {
        Self {
            url: source.url,
            branch: source.branch,
            root_folder: source.root_folder,
            auth_token: None,
        }
    }
}

#[derive(Debug, Clone)]
struct SnapshotLimits {
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

impl SnapshotLimits {
    fn from_config(config: &VolumeSourceSyncConfig) -> Self {
        Self {
            max_files: config.max_files,
            max_file_bytes: config.max_file_bytes,
            max_total_bytes: config.max_total_bytes,
        }
    }
}

fn snapshot_directory(root: &Path, limits: &SnapshotLimits) -> Result<Vec<CreateVolumeFileRow>> {
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    collect_directory(root, root, limits, &mut files, &mut total_bytes)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    limits: &SnapshotLimits,
    files: &mut Vec<CreateVolumeFileRow>,
    total_bytes: &mut u64,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|_| anyhow!("failed to read repository snapshot"))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| anyhow!("failed to read repository snapshot"))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| anyhow!("failed to read repository snapshot"))?;

        // THREAT[TM-FS-001]: Repository-controlled symlinks can point outside root.
        // Mitigation: do not follow or ingest symlinks during volume snapshots.
        if metadata.file_type().is_symlink() {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow!("failed to read repository snapshot"))?;
        if should_skip_relative_path(relative) {
            continue;
        }
        let volume_path = volume_path(relative)?;

        if metadata.is_dir() {
            push_limited(
                files,
                CreateVolumeFileRow {
                    path: volume_path,
                    content: None,
                    is_directory: true,
                    content_hash: None,
                },
                limits,
            )?;
            collect_directory(root, &path, limits, files, total_bytes)?;
        } else if metadata.is_file() {
            let size = metadata.len();
            // THREAT[TM-DOS-012]: Source repositories can contain oversized payloads.
            // Mitigation: enforce per-file and total snapshot byte limits before ingest.
            if size > limits.max_file_bytes {
                bail!("repository file exceeds maximum sync size");
            }
            *total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| anyhow!("repository snapshot exceeds maximum sync size"))?;
            if *total_bytes > limits.max_total_bytes {
                bail!("repository snapshot exceeds maximum sync size");
            }

            let content =
                std::fs::read(&path).map_err(|_| anyhow!("failed to read repository snapshot"))?;
            let content_hash = hex::encode(Sha256::digest(&content));
            push_limited(
                files,
                CreateVolumeFileRow {
                    path: volume_path,
                    content: Some(content),
                    is_directory: false,
                    content_hash: Some(content_hash),
                },
                limits,
            )?;
        }
    }

    Ok(())
}

fn push_limited(
    files: &mut Vec<CreateVolumeFileRow>,
    file: CreateVolumeFileRow,
    limits: &SnapshotLimits,
) -> Result<()> {
    if files.len() >= limits.max_files {
        bail!("repository snapshot exceeds maximum file count");
    }
    files.push(file);
    Ok(())
}

fn should_skip_relative_path(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => name == ".git",
        _ => false,
    })
}

fn volume_path(relative: &Path) -> Result<String> {
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("repository contains unsupported file paths")),
            _ => Err(anyhow!("repository contains unsupported file paths")),
        })
        .collect::<Result<Vec<_>>>()?;

    if parts.is_empty() {
        bail!("repository contains unsupported file paths");
    }

    Ok(format!("/{}", parts.join("/")))
}

fn sanitize_sync_error(error: &anyhow::Error) -> String {
    let mut message = error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| "volume source sync failed".to_string());
    message = message.replace('\n', " ");
    if message.len() > MAX_SYNC_ERROR_LEN {
        message.truncate(MAX_SYNC_ERROR_LEN);
    }
    message
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateVolumeRow;
    use everruns_core::DEFAULT_ORG_ID;
    use everruns_core::typed_id::VolumeId;

    #[test]
    fn snapshot_directory_maps_paths_and_skips_git_metadata() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let root = temp_dir.path();
        std::fs::create_dir_all(root.join(".git")).expect("git dir");
        std::fs::write(root.join(".git/config"), b"hidden").expect("git config");
        std::fs::create_dir_all(root.join("docs/nested")).expect("docs");
        std::fs::write(root.join("docs/README.md"), b"hello").expect("readme");
        std::fs::write(root.join("docs/nested/guide.md"), b"guide").expect("guide");

        let files = snapshot_directory(
            &root.join("docs"),
            &SnapshotLimits {
                max_files: 10,
                max_file_bytes: 1024,
                max_total_bytes: 2048,
            },
        )
        .expect("snapshot");

        let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, vec!["/README.md", "/nested", "/nested/guide.md"]);
        assert!(files.iter().all(|file| !file.path.contains(".git")));
    }

    #[test]
    fn snapshot_directory_enforces_total_size_limit() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp_dir.path().join("large.txt"), b"hello").expect("file");

        let error = snapshot_directory(
            temp_dir.path(),
            &SnapshotLimits {
                max_files: 10,
                max_file_bytes: 1024,
                max_total_bytes: 4,
            },
        )
        .expect_err("size limit");

        assert_eq!(
            sanitize_sync_error(&error),
            "repository snapshot exceeds maximum sync size"
        );
    }

    #[test]
    fn resolve_root_folder_rejects_git_metadata_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp_dir.path().join(".git/objects")).expect("git dir");

        let error =
            resolve_root_folder(temp_dir.path(), Some(".git")).expect_err("reject git root");

        assert_eq!(
            sanitize_sync_error(&error),
            "configured root folder cannot point inside repository metadata"
        );
    }

    #[tokio::test]
    async fn volume_sync_storage_claims_and_replaces_files() {
        let db = Arc::new(StorageBackend::in_memory());
        let created = db
            .create_volume(
                DEFAULT_ORG_ID,
                CreateVolumeRow {
                    public_id: VolumeId::new().to_string(),
                    name: "Repo".to_string(),
                    description: None,
                    source_type: "git".to_string(),
                    source_config: serde_json::json!({
                        "provider": "git",
                        "url": "https://example.com/repo.git",
                        "branch": "main",
                        "root_folder": null,
                    }),
                    is_readonly: true,
                    sync_status: "pending".to_string(),
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                },
            )
            .await
            .expect("volume");

        let claimed = db
            .claim_next_volume_sync()
            .await
            .expect("claim")
            .expect("pending volume");
        assert_eq!(claimed.id, created.id);
        assert_eq!(claimed.sync_status, "syncing");

        db.complete_volume_sync(
            claimed.id,
            claimed.updated_at,
            vec![CreateVolumeFileRow {
                path: "/README.md".to_string(),
                content: Some(b"hello".to_vec()),
                is_directory: false,
                content_hash: Some(hex::encode(Sha256::digest(b"hello"))),
            }],
        )
        .await
        .expect("complete")
        .expect("synced");

        let volume = db
            .get_volume_by_id(DEFAULT_ORG_ID, claimed.id)
            .await
            .expect("get")
            .expect("volume");
        assert_eq!(volume.sync_status, "synced");
        assert!(volume.last_synced_at.is_some());

        let files = db.list_all_volume_files(claimed.id).await.expect("files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/README.md");
        assert_eq!(files[0].content.as_deref(), Some(&b"hello"[..]));
    }

    #[tokio::test]
    async fn volume_sync_storage_rejects_stale_claim_completion() {
        let db = Arc::new(StorageBackend::in_memory());
        let created = db
            .create_volume(
                DEFAULT_ORG_ID,
                CreateVolumeRow {
                    public_id: VolumeId::new().to_string(),
                    name: "Repo".to_string(),
                    description: None,
                    source_type: "git".to_string(),
                    source_config: serde_json::json!({
                        "provider": "git",
                        "url": "https://example.com/repo.git",
                        "branch": "main",
                        "root_folder": null,
                    }),
                    is_readonly: true,
                    sync_status: "pending".to_string(),
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                },
            )
            .await
            .expect("volume");

        let claimed = db
            .claim_next_volume_sync()
            .await
            .expect("claim")
            .expect("pending volume");
        let stale_claimed_at = claimed.updated_at - chrono::Duration::seconds(1);

        let completed = db
            .complete_volume_sync(
                claimed.id,
                stale_claimed_at,
                vec![CreateVolumeFileRow {
                    path: "/README.md".to_string(),
                    content: Some(b"hello".to_vec()),
                    is_directory: false,
                    content_hash: Some(hex::encode(Sha256::digest(b"hello"))),
                }],
            )
            .await
            .expect("complete");
        assert!(completed.is_none());

        let volume = db
            .get_volume_by_id(DEFAULT_ORG_ID, created.id)
            .await
            .expect("get")
            .expect("volume");
        assert_eq!(volume.sync_status, "syncing");
        assert!(
            db.list_all_volume_files(claimed.id)
                .await
                .expect("files")
                .is_empty()
        );
    }
}
