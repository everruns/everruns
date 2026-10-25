use super::{types::*, *};
use everruns_contracts::knowledge::IndexMode;
use serde_json::{Value, json};

impl KnowledgeService {
    pub async fn collection(&self, org: i64, key: &str) -> Result<Collection, CommandError> {
        let row = if key == "everruns-docs" {
            sqlx::query_as("SELECT * FROM knowledge_collections WHERE org_id=$1 AND name='everruns-docs' AND status='active'").bind(org).fetch_optional(self.db.tx_pool()).await
        } else {
            sqlx::query_as(
                "SELECT * FROM knowledge_collections WHERE org_id=$1 AND id=$2 AND status='active'",
            )
            .bind(org)
            .bind(id(key, "kn_")?)
            .fetch_optional(self.db.tx_pool())
            .await
        };
        row.map_err(db_error)?
            .ok_or_else(|| CommandError::not_found("Knowledge"))
    }
    pub async fn validate_input(
        &self,
        org: i64,
        input: &CollectionInput,
    ) -> Result<(), CommandError> {
        if input.name.trim().is_empty()
            || input.name.len() > 255
            || input.description.len() > 4096
            || serde_json::to_vec(&input.rules).map_or(true, |v| v.len() > 16384)
        {
            return Err(CommandError::bad_request("Invalid knowledge collection"));
        }
        if (!input.rules.is_null() && !input.rules.is_object())
            || input.rules.as_object().is_some_and(|rules| {
                rules.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "ingest_exclude" | "index_exclude" | "read_exclude"
                    )
                })
            })
        {
            return Err(CommandError::bad_request(
                "Use named ingestion, indexing or read exclusion rules",
            ));
        }
        for key in ["ingest_exclude", "index_exclude", "read_exclude"] {
            if let Some(patterns) = input.rules.get(key) {
                let Some(patterns) = patterns.as_array() else {
                    return Err(CommandError::bad_request(
                        "Rules must be arrays of path globs",
                    ));
                };
                if patterns.len() > 64 {
                    return Err(CommandError::bad_request("Too many exclusion rules"));
                }
                for pattern in patterns {
                    let pattern = pattern
                        .as_str()
                        .ok_or_else(|| CommandError::bad_request("Invalid path glob"))?;
                    if pattern.len() > 1000 {
                        return Err(CommandError::bad_request("Path glob too long"));
                    }
                    everruns_core::session_path::GrepPathPattern::new(pattern)
                        .map_err(|_| CommandError::bad_request("Invalid path glob"))?;
                }
            }
        }
        if input.indexing == IndexMode::Hybrid && input.embedding_model_id.is_none() {
            return Err(CommandError::bad_request(
                "Hybrid indexing requires an embedding model",
            ));
        }
        if let Some(model) = &input.embedding_model_id {
            let model = id(model, "model_")?;
            let row = self
                .db
                .get_model(org, model)
                .await?
                .ok_or_else(|| CommandError::bad_request("Embedding model unavailable"))?;
            let capabilities: Vec<String> =
                serde_json::from_value(row.capabilities).unwrap_or_default();
            if !row.enabled
                || !capabilities
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("embeddings"))
            {
                return Err(CommandError::bad_request(
                    "Model does not support embeddings",
                ));
            }
        }
        if let Some(indexer) = input.indexer_id {
            let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_indexers WHERE org_id=$1 AND id=$2 AND status='active')").bind(org).bind(indexer).fetch_one(self.db.tx_pool()).await.map_err(db_error)?;
            if !exists {
                return Err(CommandError::bad_request("Indexer unavailable"));
            }
        }
        Ok(())
    }
    pub async fn create(&self, org: i64, input: CollectionInput) -> Result<Value, CommandError> {
        self.validate_input(org, &input).await?;
        if input.name.trim() == "everruns-docs" {
            return Err(CommandError::bad_request(
                "Reserved built-in collection name",
            ));
        }
        let collection = Uuid::now_v7();
        let source = Uuid::now_v7();
        let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
        let row:Collection=sqlx::query_as("INSERT INTO knowledge_collections(id,org_id,name,description,indexing,embedding_model_id,indexer_id,rules) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING *")
            .bind(collection).bind(org).bind(input.name.trim()).bind(input.description).bind(mode(input.indexing)).bind(input.embedding_model_id.as_deref().map(|v|id(v,"model_")).transpose()?).bind(input.indexer_id).bind(input.rules).fetch_one(&mut *tx).await.map_err(db_error)?;
        sqlx::query("INSERT INTO knowledge_sources(id,org_id,collection_id,kind) VALUES($1,$2,$3,'managed')").bind(source).bind(org).bind(collection).execute(&mut *tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        sqlx::query("UPDATE knowledge_collections SET serving_config=$2 WHERE id=$1").bind(row.id).bind(json!({"indexer_id":row.indexer_id,"embedding_model_id":row.embedding_model_id,"indexing":row.indexing})).execute(self.db.tx_pool()).await.map_err(db_error)?;
        Ok(collection_view(row))
    }
    pub async fn update(
        &self,
        org: i64,
        key: &str,
        input: CollectionInput,
    ) -> Result<Value, CommandError> {
        self.validate_input(org, &input).await?;
        let old = self.collection(org, key).await?;
        if (old.name == "everruns-docs") != (input.name.trim() == "everruns-docs") {
            return Err(CommandError::bad_request(
                "Built-in collection name is reserved",
            ));
        }
        let mut tx = self.db.tx_pool().begin().await.map_err(db_error)?;
        let row:Collection=sqlx::query_as("UPDATE knowledge_collections SET name=$3,description=$4,indexing=$5,embedding_model_id=$6,indexer_id=$7,rules=$8,revision=revision+1,updated_at=now() WHERE org_id=$1 AND id=$2 RETURNING *")
            .bind(org).bind(old.id).bind(input.name.trim()).bind(input.description).bind(mode(input.indexing)).bind(input.embedding_model_id.as_deref().map(|v|id(v,"model_")).transpose()?).bind(input.indexer_id).bind(input.rules).fetch_one(&mut *tx).await.map_err(db_error)?;
        if row.indexing != "off" {
            sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,kind,revision) VALUES($1,$2,$3,'reindex',$4) ON CONFLICT DO NOTHING").bind(Uuid::now_v7()).bind(org).bind(row.id).bind(row.revision).execute(&mut *tx).await.map_err(db_error)?;
        }
        // A configuration change supersedes source jobs too; preserve pending intake.
        sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,source_id,kind,revision) SELECT gen_random_uuid(),org_id,collection_id,id,'source',$2 FROM knowledge_sources WHERE collection_id=$1 AND kind!='managed' AND sync_status!='synced' ON CONFLICT DO NOTHING").bind(row.id).bind(row.revision).execute(&mut *tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(collection_view(row))
    }
    pub async fn enqueue(
        &self,
        c: &Collection,
        kind: &str,
        source: Option<Uuid>,
        file: Option<Uuid>,
        version: Option<Uuid>,
    ) -> Result<(), CommandError> {
        sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,source_id,file_id,version_id,kind,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
            .bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).bind(source).bind(file).bind(version).bind(kind).bind(c.revision).execute(self.db.tx_pool()).await.map_err(db_error)?;
        Ok(())
    }
    pub async fn files_page(
        &self,
        c: &Collection,
        query: &PageQuery,
    ) -> Result<Value, CommandError> {
        let prefix = query
            .path
            .as_deref()
            .map(path)
            .transpose()?
            .unwrap_or_else(|| "/".into());
        let limit = query.limit.unwrap_or(100).clamp(1, 500) as i64;
        let rows: Vec<Value> = sqlx::query_scalar(FILES_PAGE_SQL)
            .bind(c.org_id)
            .bind(c.id)
            .bind(query.after.as_deref().unwrap_or(""))
            .bind(&prefix)
            .bind(limit + 1)
            .fetch_all(self.db.tx_pool())
            .await
            .map_err(db_error)?;
        let more = rows.len() > limit as usize;
        let next = more.then(|| {
            rows[limit as usize - 1]["path"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        });
        Ok(
            json!({"data":rows.into_iter().take(limit as usize).collect::<Vec<_>>(),"next_cursor":next}),
        )
    }
    pub async fn file(&self, c: &Collection, p: &str) -> Result<File, CommandError> {
        sqlx::query_as("SELECT * FROM knowledge_files WHERE org_id=$1 AND collection_id=$2 AND path=$3 AND deleted_at IS NULL")
        .bind(c.org_id).bind(c.id).bind(path(p)?).fetch_optional(self.db.tx_pool()).await.map_err(db_error)?.ok_or_else(||CommandError::not_found("Knowledge file"))
    }
    pub async fn version(
        &self,
        org: i64,
        file: Uuid,
        version: Uuid,
    ) -> Result<FileVersion, CommandError> {
        sqlx::query_as("SELECT * FROM knowledge_versions WHERE org_id=$1 AND file_id=$2 AND id=$3")
            .bind(org)
            .bind(file)
            .bind(version)
            .fetch_optional(self.db.tx_pool())
            .await
            .map_err(db_error)?
            .ok_or_else(|| CommandError::not_found("Knowledge version"))
    }
}
pub fn mode(value: IndexMode) -> &'static str {
    match value {
        IndexMode::Off => "off",
        IndexMode::Lexical => "lexical",
        IndexMode::Hybrid => "hybrid",
    }
}
pub fn collection_view(row: Collection) -> Value {
    let key = row.public_id();
    let mut value = json!(row);
    value["id"] = json!(key);
    value.as_object_mut().map(|v| v.remove("org_id"));
    value
}
// Reuse compiled path rules across files without letting corpus size grow the
// cache. Tenant scope prevents rules from becoming a cross-org cache oracle.
type CompiledRule = Option<Arc<everruns_core::session_path::GrepPathPattern>>;
static PATH_RULES: std::sync::LazyLock<moka::sync::Cache<(i64, String), CompiledRule>> =
    std::sync::LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(256)
            .time_to_idle(std::time::Duration::from_secs(600))
            .build()
    });

