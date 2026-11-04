use super::{content::Content, repository::excluded, types::*, *};
use anyhow::{Result, anyhow, ensure};
use everruns_contracts::{
    knowledge::{KnowledgeHit, KnowledgeQuery, KnowledgeResults},
    vector_store::{VectorQuery, VectorRecord, VectorStore},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;

#[cfg(test)]
mod embedding_usage_tests;

pub fn namespace(c: &Collection) -> String {
    format!(
        "knowledge_org_{}_{}__r{}",
        c.org_id,
        c.id.simple(),
        c.revision
    )
}
impl KnowledgeService {
    pub async fn backend(&self, c: &Collection) -> Result<Option<Arc<dyn VectorStore>>> {
        // THREAT[TM-AUTHZ-026]: a selected account is explicit authority;
        // unavailable/revoked bindings must never select deployment credentials.
        if let Some(account) = c.indexer_id {
            let row=sqlx::query("SELECT i.endpoint,c.value_encrypted,c.binding FROM knowledge_indexers i JOIN credential_bindings c ON c.id=i.credential_binding_id AND c.org_id=i.org_id JOIN resource_access r ON r.id=c.id AND r.org_id=c.org_id WHERE i.org_id=$1 AND i.id=$2 AND i.status='active' AND r.scope='organization' AND r.usage='service' AND r.revoked_at IS NULL AND (c.expires_at IS NULL OR c.expires_at>now())")
                .bind(c.org_id).bind(account).fetch_optional(self.db.tx_pool()).await?.ok_or_else(||anyhow!("Indexer credential unavailable"))?;
            let endpoint: String = row.get("endpoint");
            let binding: Option<
                sqlx::types::Json<everruns_contracts::runtime::session_services::SecretBinding>,
            > = row.get("binding");
            ensure!(
                binding.as_ref().is_some_and(|v| {
                    matches!(
                        v.auth,
                        everruns_contracts::runtime::session_services::SecretAuth::Bearer
                    ) && everruns_contracts::runtime::credential_http::binding_matches(
                        v,
                        &format!("{endpoint}/v2/namespaces"),
                    )
                }),
                "Indexer credential destination mismatch"
            );
            let encrypted: Option<Vec<u8>> = row.get("value_encrypted");
            let key = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption unavailable"))?
                .decrypt_to_string(
                    encrypted
                        .as_deref()
                        .ok_or_else(|| anyhow!("Indexer key not provisioned"))?,
                )?;
            return Ok(Some(Arc::new(
                everruns_turbopuffer::TurbopufferVectorStore::new(endpoint, key),
            )));
        }
        // Only an explicit unbound/default account uses the deployment key.
        if let Ok(key) = std::env::var("TURBOPUFFER_API_KEY")
            && !key.trim().is_empty()
        {
            return Ok(Some(Arc::new(
                everruns_turbopuffer::TurbopufferVectorStore::new(
                    std::env::var("TURBOPUFFER_BASE_URL")
                        .unwrap_or_else(|_| "https://gcp-us-central1.turbopuffer.com".into()),
                    key,
                )
                .with_usage_recorder(Arc::new(super::usage::IndexUsageRecorder {
                    db: self.db.clone(),
                    org_id: c.org_id,
                    collection_id: c.id,
                })),
            )));
        }
        Ok(None)
    }
    async fn embed(&self, c: &Collection, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = c
            .embedding_model_id
            .ok_or_else(|| anyhow!("Embedding model required"))?;
        let resolver =
            crate::services::ProviderResolverService::new(self.db.clone(), self.encryption.clone())
                .with_driver_registry(self.drivers.as_ref().clone());
        let embedder =
            crate::domains::knowledge_indexes::embedding::build_embeddings_driver_for_model(
                &self.db,
                &resolver,
                &self.drivers,
                c.org_id,
                model,
            )
            .await?;
        let result = embedder
            .driver
            .embed(
                &everruns_contracts::runtime_provider::ProviderEndpoint::default(),
                everruns_contracts::driver_registry::EmbedRequest {
                    texts,
                    model: embedder.model_id.clone(),
                },
            )
            .await
            .map_err(|_| anyhow!("Knowledge embedding failed"))?;
        if let (Some(session), Some(events)) = (self.query_session, &self.events) {
            use everruns_contracts::runtime::events::{
                EventContext, EventData, EventRequest, LlmGenerationData,
            };
            let estimated_cost_usd = embedder.provider_type.parse().ok().and_then(|driver| {
                everruns_contracts::model_profiles::estimate_cost_usd(
                    &driver,
                    &embedder.model_id,
                    result.usage_tokens.unwrap_or(0),
                    0,
                    0,
                    0,
                )
            });
            let mut data = LlmGenerationData::success(
                vec![],
                vec![],
                None,
                vec![],
                embedder.model_id.clone(),
                Some(embedder.provider_type.clone()),
                Some(everruns_contracts::runtime::events::TokenUsage {
                    input_tokens: result.usage_tokens.unwrap_or(0),
                    output_tokens: 0,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    actual_cost_usd: result.actual_cost_usd,
                    estimated_cost_usd,
                    effective_cost_usd: None,
                }),
                None,
                None,
            )
            .with_provider_id(embedder.served_by.provider_config_id.clone());
            data.metadata.response_id = result.response_id.clone();
            events
                .emit(EventRequest {
                    event_type: "llm.generation".into(),
                    ts: chrono::Utc::now(),
                    session_id: session,
                    context: EventContext::empty(),
                    data: EventData::LlmGeneration(data),
                    metadata: None,
                    tags: Some(vec!["embedding".into(), "knowledge".into()]),
                })
                .await?;
        } else {
            // Background/admin embeddings have no session; agent queries use the event pipeline.
            self.db
                .create_llm_generation(
                    c.org_id,
                    None,
                    None,
                    None,
                    embedder.model_id,
                    Some(embedder.provider_type),
                    i64::from(result.usage_tokens.unwrap_or(0)),
                    0,
                    0,
                    0,
                    result.actual_cost_usd,
                    None,
                    None,
                    None,
                    result.response_id.clone(),
                    embedder.served_by,
                    chrono::Utc::now(),
                )
                .await?;
        }
        Ok(result.embeddings)
    }
    pub async fn index_file(&self, c: &Collection, job: &Job) -> Result<()> {
        let file: File = sqlx::query_as(
            "SELECT * FROM knowledge_files WHERE org_id=$1 AND id=$2 AND deleted_at IS NULL",
        )
        .bind(c.org_id)
        .bind(job.file_id)
        .fetch_optional(self.db.tx_pool())
        .await?
        .ok_or_else(|| anyhow!("file no longer available"))?;
        let Some(version) = job.version_id else {
            return Ok(());
        };
        if c.indexing == "off"
            || c.revision != job.revision
            || file.current_version != Some(version)
        {
            return Ok(());
        }
        let v = self.version(c.org_id, file.id, version).await?;
        let text = match v.text_key.as_ref() {
            Some(key) => String::from_utf8(
                Content::new(self.db.clone())
                    .range(key, v.text_bytes as u64, 0, v.text_bytes as usize)
                    .await?,
            )?,
            None => String::new(),
        };
        let allowed = file.indexable
            && file.readable
            && !excluded(c, "read_exclude", &file.path)
            && !excluded(c, "index_exclude", &file.path);
        let ranges = if allowed {
            super::extraction::fragments(&text)
        } else {
            vec![]
        };
        let remote = self.backend(c).await?;
        ensure!(
            c.indexing != "hybrid" || remote.is_some(),
            "Hybrid indexing requires a persistent vector backend"
        );
        let store = Content::new(self.db.clone());
        let model_profile = if let Some(id) =
            c.embedding_model_id.filter(|_| c.indexing == "hybrid")
        {
            let model = self
                .db
                .get_model(c.org_id, id)
                .await?
                .ok_or_else(|| anyhow!("Embedding model unavailable"))?;
            let provider = self
                .db
                .get_provider(c.org_id, model.provider_id.uuid())
                .await?
                .ok_or_else(|| anyhow!("Embedding provider unavailable"))?;
            hex::encode(Sha256::digest(serde_json::to_vec(
                &json!({"model":model.model_id,"metadata":model.provider_metadata,"provider":provider.id,"type":provider.provider_type,"url":provider.base_url,"settings":provider.settings,"profile":"knowledge-embedding-v1"}),
            )?))
        } else {
            "lexical".into()
        };
        let mut records = Vec::new();
        for batch in ranges.chunks(32) {
            let mut vectors = Vec::new();
            let mut missing = Vec::new();
            let mut keys = Vec::new();
            for (start, len) in batch {
                let passage = &text[*start..start + len];
                let key = format!(
                    "knowledge/org-{}/embeddings/{}/{}",
                    c.org_id,
                    model_profile,
                    hex::encode(Sha256::digest(passage.as_bytes()))
                );
                keys.push(key.clone());
                if c.indexing == "hybrid" {
                    match store.get(&key).await {
                        Ok(bytes) => {
                            vectors.push(Some(serde_json::from_slice::<Vec<f32>>(&bytes)?))
                        }
                        Err(_) => {
                            missing.push(passage.to_string());
                            vectors.push(None)
                        }
                    }
                } else {
                    vectors.push(Some(vec![]));
                }
            }
            if !missing.is_empty() {
                let fresh = self.embed(c, missing.clone()).await?;
                ensure!(fresh.len() == missing.len(), "embedding batch mismatch");
                let mut fresh = fresh.into_iter();
                for (index, entry) in vectors.iter_mut().enumerate() {
                    if entry.is_none() {
                        let vector = fresh.next().ok_or_else(|| anyhow!("missing embedding"))?;
                        store
                            .put(&keys[index], serde_json::to_vec(&vector)?)
                            .await?;
                        *entry = Some(vector);
                    }
                }
            }
            for ((start, len), vector) in batch.iter().zip(vectors) {
                records.push(VectorRecord {
                    id: fragment_id(version, c.revision, *start, *len),
                    document_id: version.to_string(),
                    vector: vector.unwrap_or_default(),
                    text: text[*start..start + len].to_string(),
                });
            }
            if let Some(remote) = &remote {
                remote
                    .upsert(&namespace(c), std::mem::take(&mut records))
                    .await?;
            }
        }
        // Search writes are acknowledged before the catalog's visibility fence moves.
        let mut tx = self.db.tx_pool().begin().await?;
        // Lock the same collection row as upload/configuration, then the claimed job.
        let _: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM knowledge_collections WHERE id=$1 FOR UPDATE")
                .bind(c.id)
                .fetch_optional(&mut *tx)
                .await?;
        let _: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM knowledge_jobs WHERE id=$1 FOR UPDATE")
                .bind(job.id)
                .fetch_optional(&mut *tx)
                .await?;
        let fence:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM knowledge_jobs j JOIN knowledge_collections c ON c.id=j.collection_id JOIN knowledge_files f ON f.id=j.file_id WHERE j.id=$1 AND j.lease_token=$2 AND j.status='running' AND j.lease_until>now() AND c.revision=j.revision AND c.status='active' AND f.current_version=j.version_id AND f.deleted_at IS NULL)").bind(job.id).bind(job.lease_token).fetch_one(&mut *tx).await?;
        if !fence {
            return Ok(());
        }
        let previous: Option<Uuid> = sqlx::query_scalar(
            "SELECT version_id FROM knowledge_file_projections WHERE file_id=$1 AND revision=$2",
        )
        .bind(file.id)
        .bind(c.revision)
        .fetch_optional(&mut *tx)
        .await?;
        for (ordinal, (start, len)) in ranges.iter().enumerate() {
            sqlx::query("INSERT INTO knowledge_fragments(id,org_id,collection_id,file_id,version_id,profile,ordinal,byte_start,byte_length,terms) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,to_tsvector('simple',$10)) ON CONFLICT(id) DO NOTHING")
            .bind(fragment_id(version,c.revision,*start,*len)).bind(c.org_id).bind(c.id).bind(file.id).bind(version).bind(c.revision).bind(ordinal as i32).bind(*start as i32).bind(*len as i32).bind(if remote.is_none() { &text[*start..start+len] } else { "" }).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO knowledge_file_projections(org_id,collection_id,file_id,revision,version_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(file_id,revision) DO UPDATE SET version_id=EXCLUDED.version_id").bind(c.org_id).bind(c.id).bind(file.id).bind(c.revision).bind(version).execute(&mut *tx).await?;
        sqlx::query("UPDATE knowledge_files SET indexed_version=$3,indexed_revision=$4 WHERE org_id=$1 AND id=$2").bind(c.org_id).bind(file.id).bind(version).bind(c.revision).execute(&mut *tx).await?;
        tx.commit().await?;
        if let (Some(remote), Some(previous)) = (&remote, previous)
            && previous != version
            && remote
                .delete_by_document(&namespace(c), &previous.to_string())
                .await
                .is_err()
        {
            tracing::warn!(collection_id=%c.id,"Previous Knowledge search projection awaits cleanup");
        }
        self.publish_generation(c).await?;
        Ok(())
    }
    pub async fn publish_generation(&self, c: &Collection) -> Result<()> {
        sqlx::query("UPDATE knowledge_collections c SET published_revision=revision,serving_config=$3 WHERE org_id=$1 AND id=$2 AND revision=$4 AND NOT EXISTS(SELECT 1 FROM knowledge_files f LEFT JOIN knowledge_file_projections p ON p.file_id=f.id AND p.revision=c.revision WHERE f.collection_id=c.id AND f.deleted_at IS NULL AND (p.version_id IS NULL OR p.version_id!=f.current_version))")
            .bind(c.org_id).bind(c.id).bind(json!({"indexer_id":c.indexer_id,"embedding_model_id":c.embedding_model_id,"indexing":c.indexing})).bind(c.revision).execute(self.db.tx_pool()).await?;
        Ok(())
    }
    pub async fn search(
        &self,
        org: i64,
        query: KnowledgeQuery,
    ) -> Result<KnowledgeResults, CommandError> {
        if query.query.trim().is_empty()
            || query.query.len() > 4096
            || !(1..=25).contains(&query.limit)
            || query.collections.len() > 32
            || serde_json::to_vec(&query.metadata).map_or(true, |v| v.len() > 4096)
        {
            return Err(CommandError::bad_request("Invalid bounded knowledge query"));
        }
        let mut output = KnowledgeResults::default();
        let mut ranks = Vec::new();
        for key in &query.collections {
            let mut c = match self.collection(org, key).await {
                Ok(c) => c,
                Err(_) => continue,
            };
            if c.indexing == "off" {
                output
                    .warnings
                    .push(format!("{} is unindexed; browse or grep its mount", c.name));
                continue;
            }
            let current = c.revision;
            c.revision = c.published_revision;
            if let Some(indexer) = c.serving_config.get("indexer_id") {
                c.indexer_id = serde_json::from_value(indexer.clone()).unwrap_or(None)
            }
            if let Some(model) = c.serving_config.get("embedding_model_id") {
                c.embedding_model_id = serde_json::from_value(model.clone()).unwrap_or(None)
            }
            if let Some(mode) = c.serving_config.get("indexing").and_then(Value::as_str) {
                c.indexing = mode.into();
            }
            let limit = 500i64;
            let backend = self
                .backend(&c)
                .await
                .map_err(|_| CommandError::unavailable("Knowledge indexer unavailable"))?;
            if c.indexing == "hybrid" && backend.is_none() {
                return Err(CommandError::unavailable(
                    "Hybrid Knowledge requires its persistent indexer",
                ));
            }
            let rows = if let Some(remote) = backend {
                let vector = if c.indexing == "hybrid" && !query.lexical_only {
                    Some(
                        self.embed(&c, vec![query.query.clone()])
                            .await
                            .map_err(|_| CommandError::unavailable("Embedding query failed"))?
                            .into_iter()
                            .next()
                            .ok_or_else(|| CommandError::unavailable("Empty query embedding"))?,
                    )
                } else {
                    None
                };
                let matches = remote
                    .query(
                        &namespace(&c),
                        VectorQuery {
                            vector,
                            text: Some(query.query.clone()),
                            top_k: limit as usize,
                        },
                    )
                    .await
                    .map_err(|_| CommandError::unavailable("Knowledge index search failed"))?;
                output.truncated |= matches.len() == limit as usize;
                let ids: Vec<String> = matches.into_iter().map(|m| m.id).collect();
                sqlx::query("SELECT g.*,f.path,v.source_uri,v.text_key,v.text_bytes,v.metadata FROM unnest($3::text[]) WITH ORDINALITY matches(id,rank) JOIN knowledge_fragments g ON g.id=matches.id JOIN knowledge_files f ON f.id=g.file_id JOIN knowledge_file_projections p ON p.file_id=f.id AND p.revision=$4 AND p.version_id=g.version_id JOIN knowledge_versions v ON v.id=g.version_id WHERE g.org_id=$1 AND g.collection_id=$2 AND g.profile=$4 AND f.readable AND f.indexable AND f.deleted_at IS NULL ORDER BY matches.rank")
                    .bind(org).bind(c.id).bind(ids).bind(c.revision).fetch_all(self.db.tx_pool()).await.map_err(db_error)?
            } else {
                sqlx::query("SELECT g.*,f.path,v.source_uri,v.text_key,v.text_bytes,v.metadata FROM knowledge_fragments g JOIN knowledge_files f ON f.id=g.file_id JOIN knowledge_file_projections p ON p.file_id=f.id AND p.version_id=g.version_id AND p.revision=$3 JOIN knowledge_versions v ON v.id=g.version_id WHERE g.org_id=$1 AND g.collection_id=$2 AND g.profile=$3 AND f.readable AND f.indexable AND f.deleted_at IS NULL AND g.terms @@ plainto_tsquery('simple',$4) AND ($5::text IS NULL OR starts_with(f.path,$5)) AND v.metadata @> $6 ORDER BY ts_rank_cd(g.terms,plainto_tsquery('simple',$4)) DESC,g.id LIMIT $7")
                    .bind(org).bind(c.id).bind(c.revision).bind(&query.query).bind(query.path.as_deref().map(path).transpose()?).bind(json!({"user":query.metadata})).bind(limit).fetch_all(self.db.tx_pool()).await.map_err(db_error)?
            };
            output.truncated |= rows.len() == limit as usize;
            if current != c.published_revision {
                output.warnings.push(format!(
                    "{} is rebuilding; serving its published index",
                    c.name
                ));
            }
            let mut rank = 0;
            for row in rows {
                let p: String = row.get("path");
                if excluded(&c, "read_exclude", &p) || excluded(&c, "index_exclude", &p) {
                    continue;
                }
                if let Some(prefix) = &query.path
                    && !p.starts_with(&path(prefix)?)
                {
                    continue;
                }
                let metadata: Value = row.get("metadata");
                if !query
                    .metadata
                    .iter()
                    .all(|(k, v)| metadata.get("user").and_then(|m| m.get(k)) == Some(v))
                {
                    continue;
                }
                rank += 1;
                let file: Uuid = row.get("file_id");
                let version: Uuid = row.get("version_id");
                let start: i32 = row.get("byte_start");
                let length: i32 = row.get("byte_length");
                let Some(text_key) = row.get::<Option<String>, _>("text_key") else {
                    continue;
                };
                let bytes = Content::new(self.db.clone())
                    .range(
                        &text_key,
                        row.get::<i32, _>("text_bytes") as u64,
                        start as u64,
                        length as usize,
                    )
                    .await
                    .map_err(CommandError::internal)?;
                let snippet = String::from_utf8(bytes)
                    .map_err(|_| CommandError::unavailable("Text representation unavailable"))?;
                let score = 1.0 / (60.0 + rank as f64);
                ranks.push(KnowledgeHit{id:row.get("id"),collection_id:c.public_id(),file_id:file.to_string(),version_id:version.to_string(),path:format!("/knowledge/{}/{p}",c.public_id()).replace("//","/"),title:metadata.pointer("/document/frontmatter/title").and_then(Value::as_str).unwrap_or(&p).into(),source_uri:row.get("source_uri"),location:json!({"representation":"extracted_text","byte_start":start,"byte_length":length}),snippet:snippet.chars().take(1400).collect(),score});
                if rank >= query.limit {
                    break;
                }
            }
        }
        ranks.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        ranks.truncate(query.limit);
        output.results = ranks;
        Ok(output)
    }
}
fn fragment_id(version: Uuid, profile: i64, start: usize, len: usize) -> String {
    hex::encode(Sha256::digest(format!("{version}:{profile}:{start}:{len}")))
}
