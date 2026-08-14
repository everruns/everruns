//! Knowledge Index Syncout pipeline. See knowledge/runtime-resources/knowledge-indexes.md.
//!
//! Mirrors the source-backed Memory sync worker
//! (`crates/server/src/domains/memory/source_sync.rs`): a background poll task
//! claims the next pending index, shallow-clones the configured GitHub source,
//! enumerates text documents, chunks + embeds them, persists documents + chunks
//! to Postgres (`complete_knowledge_index_sync`), and writes vectors into the
//! platform vector store namespace. On any failure it records a sanitized error
//! via `fail_knowledge_index_sync`, preserving previously indexed content.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;

use everruns_core::connection_services::UserConnectionResolver;
use everruns_platform::vector_store::{VectorRecord, VectorStore};
use everruns_provider::driver_registry::{DriverRegistry, EmbedRequest};
use everruns_provider::typed_id::{KnowledgeIndexChunkId, KnowledgeIndexDocumentId};
use git2::{AutotagOption, Cred, FetchOptions, RemoteCallbacks, build::RepoBuilder};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::task;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::domains::git_sources::{github_clone_url, safe_git_clone_error};
use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateKnowledgeIndexChunkRow, CreateKnowledgeIndexDocumentWithChunks, KnowledgeIndexRow,
};

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_MAX_FILES: usize = 5_000;
const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SYNC_ERROR_LEN: usize = 512;

/// Chunking window: ~1500 chars per chunk with 150 chars of overlap. Char-based
/// (not token-based) keeps the OSS pipeline dependency-free; token counts are
/// approximated for telemetry only. See `chunk_text`.
const CHUNK_SIZE_CHARS: usize = 1500;
const CHUNK_OVERLAP_CHARS: usize = 150;
/// Embedding batch size: how many chunks are sent to the embeddings driver per
/// request.
const EMBED_BATCH_SIZE: usize = 64;
/// Max embedding batches in flight at once (EVE-635). Bounds fan-out so a large
/// document does not open an unbounded number of concurrent provider requests.
const EMBED_BATCH_CONCURRENCY: usize = 4;

