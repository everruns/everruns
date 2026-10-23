use super::{repository::collection_view, types::*, *};
use crate::domains::common::*;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListKnowledge {
    pub after: Option<String>,
    pub limit: Option<u32>,
}
#[command(name="list_knowledge",category="knowledge",description="List versioned knowledge collections.",method="GET",path="/v1/knowledge",policy=VIEW,cli=CliRoute::new(&["knowledge"],"list").with_examples(&[CliExample::new("Find available Knowledge collections","everruns knowledge list --limit 10")]))]
impl Command for ListKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let service = KnowledgeService::from_ctx(ctx);
        service.ensure_docs(ctx.org_id()).await?;
        let limit = self.limit.unwrap_or(100).clamp(1, 500) as i64;
        let rows:Vec<Collection>=sqlx::query_as("SELECT * FROM knowledge_collections WHERE org_id=$1 AND name>$2 AND status='active' ORDER BY name LIMIT $3").bind(ctx.org_id()).bind(self.after.unwrap_or_default()).bind(limit+1).fetch_all(ctx.db.tx_pool()).await.map_err(db_error)?;
        let next = (rows.len() > limit as usize).then(|| rows[limit as usize - 1].name.clone());
        Ok(
            json!({"data":rows.into_iter().take(limit as usize).map(collection_view).collect::<Vec<_>>(),"next_cursor":next}),
        )
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateKnowledge {
    #[serde(flatten)]
    pub input: CollectionInput,
}
#[command(name="create_knowledge",category="knowledge",description="Create a knowledge collection; indexing is optional.",method="POST",path="/v1/knowledge",policy=MANAGE,cli=CliRoute::new(&["knowledge"],"create").with_examples(&[CliExample::new("Create a collection without indexing","everruns knowledge create --name 'Support handbook' --indexing off")]))]
impl Command for CreateKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        if self.input.name.trim() == "everruns-docs" {
            return Err(CommandError::bad_request(
                "Reserved built-in collection name",
            ));
        }
        KnowledgeService::from_ctx(ctx)
            .create(ctx.org_id(), self.input)
            .await
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GetKnowledge {
    pub collection_id: String,
}
#[command(name="get_knowledge",category="knowledge",description="Inspect a knowledge collection, sources and processing status.",method="GET",path="/v1/knowledge/{collection_id}",policy=VIEW,cli=CliRoute::new(&["knowledge"],"get").with_examples(&[CliExample::new("Inspect source and indexing freshness","everruns knowledge get --collection-id kn_01950000000070008000000000000001")]))]
impl Command for GetKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let service = KnowledgeService::from_ctx(ctx);
        service.ensure_docs(ctx.org_id()).await?;
        let c = service
            .collection(ctx.org_id(), &self.collection_id)
            .await?;
        let sources:Vec<Source>=sqlx::query_as("SELECT * FROM knowledge_sources WHERE org_id=$1 AND collection_id=$2 ORDER BY root_path").bind(c.org_id).bind(c.id).fetch_all(ctx.db.tx_pool()).await.map_err(db_error)?;
        let counts=sqlx::query("SELECT count(*) AS files,count(*) FILTER(WHERE current_version=indexed_version AND indexed_revision=$3) AS indexed FROM knowledge_files WHERE org_id=$1 AND collection_id=$2 AND deleted_at IS NULL").bind(c.org_id).bind(c.id).bind(c.revision).fetch_one(ctx.db.tx_pool()).await.map_err(db_error)?;
        use sqlx::Row;
        let jobs:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'kind',kind,'status',status,'attempts',attempts,'error',last_error,'cursor',cursor) FROM knowledge_jobs WHERE org_id=$1 AND collection_id=$2 AND status!='done' ORDER BY created_at DESC LIMIT 50").bind(c.org_id).bind(c.id).fetch_all(ctx.db.tx_pool()).await.map_err(db_error)?;
        let mut view = collection_view(c);
        view["sources"] = json!(sources);
        view["file_count"] = json!(counts.get::<i64, _>("files"));
        view["indexed_count"] = json!(counts.get::<i64, _>("indexed"));
        view["jobs"] = json!(jobs);
        Ok(view)
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateKnowledge {
    pub collection_id: String,
    #[serde(flatten)]
    pub input: CollectionInput,
}
#[command(name="update_knowledge",category="knowledge",description="Configure knowledge indexing and exclusion rules.",method="PATCH",path="/v1/knowledge/{collection_id}",policy=MANAGE,cli=CliRoute::new(&["knowledge"],"update").with_examples(&[CliExample::new("Enable keyword indexing on retained files","everruns knowledge update --collection-id kn_01950000000070008000000000000001 --name 'Support handbook' --indexing lexical")]))]
impl Command for UpdateKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        KnowledgeService::from_ctx(ctx)
            .update(ctx.org_id(), &self.collection_id, self.input)
            .await
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ArchiveKnowledge {
    pub collection_id: String,
}
#[command(name="archive_knowledge",category="knowledge",description="Archive a knowledge collection.",method="DELETE",path="/v1/knowledge/{collection_id}",policy=MANAGE,cli=CliRoute::new(&["knowledge"],"delete").with_examples(&[CliExample::new("Remove a collection from active use","everruns knowledge delete --collection-id kn_01950000000070008000000000000001")]))]
impl Command for ArchiveKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let c = KnowledgeService::from_ctx(ctx)
            .collection(ctx.org_id(), &self.collection_id)
            .await?;
        if c.name == "everruns-docs" {
            return Err(CommandError::bad_request(
                "The predefined documentation collection cannot be archived",
            ));
        }
        sqlx::query("UPDATE knowledge_collections SET status='archived',updated_at=now() WHERE org_id=$1 AND id=$2").bind(c.org_id).bind(c.id).execute(ctx.db.tx_pool()).await.map_err(db_error)?;
        Ok(json!({"archived":true}))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteKnowledgeFile {
    pub collection_id: String,
    #[serde(flatten)]
    pub input: FileInput,
}
#[command(name="write_knowledge_file",category="knowledge",description="Add or update an original file in managed knowledge.",method="PUT",path="/v1/knowledge/{collection_id}/files",policy=MANAGE,cli=CliRoute::new(&["knowledge","files"],"write").with_examples(&[CliExample::new("Write a managed note preserving its path","everruns knowledge files write --collection-id kn_01950000000070008000000000000001 --path /policies/refunds.md --content 'Refunds are available for thirty days.'")]))]
impl Command for WriteKnowledgeFile {
    type Output = Value;
    async fn execute(mut self, ctx: &Ctx) -> Result<Value, CommandError> {
        let service = KnowledgeService::from_ctx(ctx);
        let c = service
            .collection(ctx.org_id(), &self.collection_id)
            .await?;
        let source = managed(&service, &c).await?;
        let bytes = match (self.input.content.take(), self.input.content_base64.take()) {
            (Some(text), None) => text.into_bytes(),
            (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| CommandError::bad_request("Invalid base64"))?,
            _ => {
                return Err(CommandError::bad_request(
                    "Supply content or content_base64",
                ));
            }
        };
        if bytes.len() > super::content::EXTRACT_LIMIT {
            return Err(CommandError::bad_request(
                "Use the streaming upload endpoint for files over 8 MiB",
            ));
        }
        let uri = format!(
            "knowledge://{}/{}",
            c.public_id(),
            self.input.path.trim_matches('/')
        );
        service
            .ingest(
                &c,
                &source,
                self.input,
                futures::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from(bytes))
                }),
                uri,
                None,
                None,
                None,
            )
            .await
    }
}
pub async fn managed(service: &KnowledgeService, c: &Collection) -> Result<Source, CommandError> {
    if c.name == "everruns-docs" {
        return Err(CommandError::forbidden(
            "Built-in documentation is read-only",
        ));
    }
    sqlx::query_as(
        "SELECT * FROM knowledge_sources WHERE org_id=$1 AND collection_id=$2 AND kind='managed'",
    )
    .bind(c.org_id)
    .bind(c.id)
    .fetch_optional(service.db.tx_pool())
    .await
    .map_err(db_error)?
    .ok_or_else(|| CommandError::forbidden("Knowledge source is read-only"))
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListKnowledgeFiles {
    pub collection_id: String,
    #[serde(flatten)]
    pub query: PageQuery,
}
#[command(name="list_knowledge_files",category="knowledge",description="List knowledge files with bounded cursor pagination.",method="GET",path="/v1/knowledge/{collection_id}/files",policy=VIEW,cli=CliRoute::new(&["knowledge","files"],"list").with_examples(&[CliExample::new("Page through a collection folder","everruns knowledge files list --collection-id kn_01950000000070008000000000000001 --path /policies/ --limit 20")]))]
impl Command for ListKnowledgeFiles {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let s = KnowledgeService::from_ctx(ctx);
        let c = s.collection(ctx.org_id(), &self.collection_id).await?;
        s.files_page(&c, &self.query).await
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeleteKnowledgeFile {
    pub collection_id: String,
    pub path: String,
}
#[command(name="delete_knowledge_file",category="knowledge",description="Delete a managed knowledge file and stop serving its search results.",method="DELETE",path="/v1/knowledge/{collection_id}/files",policy=MANAGE,cli=CliRoute::new(&["knowledge","files"],"delete").with_examples(&[CliExample::new("Delete a managed file","everruns knowledge files delete --collection-id kn_01950000000070008000000000000001 --path /policies/obsolete.md")]))]
impl Command for DeleteKnowledgeFile {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let s = KnowledgeService::from_ctx(ctx);
        let c = s.collection(ctx.org_id(), &self.collection_id).await?;
        let source = managed(&s, &c).await?;
        let f = s.file(&c, &self.path).await?;
        if f.source_id != source.id {
            return Err(CommandError::forbidden("Connected files are read-only"));
        }
        sqlx::query("UPDATE knowledge_files SET deleted_at=now() WHERE org_id=$1 AND id=$2")
            .bind(c.org_id)
            .bind(f.id)
            .execute(ctx.db.tx_pool())
            .await
            .map_err(db_error)?;
        Ok(json!({"deleted":true}))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddKnowledgeSource {
    pub collection_id: String,
    #[serde(flatten)]
    pub input: SourceInput,
}
#[command(name="add_knowledge_source",category="knowledge",description="Connect an existing GitHub identity or public Git source to knowledge.",method="POST",path="/v1/knowledge/{collection_id}/sources",policy=MANAGE,cli=CliRoute::new(&["knowledge","sources"],"add").with_examples(&[CliExample::new("Attach an existing GitHub connection to a source root","everruns knowledge sources add --collection-id kn_01950000000070008000000000000001 --kind github --root-path /product --config '{\"repository\":\"everruns/everruns\",\"root_folder\":\"docs\"}'")]))]
impl Command for AddKnowledgeSource {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let s = KnowledgeService::from_ctx(ctx);
        let c = s.collection(ctx.org_id(), &self.collection_id).await?;
        Ok(json!(s.add_source(ctx, &c, self.input).await?))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SyncKnowledge {
    pub collection_id: String,
}
#[command(name="sync_knowledge",category="knowledge",description="Queue source refresh and retry failed indexing jobs.",method="POST",path="/v1/knowledge/{collection_id}/sync",policy=MANAGE,cli=CliRoute::new(&["knowledge"],"sync").with_examples(&[CliExample::new("Refresh connected sources incrementally","everruns knowledge sync --collection-id kn_01950000000070008000000000000001")]))]
impl Command for SyncKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let s = KnowledgeService::from_ctx(ctx);
        let c = s.collection(ctx.org_id(), &self.collection_id).await?;
        // Retry from current source/file identities; obsolete failed jobs must not
        // collide with a newer active job for the same revision.
        sqlx::query("UPDATE knowledge_jobs SET status='done' WHERE org_id=$1 AND collection_id=$2 AND status='failed'").bind(c.org_id).bind(c.id).execute(ctx.db.tx_pool()).await.map_err(db_error)?;
        let sources:Vec<Source>=sqlx::query_as("SELECT * FROM knowledge_sources WHERE org_id=$1 AND collection_id=$2 AND kind!='managed'").bind(c.org_id).bind(c.id).fetch_all(ctx.db.tx_pool()).await.map_err(db_error)?;
        for source in sources {
            s.enqueue(&c, "source", Some(source.id), None, None).await?;
        }
        if c.indexing != "off" {
            s.enqueue(&c, "reindex", None, None, None).await?;
        }
        Ok(json!({"queued":true}))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchKnowledgeCollections {
    #[serde(flatten)]
    pub query: everruns_contracts::knowledge::KnowledgeQuery,
}
#[command(name="search_knowledge_collections",category="knowledge",description="Search selected knowledge collections with immutable citations.",method="POST",path="/v1/knowledge/search",policy=VIEW,read_only=true,cli=CliRoute::new(&["knowledge"],"search").with_examples(&[CliExample::new("Retrieve cited policy passages","everruns knowledge search --collections kn_01950000000070008000000000000001 --query 'refund policy' --limit 5")]))]
impl Command for SearchKnowledgeCollections {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        Ok(json!(
            KnowledgeService::from_ctx(ctx)
                .search(ctx.org_id(), self.query)
                .await?
        ))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateKnowledgeIndexer {
    #[serde(flatten)]
    pub input: IndexerInput,
}
#[command(name="create_knowledge_indexer",category="knowledge",description="Configure an organization Turbopuffer account using an existing write-only credential binding.",method="POST",path="/v1/knowledge-indexers",policy=MANAGE,cli=CliRoute::new(&["knowledge","indexers"],"create").with_examples(&[CliExample::new("Use an organization service credential for Turbopuffer","everruns knowledge indexers create --name 'Search account' --endpoint https://gcp-us-central1.turbopuffer.com --credential-binding-id 01950000-0000-7000-8000-000000000002")]))]
impl Command for CreateKnowledgeIndexer {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let url = url::Url::parse(&self.input.endpoint)
            .map_err(|_| CommandError::bad_request("Invalid indexer endpoint"))?;
        if url.scheme() != "https"
            || !url
                .host_str()
                .is_some_and(|h| h.ends_with(".turbopuffer.com"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || self.input.name.trim().is_empty()
            || self.input.name.len() > 255
        {
            return Err(CommandError::bad_request(
                "Select an HTTPS regional Turbopuffer endpoint",
            ));
        }
        let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM credential_bindings c JOIN resource_access r ON r.id=c.id AND r.org_id=c.org_id WHERE c.org_id=$1 AND c.id=$2 AND r.scope='organization' AND r.usage='service' AND r.revoked_at IS NULL AND c.kind='http')").bind(ctx.org_id()).bind(self.input.credential_binding_id).fetch_one(ctx.db.tx_pool()).await.map_err(db_error)?;
        if !allowed {
            return Err(CommandError::bad_request(
                "Use an active organization service credential bound to this endpoint",
            ));
        }
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO knowledge_indexers(id,org_id,name,endpoint,credential_binding_id) VALUES($1,$2,$3,$4,$5)").bind(id).bind(ctx.org_id()).bind(self.input.name).bind(self.input.endpoint.trim_end_matches('/')).bind(self.input.credential_binding_id).execute(ctx.db.tx_pool()).await.map_err(db_error)?;
        Ok(json!({"id":id}))
    }
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkerKnowledge {
    pub session_id: String,
    pub operation: String,
    pub query: Option<everruns_contracts::knowledge::KnowledgeQuery>,
    pub input_id: Option<String>,
    pub catalog: Option<everruns_contracts::knowledge::KnowledgeCatalogQuery>,
}
#[command(
    name = "worker_knowledge",
    category = "knowledge",
    description = "Internal: authorize attached knowledge retrieval.",
    method = "POST",
    path = "/internal/knowledge/runtime",
    read_only = true
)]
impl Command for WorkerKnowledge {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        if !ctx.feature_flags.knowledge {
            return Err(CommandError::feature_not_enabled("knowledge"));
        }
        let session = self
            .session_id
            .parse()
            .map_err(|_| CommandError::not_found("Session"))?;
        let service = KnowledgeService::from_ctx(ctx);
        match self.operation.as_str() {
            "files" => Ok(json!(
                service
                    .session_catalog(
                        ctx.org_id(),
                        session,
                        self.catalog
                            .ok_or_else(|| CommandError::bad_request("Catalog query required"))?
                    )
                    .await?
            )),
            "search" => Ok(json!(
                service
                    .session_search(
                        ctx.org_id(),
                        session,
                        self.query
                            .ok_or_else(|| CommandError::bad_request("Query required"))?
                    )
                    .await?
            )),
            "automatic" => Ok(json!(
                service
                    .automatic(
                        ctx.org_id(),
                        session,
                        self.input_id
                            .ok_or_else(|| CommandError::bad_request("Input required"))?
                            .parse()
                            .map_err(|_| CommandError::bad_request("Invalid input ID"))?,
                        self.query.map(|q| q.query).unwrap_or_default()
                    )
                    .await?
            )),
            _ => Err(CommandError::bad_request("Unknown knowledge operation")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateKnowledgeFileMetadata {
    pub collection_id: String,
    #[serde(flatten)]
    pub input: FileMetadataInput,
}
#[command(name="update_knowledge_file_metadata",category="knowledge",description="Update annotations or file read/index policy without extracting content again.",method="PATCH",path="/v1/knowledge/{collection_id}/files",policy=MANAGE,cli=CliRoute::new(&["knowledge","files"],"update").with_examples(&[CliExample::new("Exclude a file from indexing while retaining it","everruns knowledge files update --collection-id kn_01950000000070008000000000000001 --path /policies/draft.md --indexable false")]))]
impl Command for UpdateKnowledgeFileMetadata {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let service = KnowledgeService::from_ctx(ctx);
        let c = service
            .collection(ctx.org_id(), &self.collection_id)
            .await?;
        if let Some(metadata) = &self.input.metadata
            && (!metadata.is_object()
                || serde_json::to_vec(metadata).map_or(true, |v| v.len() > 65536))
        {
            return Err(CommandError::bad_request(
                "Metadata must be an object of at most 64 KiB",
            ));
        }
        let mut tx = ctx.db.tx_pool().begin().await.map_err(db_error)?;
        let _: i64 =
            sqlx::query_scalar("SELECT revision FROM knowledge_collections WHERE id=$1 FOR UPDATE")
                .bind(c.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        let f:File=sqlx::query_as("SELECT * FROM knowledge_files WHERE collection_id=$1 AND path=$2 AND deleted_at IS NULL FOR UPDATE").bind(c.id).bind(path(&self.input.path)?).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(||CommandError::not_found("Knowledge file"))?;
        let version = Uuid::now_v7();
        sqlx::query("INSERT INTO knowledge_versions(id,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,text_key,text_bytes,metadata,warnings,extraction_status,source_uri,source_revision) SELECT $1,org_id,file_id,content_hash,size_bytes,mime_type,raw_key,raw_parts,text_key,text_bytes,CASE WHEN $3::jsonb IS NULL THEN metadata ELSE jsonb_set(metadata,'{user}',$3) END,warnings,extraction_status,source_uri,source_revision FROM knowledge_versions WHERE id=$2").bind(version).bind(f.current_version).bind(&self.input.metadata).execute(&mut *tx).await.map_err(db_error)?;
        sqlx::query("UPDATE knowledge_files SET current_version=$2,indexable=coalesce($3,indexable),readable=coalesce($4,readable),updated_at=now() WHERE id=$1").bind(f.id).bind(version).bind(self.input.indexable).bind(self.input.readable).execute(&mut *tx).await.map_err(db_error)?;
        if c.indexing != "off" {
            sqlx::query("INSERT INTO knowledge_jobs(id,org_id,collection_id,file_id,version_id,kind,revision) VALUES($1,$2,$3,$4,$5,'index',$6)").bind(Uuid::now_v7()).bind(c.org_id).bind(c.id).bind(f.id).bind(version).bind(c.revision).execute(&mut *tx).await.map_err(db_error)?;
        }
        tx.commit().await.map_err(db_error)?;
        Ok(json!({"file_id":f.id,"version_id":version}))
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListKnowledgeIndexers {}
#[command(name="list_knowledge_indexers",category="knowledge",description="List organization search-provider accounts without credential values.",method="GET",path="/v1/knowledge-indexers",policy=VIEW,cli=CliRoute::new(&["knowledge","indexers"],"list").with_examples(&[CliExample::new("List organization search accounts without exposing credentials","everruns knowledge indexers list")]))]
impl Command for ListKnowledgeIndexers {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'name',name,'endpoint',endpoint,'credential_binding_id',credential_binding_id,'status',status) FROM knowledge_indexers WHERE org_id=$1 ORDER BY name").bind(ctx.org_id()).fetch_all(ctx.db.tx_pool()).await.map_err(db_error)?;
        Ok(json!({"data":rows}))
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ValidateKnowledgeOkf {
    pub collection_id: String,
    #[serde(flatten)]
    pub query: PageQuery,
}
#[command(name="validate_knowledge_okf",category="knowledge",description="Validate a page of OKF v0.2 metadata and reserved navigation files without converting originals.",method="GET",path="/v1/knowledge/{collection_id}/okf",policy=VIEW,cli=CliRoute::new(&["knowledge","okf"],"validate").with_examples(&[CliExample::new("Validate a page of OKF metadata","everruns knowledge okf validate --collection-id kn_01950000000070008000000000000001 --limit 100")]))]
impl Command for ValidateKnowledgeOkf {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let service = KnowledgeService::from_ctx(ctx);
        let c = service
            .collection(ctx.org_id(), &self.collection_id)
            .await?;
        service.validate_okf(&c, &self.query).await
    }
}
