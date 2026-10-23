use super::{
    content::{Content, EXTRACT_LIMIT, READ_LIMIT},
    repository::excluded,
    types::*,
    *,
};
use futures::Stream;
use serde_json::{Value, json};
impl KnowledgeService {
    // Original bytes, provenance and the source lease meet at one publication fence.
    #[allow(clippy::too_many_arguments)]
    pub async fn ingest<S, E>(
        &self,
        c: &Collection,
        source: &Source,
        mut input: FileInput,
        stream: S,
        source_uri: String,
        source_revision: Option<String>,
        run: Option<Uuid>,
        fence: Option<&Job>,
    ) -> Result<Value, CommandError>
    where
        S: Stream<Item = std::result::Result<axum::body::Bytes, E>> + Send,
        E: std::fmt::Display,
    {
        let p = path(&format!(
            "{}/{}",
            source.root_path,
            input.path.trim_start_matches('/')
        ))?;
        if p == "/" || excluded(c, "ingest_exclude", &p) {
            return Err(CommandError::bad_request("Path excluded from ingestion"));
        }
        if input.mime_type.as_ref().is_some_and(|mime| {
            mime.len() > 255
                || !mime.contains('/')
                || axum::http::HeaderValue::from_str(mime).is_err()
        }) {
            return Err(CommandError::bad_request("Invalid bounded MIME type"));
        }
        if serde_json::to_vec(&input.metadata).map_or(true, |v| v.len() > 64 * 1024) {
            return Err(CommandError::bad_request("File metadata exceeds limit"));
        }
        let previous = match self.file(c, &p).await {
            Ok(file) => {
                let previous = self
                    .version(
                        c.org_id,
                        file.id,
                        file.current_version
                            .ok_or_else(|| CommandError::not_found("Knowledge version"))?,
                    )
                    .await?;
                Some((file, previous))
            }
            Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => None,
            Err(error) => return Err(error),
        };
        if previous
            .as_ref()
            .is_some_and(|(file, _)| file.source_id != source.id)
        {
            return Err(CommandError::conflict("Path belongs to a different source"));
        }
        let version = Uuid::now_v7();
        let store = Content::new(self.db.clone());
        let stored = store.store(c.org_id, version, stream).await.map_err(|_| {
            CommandError::bad_request("Content transfer failed or exceeded storage quota")
        })?;
        let mime = input.mime_type.clone().unwrap_or_else(|| {
            mime_guess::from_path(&p)
                .first_or_octet_stream()
                .to_string()
        });
        // A source refresh replaces content, not a curator's visibility or annotations.
        if source.kind != "managed"
            && let Some((file, old)) = &previous
        {
            input.indexable = Some(file.indexable);
            input.readable = Some(file.readable);
            input.metadata = old
                .metadata
                .get("user")
                .cloned()
                .unwrap_or_else(|| json!({}));
        }
        let indexable = input
            .indexable
            .unwrap_or_else(|| previous.as_ref().is_none_or(|(f, _)| f.indexable));
        let readable = input
            .readable
            .unwrap_or_else(|| previous.as_ref().is_none_or(|(f, _)| f.readable));
        if input.metadata.is_null() {
            input.metadata = previous
                .as_ref()
                .and_then(|(_, v)| v.metadata.get("user").cloned())
                .unwrap_or_else(|| json!({}));
        }
        if !input.metadata.is_object() {
            return Err(CommandError::bad_request("File metadata must be an object"));
        }
        if let Some((file, old)) = &previous
            && old.content_hash == stored.hash
            && old.mime_type == mime
            && old.metadata["extractor"] == super::extraction::PROFILE
            && old.metadata.get("user") == Some(&input.metadata)
            && file.indexable == indexable
            && file.readable == readable
        {
            let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
            let revision:Option<i64>=sqlx::query_scalar("SELECT revision FROM knowledge_collections WHERE id=$1 AND status='active' FOR UPDATE").bind(c.id).fetch_optional(&mut *tx).await.map_err(db_error)?;
            if revision != Some(c.revision) {
                return Err(CommandError::conflict("Collection changed"));
            }
            if let Some(job) = fence {
                let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM knowledge_jobs WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_until>now() FOR UPDATE").bind(job.id).bind(job.lease_token).fetch_optional(&mut *tx).await.map_err(db_error)?;
                if valid.is_none() {
                    return Err(CommandError::conflict("Source lease expired"));
                }
            }
            let updated=sqlx::query("UPDATE knowledge_files SET seen_run=$3 WHERE org_id=$1 AND id=$2 AND current_version=$4")
                .bind(c.org_id).bind(file.id).bind(run).bind(old.id).execute(&mut *tx).await.map_err(db_error)?.rows_affected();
            if updated != 1 {
                return Err(CommandError::conflict("File changed; retry upload"));
            }
            tx.commit().await.map_err(db_error)?;
            // No-op uploads must not accumulate unreferenced content segments.
            store
                .remove(&stored.key, stored.parts)
                .await
                .map_err(CommandError::internal)?;
            return Ok(
                json!({"file_id":file.id,"version_id":old.id,"path":file.path,"changed":false,"extraction_status":old.extraction_status}),
            );
        }
        let unchanged_content = previous.as_ref().filter(|(_, old)| {
            old.content_hash == stored.hash
                && old.mime_type == mime
                && old.metadata["extractor"] == super::extraction::PROFILE
        });
        let (extracted, text_key, text_len) = if let Some((_, old)) = unchanged_content {
            (
                super::extraction::Extracted {
                    text: None,
                    metadata: old.metadata["document"].clone(),
                    warnings: serde_json::from_value(old.warnings.clone()).unwrap_or_default(),
                    status: match old.extraction_status.as_str() {
                        "ready" => "ready",
                        "over_budget" => "over_budget",
                        "failed" => "failed",
                        "partial" => "partial",
                        _ => "unsupported",
                    },
                },
                old.text_key.clone(),
                old.text_bytes as usize,
            )
        } else {
            let extracted = if stored.bytes > EXTRACT_LIMIT as i64 {
                super::extraction::Extracted {
                    text: None,
                    metadata: json!({}),
                    warnings: vec!["extraction_budget_exceeded_original_retained".into()],
                    status: "over_budget",
                }
            } else {
                let name = p.clone();
                let mime = mime.clone();
                let bytes = stored.extraction;
                // THREAT[TM-DOS-049]: archive/cell budgets are per extraction;
                // also bound parallel CPU/memory work across uploads and jobs.
                static EXTRACTIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
                let permit = EXTRACTIONS
                    .acquire()
                    .await
                    .map_err(|_| CommandError::unavailable("Extraction unavailable"))?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    super::extraction::extract(&bytes, &name, &mime)
                })
                .await
                .map_err(|_| CommandError::unavailable("Extraction interrupted"))?
            };
            let text_len = extracted.text.as_ref().map_or(0, |v| v.len());
            let text_key = if let Some(text) = &extracted.text {
                Some(
                    store
                        .text("text", c.org_id, version, text)
                        .await
                        .map_err(CommandError::internal)?,
                )
            } else {
                None
            };
            (extracted, text_key, text_len)
        };
        let metadata = json!({"user":input.metadata,"document":extracted.metadata,"extractor":super::extraction::PROFILE});
        let file = previous
            .as_ref()
            .map(|(f, _)| f.id)
            .unwrap_or_else(Uuid::now_v7);
        let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
        // Serialize publication against config changes, source deletion and competing writers.
        let revision:Option<i64>=sqlx::query_scalar("SELECT revision FROM knowledge_collections WHERE org_id=$1 AND id=$2 AND status='active' FOR UPDATE").bind(c.org_id).bind(c.id).fetch_optional(&mut *tx).await.map_err(db_error)?;
        let owner: Option<Uuid> = sqlx::query_scalar(
            "SELECT source_id FROM knowledge_files WHERE collection_id=$1 AND path=$2 FOR UPDATE",
        )
        .bind(c.id)
        .bind(&p)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        if owner.is_some_and(|id| id != source.id) {
            return Err(CommandError::conflict("Path belongs to a different source"));
        }
        if let Some(job) = fence {
            let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_jobs WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_until>now())").bind(job.id).bind(job.lease_token).fetch_one(&mut *tx).await.map_err(db_error)?;
            if !valid {
                return Err(CommandError::conflict("Source lease expired"));
            }
        }
        if revision != Some(c.revision) {
            return Err(CommandError::conflict(
                "Collection changed; retry the upload",
            ));
        }
        sqlx::query("INSERT INTO knowledge_files(id,org_id,collection_id,source_id,path,indexable,readable,seen_run) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(collection_id,path) DO UPDATE SET indexable=$6,readable=$7,seen_run=$8,deleted_at=NULL,updated_at=now()")
            .bind(file).bind(c.org_id).bind(c.id).bind(source.id).bind(&p).bind(indexable).bind(readable).bind(run).execute(&mut *tx).await.map_err(db_error)?;
        let file: Uuid =
            sqlx::query_scalar("SELECT id FROM knowledge_files WHERE collection_id=$1 AND path=$2")
                .bind(c.id)
                .bind(&p)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,text_key,text_bytes,metadata,warnings,extraction_status,source_uri,source_revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
            .bind(version).bind(c.org_id).bind(file).bind(stored.hash).bind(stored.bytes).bind(mime).bind(stored.key).bind(stored.parts).bind(text_key).bind(text_len as i32).bind(metadata).bind(json!(extracted.warnings)).bind(extracted.status).bind(source_uri).bind(source_revision).execute(&mut *tx).await.map_err(db_error)?;
        sqlx::query("UPDATE knowledge_files SET current_version=$3 WHERE org_id=$1 AND id=$2")
            .bind(c.org_id)
            .bind(file)
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        if c.indexing != "off" {
            sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,file_id,version_id,kind,revision) VALUES($1,$2,$3,$4,$5,'index',$6) ON CONFLICT DO NOTHING").bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).bind(file).bind(version).bind(c.revision).execute(&mut *tx).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(
            json!({"file_id":file,"version_id":version,"path":p,"changed":true,"extraction_status":extracted.status}),
        )
    }
    pub async fn read_bytes(
        &self,
        c: &Collection,
        p: &str,
        q: &PageQuery,
    ) -> Result<(FileVersion, Vec<u8>), CommandError> {
        self.read_content(c, p, q, true).await
    }

    /// Management callers already pass VIEW; agent read denies do not remove
    /// an administrator's ability to curate or download the retained original.
    pub(crate) async fn read_management_bytes(
        &self,
        c: &Collection,
        p: &str,
        q: &PageQuery,
    ) -> Result<(FileVersion, Vec<u8>), CommandError> {
        self.read_content(c, p, q, false).await
    }

    async fn read_content(
        &self,
        c: &Collection,
        p: &str,
        q: &PageQuery,
        enforce_agent_policy: bool,
    ) -> Result<(FileVersion, Vec<u8>), CommandError> {
        let file = self.file(c, p).await?;
        if enforce_agent_policy && (!file.readable || excluded(c, "read_exclude", &file.path)) {
            return Err(CommandError::not_found("Knowledge file"));
        }
        let version = self
            .version(
                c.org_id,
                file.id,
                q.version
                    .or(file.current_version)
                    .ok_or_else(|| CommandError::not_found("Knowledge version"))?,
            )
            .await?;
        let extracted = q.extracted.unwrap_or(false);
        let key = if extracted {
            version
                .text_key
                .as_ref()
                .ok_or_else(|| CommandError::bad_request("No text extraction; original retained"))?
        } else {
            &version.raw_key
        };
        let size = if extracted {
            version.text_bytes as u64
        } else {
            version.size_bytes as u64
        };
        let bytes = Content::new(self.db.clone())
            .range(
                key,
                size,
                q.offset.unwrap_or(0),
                q.length.unwrap_or(READ_LIMIT as u32).min(READ_LIMIT as u32) as usize,
            )
            .await
            .map_err(CommandError::internal)?;
        Ok((version, bytes))
    }
}