#[derive(Debug, Clone)]
pub struct KnowledgeIndexSyncConfig {
    pub poll_interval: Duration,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl KnowledgeIndexSyncConfig {
    pub fn from_env() -> Self {
        Self {
            poll_interval: Duration::from_secs(env_u64(
                "KNOWLEDGE_INDEX_SYNC_INTERVAL_SECS",
                DEFAULT_POLL_INTERVAL_SECS,
            )),
            max_files: env_usize("KNOWLEDGE_INDEX_SYNC_MAX_FILES", DEFAULT_MAX_FILES),
            max_file_bytes: env_u64(
                "KNOWLEDGE_INDEX_SYNC_MAX_FILE_BYTES",
                DEFAULT_MAX_FILE_BYTES,
            ),
            max_total_bytes: env_u64(
                "KNOWLEDGE_INDEX_SYNC_MAX_TOTAL_BYTES",
                DEFAULT_MAX_TOTAL_BYTES,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeIndexSyncOutcome {
    Synced {
        index_id: Uuid,
        document_count: usize,
        chunk_count: usize,
    },
    Failed {
        index_id: Uuid,
        error: String,
    },
}

/// Dependencies the sync pipeline resolves at runtime.
#[derive(Clone)]
pub struct KnowledgeIndexSyncService {
    db: Arc<StorageBackend>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    provider_resolver: Arc<ProviderResolverService>,
    driver_registry: Arc<DriverRegistry>,
    vector_store: Arc<dyn VectorStore>,
    config: KnowledgeIndexSyncConfig,
}

impl KnowledgeIndexSyncService {
    pub fn new(
        db: Arc<StorageBackend>,
        connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
        provider_resolver: Arc<ProviderResolverService>,
        driver_registry: Arc<DriverRegistry>,
        vector_store: Arc<dyn VectorStore>,
        config: KnowledgeIndexSyncConfig,
    ) -> Self {
        Self {
            db,
            connection_resolver,
            provider_resolver,
            driver_registry,
            vector_store,
            config,
        }
    }

    pub async fn run_once(&self) -> Result<Option<KnowledgeIndexSyncOutcome>> {
        let Some(index) = self.db.claim_next_knowledge_index_sync().await? else {
            return Ok(None);
        };

        let index_id = index.id;
        let claimed_at = index.updated_at;

        match self.sync_index(&index).await {
            Ok(prepared) => self.persist(index, claimed_at, prepared).await,
            Err(error) => {
                let message = sanitize_sync_error(&error);
                match self
                    .db
                    .fail_knowledge_index_sync(index_id, claimed_at, &message)
                    .await?
                {
                    Some(_) => Ok(Some(KnowledgeIndexSyncOutcome::Failed {
                        index_id,
                        error: message,
                    })),
                    None => Ok(None),
                }
            }
        }
    }

    /// Persist Postgres documents/chunks then reconcile the vector store. A
    /// failure after the Postgres claim was lost (stale claim) yields None.
    async fn persist(
        &self,
        index: KnowledgeIndexRow,
        claimed_at: chrono::DateTime<chrono::Utc>,
        prepared: PreparedSync,
    ) -> Result<Option<KnowledgeIndexSyncOutcome>> {
        let index_id = index.id;
        let document_count = prepared.documents.len();
        let chunk_count: usize = prepared.documents.iter().map(|d| d.chunks.len()).sum();

        let result = self
            .db
            .complete_knowledge_index_sync(
                index_id,
                claimed_at,
                prepared.documents,
                prepared.vector_dim,
            )
            .await;

        match result {
            Ok(Some(_)) => {
                // Postgres is the source of truth; the vector store is a
                // rebuildable projection. Reconcile it after the commit.
                if let Some(namespace) = index.vector_namespace.as_deref()
                    && let Err(error) = self.write_vectors(namespace, prepared.records).await
                {
                    let message = sanitize_sync_error(&error);
                    let _ = self
                        .db
                        .fail_knowledge_index_sync(index_id, claimed_at, &message)
                        .await;
                    return Ok(Some(KnowledgeIndexSyncOutcome::Failed {
                        index_id,
                        error: message,
                    }));
                }
                Ok(Some(KnowledgeIndexSyncOutcome::Synced {
                    index_id,
                    document_count,
                    chunk_count,
                }))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                let message = sanitize_sync_error(&error);
                let _ = self
                    .db
                    .fail_knowledge_index_sync(index_id, claimed_at, &message)
                    .await;
                Ok(Some(KnowledgeIndexSyncOutcome::Failed {
                    index_id,
                    error: message,
                }))
            }
        }
    }

    /// Replace a namespace's vectors with the freshly embedded records. A full
    /// re-sync always reconciles the projection (knowledge/runtime-resources/knowledge-indexes.md).
    async fn write_vectors(&self, namespace: &str, records: Vec<VectorRecord>) -> Result<()> {
        self.vector_store.delete_namespace(namespace).await?;
        if !records.is_empty() {
            self.vector_store.upsert(namespace, records).await?;
        }
        Ok(())
    }

    /// Clone the source, extract+chunk documents, then embed every chunk.
    async fn sync_index(&self, index: &KnowledgeIndexRow) -> Result<PreparedSync> {
        let source = self.resolve_source(index).await?;
        let config = self.config.clone();
        let docs = task::spawn_blocking(move || snapshot_documents(source, config))
            .await
            .map_err(|_| anyhow!("knowledge index sync worker was interrupted"))??;

        self.embed_documents(index, docs).await
    }

    async fn resolve_source(&self, index: &KnowledgeIndexRow) -> Result<ResolvedGitSource> {
        let source: GithubSourceConfig = serde_json::from_value(index.source_config.clone())
            .context("invalid knowledge index source config")?;
        // Normalize before touching connections so malformed coordinates are
        // reported as input errors rather than misleading authentication errors.
        let mut resolved = ResolvedGitSource::from_github(source, None)?;

        // THREAT[TM-API-018]: GitHub tokens must not be persisted in the index
        // source config. Mitigation: resolve the owner's short-lived connection
        // token only at sync time.
        let auth_token = match (
            self.connection_resolver.clone(),
            index.resolved_owner_user_id,
        ) {
            (Some(resolver), Some(user_id)) => resolver
                .get_connection_token_for_user(user_id, "github")
                .await
                .map_err(|_| anyhow!("failed to resolve GitHub connection"))?,
            _ => None,
        };
        resolved.auth_token = auth_token;
        Ok(resolved)
    }

    /// Build the embeddings driver from the index's embedding model and embed
    /// every chunk, producing both the Postgres rows and the vector records.
    async fn embed_documents(
        &self,
        index: &KnowledgeIndexRow,
        docs: Vec<ExtractedDocument>,
    ) -> Result<PreparedSync> {
        // Resolve credentials for the embeddings service, bound to the model's
        // provider connection (knowledge/foundations/providers.md resolve_service). Shared with
        // the retrieval slice via `embedding::build_embeddings_driver`.
        let embedder = super::embedding::build_embeddings_driver(
            &self.db,
            &self.provider_resolver,
            &self.driver_registry,
            index,
        )
        .await?;
        // EVE-635: share the driver across concurrently in-flight embedding
        // batches (bounded fan-out below). `BoxedEmbeddingsDriver` is not Clone,
        // so wrap it once in an Arc.
        let driver = Arc::new(embedder.driver);
        let model_id = embedder.model_id.clone();

        let mut documents = Vec::with_capacity(docs.len());
        let mut records = Vec::new();
        let mut vector_dim: Option<i32> = None;
        // EVE-894: embedding usage for this sync run. Previously discarded
        // outright; accumulated here so the spend is at least observable.
        let mut embed_tokens: u64 = 0;
        let mut embed_cost_usd: Option<f64> = None;

        for doc in docs {
            let pieces = chunk_text(&doc.text, CHUNK_SIZE_CHARS, CHUNK_OVERLAP_CHARS);
            let mut chunk_rows = Vec::with_capacity(pieces.len());

            // Embed in batches to bound request size. EVE-635: run batches with
            // bounded concurrency rather than strictly one-at-a-time, then
            // reassemble in batch order so embeddings still line up with chunks.
            let batch_inputs: Vec<(usize, Vec<String>)> = pieces
                .chunks(EMBED_BATCH_SIZE)
                .enumerate()
                .map(|(i, batch)| (i, batch.iter().map(|p| p.text.clone()).collect()))
                .collect();
            let mut batch_results: Vec<Option<Vec<Vec<f32>>>> = vec![None; batch_inputs.len()];

            let mut embed_stream =
                futures::stream::iter(batch_inputs.into_iter().map(|(i, texts)| {
                    let driver = Arc::clone(&driver);
                    let model = model_id.clone();
                    async move {
                        let expected = texts.len();
                        let response = driver
                            .embed(
                                &everruns_provider::runtime_provider::ProviderEndpoint::default(),
                                EmbedRequest { texts, model },
                            )
                            .await
                            .map_err(|e| anyhow!("embedding request failed: {e}"))?;
                        if response.embeddings.len() != expected {
                            bail!("embeddings provider returned a mismatched batch size");
                        }
                        Ok::<(usize, Vec<Vec<f32>>, Option<u32>, Option<f64>), anyhow::Error>((
                            i,
                            response.embeddings,
                            response.usage_tokens,
                            response.actual_cost_usd,
                        ))
                    }
                }))
                .buffer_unordered(EMBED_BATCH_CONCURRENCY);

            while let Some(result) = embed_stream.next().await {
                let (i, batch_embeddings, usage_tokens, actual_cost_usd) = result?;
                batch_results[i] = Some(batch_embeddings);
                // EVE-894: sync-time embedding usage is accumulated per run and
                // reported below. It is not yet written to `llm_generations`:
                // that table requires a session and index sync is host-triggered
                // background work with none. See the follow-up issue.
                embed_tokens = embed_tokens.saturating_add(u64::from(usage_tokens.unwrap_or(0)));
                if let Some(cost) = actual_cost_usd {
                    embed_cost_usd = Some(embed_cost_usd.unwrap_or(0.0) + cost);
                }
            }

            // Flatten batches back into a single per-chunk order.
            let embeddings: Vec<Vec<f32>> = batch_results.into_iter().flatten().flatten().collect();

            let document_public_id = KnowledgeIndexDocumentId::new().to_string();
            for (ordinal, (piece, embedding)) in pieces.into_iter().zip(embeddings).enumerate() {
                if let Some(dim) = vector_dim {
                    if dim as usize != embedding.len() {
                        bail!("embeddings provider returned inconsistent dimensions");
                    }
                } else {
                    vector_dim = Some(embedding.len() as i32);
                }

                let chunk_public_id = KnowledgeIndexChunkId::new().to_string();
                records.push(VectorRecord {
                    id: chunk_public_id.clone(),
                    vector: embedding,
                    text: piece.text.clone(),
                    document_id: document_public_id.clone(),
                });
                chunk_rows.push(CreateKnowledgeIndexChunkRow {
                    public_id: chunk_public_id,
                    ordinal: ordinal as i32,
                    text: piece.text,
                    location: Some(serde_json::json!({
                        "char_start": piece.char_start,
                        "char_end": piece.char_end,
                    })),
                    token_count: Some(piece.token_count as i32),
                });
            }

            documents.push(CreateKnowledgeIndexDocumentWithChunks {
                public_id: document_public_id,
                source_uri: doc.source_uri,
                title: doc.title,
                mime_type: doc.mime_type,
                content_hash: Some(doc.content_hash),
                size_bytes: Some(doc.size_bytes as i64),
                chunks: chunk_rows,
            });
        }

        tracing::info!(
            index_id = %index.public_id,
            model = %model_id,
            provider = %embedder.provider_type,
            embed_tokens,
            embed_cost_usd,
            "knowledge index sync embedding usage"
        );

        Ok(PreparedSync {
            documents,
            records,
            vector_dim,
        })
    }
}

/// Result of a full sync pass, ready to persist.
struct PreparedSync {
    documents: Vec<CreateKnowledgeIndexDocumentWithChunks>,
    records: Vec<VectorRecord>,
    vector_dim: Option<i32>,
}

pub fn spawn_knowledge_index_sync_task(
    db: Arc<StorageBackend>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    provider_resolver: Arc<ProviderResolverService>,
    driver_registry: Arc<DriverRegistry>,
    vector_store: Arc<dyn VectorStore>,
) -> Option<JoinHandle<()>> {
    if !env_bool("KNOWLEDGE_INDEX_SYNC_ENABLED", true) {
        tracing::info!("Knowledge index sync background task disabled");
        return None;
    }

    let config = KnowledgeIndexSyncConfig::from_env();
    if config.poll_interval.is_zero() {
        tracing::info!("Knowledge index sync background task disabled by zero interval");
        return None;
    }

    let service = KnowledgeIndexSyncService::new(
        db,
        connection_resolver,
        provider_resolver,
        driver_registry,
        vector_store,
        config.clone(),
    );
    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.poll_interval);
        tracing::info!(
            interval_secs = config.poll_interval.as_secs(),
            "Started knowledge index sync background task"
        );
        loop {
            interval.tick().await;
            match service.run_once().await {
                Ok(Some(KnowledgeIndexSyncOutcome::Synced {
                    index_id,
                    document_count,
                    chunk_count,
                })) => {
                    tracing::info!(
                        %index_id,
                        document_count,
                        chunk_count,
                        "Knowledge index sync completed"
                    );
                }
                Ok(Some(KnowledgeIndexSyncOutcome::Failed { index_id, error })) => {
                    tracing::warn!(%index_id, %error, "Knowledge index sync failed");
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "Knowledge index sync task failed");
                }
            }
        }
    }))
}