pub fn excluded(c: &Collection, kind: &str, p: &str) -> bool {
    c.rules
        .get(kind)
        .and_then(Value::as_array)
        .is_some_and(|patterns| {
            patterns.iter().filter_map(Value::as_str).any(|pattern| {
                PATH_RULES
                    .get_with((c.org_id, pattern.to_owned()), || {
                        everruns_core::session_path::GrepPathPattern::new(pattern)
                            .ok()
                            .map(Arc::new)
                    })
                    .is_none_or(|g| g.is_match(p))
            })
        })
}

/// One catalog page in path order. Binds: org, collection, the cursor (paths
/// after it), the path prefix and the row limit. `UNIQUE(collection_id,path)`
/// serves the order and the cursor, so a page reads its rows and no more however
/// large the collection; `files_page_reads_the_path_index` pins that plan and the
/// weekly `knowledge_catalog` bench times it on a million files.
pub(crate) const FILES_PAGE_SQL: &str = "SELECT to_jsonb(f)||jsonb_build_object('version',to_jsonb(v)-'raw_key'-'raw_parts'-'text_key'-'org_id') FROM knowledge_files f JOIN knowledge_versions v ON v.id=f.current_version WHERE f.org_id=$1 AND f.collection_id=$2 AND f.deleted_at IS NULL AND f.path>$3 AND starts_with(f.path,$4) ORDER BY f.path LIMIT $5";
