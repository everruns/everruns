use super::{repository::excluded, types::*, *};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use tokio_util::io::ReaderStream;
impl KnowledgeService {
    pub async fn add_source(
        &self,
        ctx: &Ctx,
        c: &Collection,
        mut input: SourceInput,
    ) -> Result<Source, CommandError> {
        if !matches!(input.kind.as_str(), "github" | "git") {
            return Err(CommandError::bad_request(
                "Use github or git for connected sources",
            ));
        }
        let root = path(&input.root_path)?;
        let root = root.trim_matches('/').to_string();
        let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
        let _: Uuid =
            sqlx::query_scalar("SELECT id FROM knowledge_collections WHERE id=$1 FOR UPDATE")
                .bind(c.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let sources:Vec<Source>=sqlx::query_as("SELECT * FROM knowledge_sources WHERE org_id=$1 AND collection_id=$2 AND kind!='managed'").bind(c.org_id).bind(c.id).fetch_all(&mut *tx).await.map_err(db_error)?;
        if sources.iter().any(|s| {
            s.root_path == root
                || s.root_path.is_empty()
                || root.is_empty()
                || s.root_path.starts_with(&format!("{root}/"))
                || root.starts_with(&format!("{}/", s.root_path))
        }) {
            return Err(CommandError::conflict("Source roots overlap"));
        }
        let config = input
            .config
            .as_object_mut()
            .ok_or_else(|| CommandError::bad_request("Source config must be an object"))?;
        if config
            .keys()
            .any(|k| !matches!(k.as_str(), "repository" | "url" | "branch" | "root_folder"))
        {
            return Err(CommandError::bad_request(
                "Source configuration contains unsupported fields; credentials use existing connections",
            ));
        }
        if input.kind == "github" {
            let repo = config
                .get("repository")
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::bad_request("Repository required"))?;
            let repo = crate::domains::git_sources::normalize_github_repository(repo)
                .map_err(CommandError::bad_request)?;
            config.insert("repository".into(), json!(repo));
        } else {
            let url = config
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::bad_request("Git URL required"))?;
            everruns_contracts::url_validation::validate_safe_url(url)
                .map_err(|_| CommandError::bad_request("Unsafe Git URL"))?;
            let url =
                url::Url::parse(url).map_err(|_| CommandError::bad_request("Invalid Git URL"))?;
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(CommandError::bad_request(
                    "Use credential-free HTTPS Git coordinates",
                ));
            }
        }
        if let Some(branch) = config.get("branch").and_then(Value::as_str)
            && !git2::Reference::is_valid_name(&format!("refs/heads/{branch}"))
        {
            return Err(CommandError::bad_request("Invalid Git branch"));
        }
        if let Some(folder) = config.get("root_folder").and_then(Value::as_str) {
            path(folder)?;
            if folder.split('/').any(|p| p == ".git") {
                return Err(CommandError::bad_request(
                    "Git internal paths are not source roots",
                ));
            }
        }
        if let Some(owner) = input.connection_owner {
            crate::domains::virtual_users::connection_target(
                &ctx.db,
                ctx.permission_resolver.as_ref(),
                &ctx.caller,
                &everruns_contracts::typed_id::VirtualUserId::from_uuid(owner).to_string(),
            )
            .await?;
        }
        let row:Source=sqlx::query_as("INSERT INTO knowledge_sources(id,org_id,collection_id,kind,root_path,config,connection_owner,sync_status) VALUES($1,$2,$3,$4,$5,$6,$7,'pending') RETURNING *")
            .bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).bind(input.kind).bind(root).bind(input.config).bind(input.connection_owner).fetch_one(&mut *tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        self.enqueue(c, "source", Some(row.id), None, None).await?;
        Ok(row)
    }
    pub async fn ensure_docs(&self, org: i64) -> Result<(), CommandError> {
        // Publish the built-in collection, source and work atomically. A lost
        // request must not leave a permanently empty predefined collection.
        let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
        let created: Option<Collection> = sqlx::query_as("INSERT INTO knowledge_collections(id,org_id,name,description) VALUES($1,$2,'everruns-docs','Built-in public Everruns documentation; read and grep without credentials') ON CONFLICT(org_id,name) DO NOTHING RETURNING *")
            .bind(Uuid::now_v7()).bind(org).fetch_optional(&mut *tx).await.map_err(db_error)?;
        if let Some(c) = created {
            let source = Uuid::now_v7();
            sqlx::query("INSERT INTO knowledge_sources(id,org_id,collection_id,kind,config,sync_status) VALUES($1,$2,$3,'builtin',$4,'pending')")
                .bind(source).bind(org).bind(c.id).bind(json!({"revision":env!("CARGO_PKG_VERSION")})).execute(&mut *tx).await.map_err(db_error)?;
            sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,source_id,kind,revision) VALUES($1,$2,$3,$4,'source',$5)")
                .bind(Uuid::now_v7()).bind(org).bind(c.id).bind(source).bind(c.revision).execute(&mut *tx).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)?;

        Ok(())
    }
    pub async fn sync_source(&self, c: &Collection, job: &Job) -> Result<()> {
        // Each enumeration attempt needs its own marker. Reusing the job ID
        // would retain files seen before a crash but deleted before its retry.
        let run = job
            .lease_token
            .ok_or_else(|| anyhow!("source lease missing"))?;
        let source: Source = sqlx::query_as(
            "SELECT * FROM knowledge_sources WHERE org_id=$1 AND id=$2 AND collection_id=$3",
        )
        .bind(c.org_id)
        .bind(job.source_id)
        .bind(c.id)
        .fetch_one(self.db.tx_pool())
        .await?;
        sqlx::query(
            "UPDATE knowledge_sources SET sync_status='syncing',last_error=NULL WHERE id=$1",
        )
        .bind(source.id)
        .execute(self.db.tx_pool())
        .await?;
        if source.kind == "builtin" {
            for (p, text) in everruns_capabilities::knowledge_docs::FILES {
                if excluded(
                    c,
                    "ingest_exclude",
                    &path(&format!("{}/{p}", source.root_path))?,
                ) {
                    continue;
                }
                if !self.heartbeat(job, p).await? {
                    return Ok(());
                }
                self.ingest(
                    c,
                    &source,
                    FileInput {
                        path: (*p).into(),
                        content: None,
                        content_base64: None,
                        mime_type: Some("text/markdown".into()),
                        metadata: json!({"release":env!("CARGO_PKG_VERSION")}),
                        indexable: None,
                        readable: None,
                    },
                    futures::stream::once(async {
                        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(text.as_bytes()))
                    }),
                    format!(
                        "https://everruns.com/docs/{}/",
                        p.trim_end_matches(".md")
                            .trim_end_matches("/index")
                            .trim_start_matches("index")
                    ),
                    Some(env!("CARGO_PKG_VERSION").into()),
                    Some(run),
                    Some(job),
                )
                .await?;
            }
        } else {
            let url = if source.kind == "github" {
                crate::domains::git_sources::github_clone_url(
                    source.config["repository"]
                        .as_str()
                        .ok_or_else(|| anyhow!("repository missing"))?,
                )
                .map_err(|_| anyhow!("invalid repository"))?
                .0
            } else {
                source.config["url"]
                    .as_str()
                    .ok_or_else(|| anyhow!("url missing"))?
                    .to_string()
            };
            let token = if let ("github", Some(resolver), Some(owner)) = (
                source.kind.as_str(),
                &self.connections,
                Some(source.connection_owner.unwrap_or_default()),
            ) {
                resolver
                    .get_github_token_for_owner(
                        c.org_id,
                        owner,
                        &crate::domains::git_sources::github_read_access(
                            source.config["repository"].as_str().unwrap_or(""),
                        ),
                    )
                    .await?
            } else {
                None
            };
            let branch = source.config["branch"]
                .as_str()
                .unwrap_or("main")
                .to_string();
            let dir = tempfile::TempDir::new()?;
            let target = dir.path().to_path_buf();
            let url_copy = url.clone();
            let (dir, commit) = tokio::task::spawn_blocking(move || {
                crate::domains::git_fetch::shallow_checkout(
                    &crate::domains::git_fetch::FetchRequest {
                        url: &url_copy,
                        branch: &branch,
                        auth_token: token.as_deref(),
                    },
                    &target,
                )
                .map_err(|_| anyhow!("Git source fetch failed"))?;
                let repo = git2::Repository::open(target)?;
                let commit = repo.head()?.peel_to_commit()?.id().to_string();
                Ok::<_, anyhow::Error>((dir, commit))
            })
            .await??;
            let folder = source.config["root_folder"].as_str().unwrap_or("");
            let root = dir.path().join(folder).canonicalize()?;
            ensure!(
                root.starts_with(dir.path().canonicalize()?),
                "source root escaped repository"
            );
            // Retain one iterator per depth, not every sibling directory in
            // a wide source. The bound also caps simultaneously open handles.
            let mut directories = vec![tokio::fs::read_dir(&root).await?];
            while let Some(entries) = directories.last_mut() {
                let Some(entry) = entries.next_entry().await? else {
                    directories.pop();
                    continue;
                };
                let kind = entry.file_type().await?;
                if kind.is_symlink() || entry.file_name() == ".git" {
                    continue;
                }
                if kind.is_dir() {
                    ensure!(
                        directories.len() < 256,
                        "source directory depth exceeds limit"
                    );
                    directories.push(tokio::fs::read_dir(entry.path()).await?);
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&root)?
                    .to_str()
                    .ok_or_else(|| anyhow!("unsupported filename"))?
                    .replace('\\', "/");
                let virtual_path = path(&format!("{}/{}", source.root_path, relative))?;
                if excluded(c, "ingest_exclude", &virtual_path) {
                    continue;
                }
                if !self.heartbeat(job, &relative).await? {
                    return Ok(());
                }
                let repo_path = if folder.is_empty() {
                    relative.clone()
                } else {
                    format!("{}/{relative}", folder.trim_matches('/'))
                };
                let blob = {
                    let repo = git2::Repository::open(dir.path())?;
                    repo.revparse_single(&format!("HEAD:{repo_path}"))?
                        .id()
                        .to_string()
                };
                let unchanged:u64=sqlx::query("UPDATE knowledge_files f SET seen_run=$8 WHERE f.org_id=$1 AND f.source_id=$2 AND f.path=$3 AND f.deleted_at IS NULL AND EXISTS(SELECT 1 FROM knowledge_versions v WHERE v.id=f.current_version AND v.source_revision=$5 AND v.metadata->>'extractor'=$7) AND EXISTS(SELECT 1 FROM knowledge_jobs j WHERE j.id=$4 AND j.lease_token=$6 AND j.status='running' AND j.lease_until>now())").bind(c.org_id).bind(source.id).bind(&virtual_path).bind(job.id).bind(&blob).bind(job.lease_token).bind(super::extraction::PROFILE).bind(run).execute(self.db.tx_pool()).await?.rows_affected();
                if unchanged == 1 {
                    continue;
                }
                let bytes = tokio::fs::File::open(entry.path()).await?;
                let uri = if source.kind == "github" {
                    let mut uri = url::Url::parse("https://github.com")?;
                    uri.set_path(&format!(
                        "/{}/blob/{commit}/{repo_path}",
                        source.config["repository"].as_str().unwrap_or("")
                    ));
                    uri.to_string()
                } else {
                    format!("{url}#revision={commit}&path={repo_path}")
                };
                self.ingest(
                    c,
                    &source,
                    FileInput {
                        path: relative,
                        content: None,
                        content_base64: None,
                        mime_type: None,
                        metadata: json!({}),
                        indexable: None,
                        readable: None,
                    },
                    ReaderStream::new(bytes),
                    uri,
                    Some(blob),
                    Some(run),
                    Some(job),
                )
                .await?;
            }
        }
        // Absence becomes deletion only after a complete, successful traversal.
        let mut tx = self.db.tx_pool().begin().await?;
        let _:Option<i64>=sqlx::query_scalar("SELECT revision FROM knowledge_collections WHERE id=$1 AND revision=$2 AND status='active' FOR UPDATE").bind(c.id).bind(c.revision).fetch_optional(&mut *tx).await?;
        let _: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM knowledge_jobs WHERE id=$1 FOR UPDATE")
                .bind(job.id)
                .fetch_optional(&mut *tx)
                .await?;
        let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_jobs WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_until>now() AND revision=$3 AND EXISTS(SELECT 1 FROM knowledge_collections c WHERE c.id=knowledge_jobs.collection_id AND c.revision=$3 AND c.status='active'))").bind(job.id).bind(job.lease_token).bind(c.revision).fetch_one(&mut *tx).await?;
        if live {
            sqlx::query("UPDATE knowledge_files SET deleted_at=now() WHERE org_id=$1 AND source_id=$2 AND seen_run IS DISTINCT FROM $3 AND deleted_at IS NULL").bind(c.org_id).bind(source.id).bind(run).execute(&mut *tx).await?;
            sqlx::query("UPDATE knowledge_sources SET sync_status='synced',checkpoint=$2,last_synced_at=now(),last_error=NULL WHERE id=$1").bind(source.id).bind(job.id.to_string()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