// ============================================================================
// Source config + clone
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
struct GithubSourceConfig {
    repository: String,
    #[serde(default = "default_branch")]
    branch: String,
    #[serde(default)]
    root_folder: Option<String>,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone)]
struct ResolvedGitSource {
    url: String,
    repository: String,
    branch: String,
    root_folder: Option<String>,
    auth_token: Option<String>,
}

impl ResolvedGitSource {
    fn from_github(source: GithubSourceConfig, auth_token: Option<String>) -> Result<Self> {
        let (url, repository) =
            github_clone_url(&source.repository).map_err(|message| anyhow!(message))?;
        Ok(Self {
            url,
            repository,
            branch: source.branch,
            root_folder: source.root_folder,
            auth_token,
        })
    }
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
        .map_err(|error| {
            // Keep provider details in operator logs while returning only the
            // stable, credential-safe message stored on the index.
            tracing::warn!(
                git_error_code = ?error.code(),
                git_error_class = ?error.class(),
                git_error = %error.message(),
                "Git clone failed during knowledge index sync"
            );
            anyhow!(safe_git_clone_error(
                &error,
                true,
                source.auth_token.is_some()
            ))
        })
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

// ============================================================================
// Document extraction
// ============================================================================

/// A text document extracted from the source, ready to chunk + embed.
#[derive(Debug, Clone)]
struct ExtractedDocument {
    source_uri: String,
    title: Option<String>,
    mime_type: Option<String>,
    content_hash: String,
    size_bytes: u64,
    text: String,
}

#[derive(Debug, Clone)]
struct SnapshotLimits {
    max_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

fn snapshot_documents(
    source: ResolvedGitSource,
    config: KnowledgeIndexSyncConfig,
) -> Result<Vec<ExtractedDocument>> {
    let temp_dir = TempDir::new().map_err(|_| anyhow!("failed to prepare repository checkout"))?;
    clone_repository(&source, temp_dir.path())?;
    let root = resolve_root_folder(temp_dir.path(), source.root_folder.as_deref())?;

    let limits = SnapshotLimits {
        max_files: config.max_files,
        max_file_bytes: config.max_file_bytes,
        max_total_bytes: config.max_total_bytes,
    };
    let mut documents = Vec::new();
    let mut total_bytes = 0_u64;
    collect_documents(
        &root,
        &root,
        &source,
        &limits,
        &mut documents,
        &mut total_bytes,
    )?;
    documents.sort_by(|a, b| a.source_uri.cmp(&b.source_uri));
    Ok(documents)
}

fn collect_documents(
    root: &Path,
    directory: &Path,
    source: &ResolvedGitSource,
    limits: &SnapshotLimits,
    documents: &mut Vec<ExtractedDocument>,
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

        // THREAT[TM-FS-001]: repository-controlled symlinks can escape root.
        // Mitigation: never follow or ingest symlinks.
        if metadata.file_type().is_symlink() {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow!("failed to read repository snapshot"))?;
        if should_skip_relative_path(relative) {
            continue;
        }

        if metadata.is_dir() {
            collect_documents(root, &path, source, limits, documents, total_bytes)?;
        } else if metadata.is_file() {
            if !is_text_document(&path) {
                continue;
            }
            let size = metadata.len();
            // THREAT[TM-DOS-012]: source repos can contain oversized payloads.
            // Mitigation: enforce per-file and total byte limits before ingest.
            if size > limits.max_file_bytes {
                bail!("repository file exceeds maximum sync size");
            }
            let bytes =
                std::fs::read(&path).map_err(|_| anyhow!("failed to read repository snapshot"))?;
            // Skip files that are not valid UTF-8 text (likely binary).
            let Ok(text) = String::from_utf8(bytes.clone()) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }

            *total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| anyhow!("repository snapshot exceeds maximum sync size"))?;
            if *total_bytes > limits.max_total_bytes {
                bail!("repository snapshot exceeds maximum sync size");
            }
            if documents.len() >= limits.max_files {
                bail!("repository snapshot exceeds maximum file count");
            }

            let rel_path = relative_path_string(relative)?;
            let content_hash = hex::encode(Sha256::digest(&bytes));
            documents.push(ExtractedDocument {
                source_uri: format!(
                    "github://{}@{}/{}",
                    source.repository, source.branch, rel_path
                ),
                title: Some(rel_path),
                mime_type: Some(mime_for_path(&path).to_string()),
                content_hash,
                size_bytes: size,
                text,
            });
        }
    }
    Ok(())
}

fn should_skip_relative_path(relative: &Path) -> bool {
    relative.components().any(|component| match component {
        Component::Normal(name) => name == ".git",
        _ => false,
    })
}

fn relative_path_string(relative: &Path) -> Result<String> {
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
    Ok(parts.join("/"))
}

/// Text/markdown/code documents are indexed; everything else is skipped.
/// PDF/Office extraction is deferred (knowledge/runtime-resources/knowledge-indexes.md phase 6).
fn is_text_document(path: &Path) -> bool {
    const TEXT_EXTENSIONS: &[&str] = &[
        "md",
        "markdown",
        "mdx",
        "txt",
        "text",
        "rst",
        "adoc",
        "org",
        "html",
        "htm",
        "csv",
        "tsv",
        "json",
        "jsonl",
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "rs",
        "py",
        "js",
        "jsx",
        "ts",
        "tsx",
        "go",
        "java",
        "kt",
        "kts",
        "c",
        "h",
        "cpp",
        "hpp",
        "cc",
        "cs",
        "rb",
        "php",
        "swift",
        "scala",
        "sh",
        "bash",
        "zsh",
        "sql",
        "proto",
        "graphql",
        "gql",
        "vue",
        "svelte",
        "tf",
        "hcl",
        "dockerfile",
        "lua",
        "r",
        "jl",
        "ex",
        "exs",
        "clj",
        "cljs",
        "elm",
        "dart",
    ];
    // Common extensionless text files.
    const TEXT_FILENAMES: &[&str] = &[
        "dockerfile",
        "makefile",
        "readme",
        "license",
        "notice",
        "changelog",
    ];

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return TEXT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str());
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        return TEXT_FILENAMES.contains(&name.to_ascii_lowercase().as_str());
    }
    false
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") | Some("markdown") | Some("mdx") => "text/markdown",
        Some("html") | Some("htm") => "text/html",
        Some("json") | Some("jsonl") => "application/json",
        Some("csv") => "text/csv",
        _ => "text/plain",
    }
}

// ============================================================================
// Chunking
// ============================================================================

/// A chunk of a document with its character offsets in the source text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    pub text: String,
    /// Inclusive start char offset (counting Unicode scalar values).
    pub char_start: usize,
    /// Exclusive end char offset.
    pub char_end: usize,
    /// Approximate token count (~chars / 4); telemetry only.
    pub token_count: usize,
}

/// Split `text` into overlapping windows of up to `size` characters with
/// `overlap` characters shared between adjacent chunks. Offsets are counted in
/// Unicode scalar values so they survive multi-byte content. Empty/whitespace
/// input yields no chunks. `overlap` is clamped below `size` to guarantee
/// forward progress.
pub fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<TextChunk> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || text.trim().is_empty() {
        return Vec::new();
    }
    let size = size.max(1);
    let overlap = overlap.min(size.saturating_sub(1));
    let step = size - overlap;

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = (start + size).min(chars.len());
        let slice: String = chars[start..end].iter().collect();
        if !slice.trim().is_empty() {
            chunks.push(TextChunk {
                text: slice,
                char_start: start,
                char_end: end,
                token_count: (end - start).div_ceil(4),
            });
        }
        if end == chars.len() {
            break;
        }
        start += step;
    }
    chunks
}

// ============================================================================
// Helpers
// ============================================================================

fn sanitize_sync_error(error: &anyhow::Error) -> String {
    let mut message = error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| "knowledge index sync failed".to_string());
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
mod tests;
