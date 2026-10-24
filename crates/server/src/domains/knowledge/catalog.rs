//! Paginated navigation available to agents even when indexing is off.
use super::{repository::excluded, *};
use everruns_contracts::{
    knowledge::{KnowledgeCatalog, KnowledgeCatalogQuery, KnowledgeFileSummary},
    typed_id::SessionId,
};
use serde_json::Value;
use sqlx::Row;

impl KnowledgeService {
    pub async fn session_catalog(
        &self,
        org: i64,
        session: SessionId,
        query: KnowledgeCatalogQuery,
    ) -> Result<KnowledgeCatalog, CommandError> {
        if !(1..=100).contains(&query.limit)
            || query
                .after
                .as_ref()
                .is_some_and(|v| v.len() > 4096 || v.contains('\0'))
        {
            return Err(CommandError::bad_request("Invalid bounded catalog query"));
        }
        // THREAT[TM-FS-023]: browsing uses the calling session's live bindings,
        // never the attachments of another session sharing its workspace.
        let config = self.session_config(org, session).await?;
        if config.collections.iter().any(|c| c == "everruns-docs") {
            self.ensure_docs(org).await?;
        }
        let c = self.collection(org, &query.collection).await?;
        if !config.collections.iter().any(|key| {
            key == &c.public_id() || (key == "everruns-docs" && c.name == "everruns-docs")
        }) {
            return Err(CommandError::not_found("Attached Knowledge"));
        }
        let prefix = path(query.path.as_deref().unwrap_or("/"))?;
        let limit = if query.include_metadata {
            query.limit.min(10)
        } else {
            query.limit
        };
        let mut result = KnowledgeCatalog {
            collection_id: c.public_id(),
            files: vec![],
            next_cursor: None,
            warnings: vec![],
        };
        if limit < query.limit {
            result
                .warnings
                .push("Metadata pages are limited to ten files".into());
        }
        let mut after = query.after.unwrap_or_default();
        let mut last_visible = None;
        let mut examined = 0;
        loop {
            // Metadata is an explicit disclosure step. Cap its serialized size
            // in SQL, before transferring JSON into the runtime's memory.
            let rows=sqlx::query("SELECT f.id,f.path,f.current_version,f.indexable,v.mime_type,v.size_bytes,v.extraction_status,CASE WHEN $6 AND octet_length(v.metadata::text)<=65536 THEN v.metadata ELSE NULL END AS metadata,($6 AND octet_length(v.metadata::text)>65536) AS metadata_truncated FROM knowledge_files f JOIN knowledge_versions v ON v.id=f.current_version WHERE f.org_id=$1 AND f.collection_id=$2 AND f.deleted_at IS NULL AND f.readable AND f.path>$3 AND starts_with(f.path,$4) ORDER BY f.path LIMIT $5")
                .bind(org).bind(c.id).bind(&after).bind(&prefix).bind(if query.include_metadata { (limit+1) as i64 } else { 100i64 }).bind(query.include_metadata).fetch_all(self.db.tx_pool()).await.map_err(db_error)?;
            if rows.is_empty() {
                return Ok(result);
            }
            for row in rows {
                let p: String = row.get("path");
                after = p.clone();
                examined += 1;
                if !excluded(&c, "read_exclude", &p) {
                    if result.files.len() == limit {
                        result.next_cursor = last_visible;
                        return Ok(result);
                    }
                    let metadata: Option<Value> = row.get("metadata");
                    let metadata_truncated: bool = row.get("metadata_truncated");
                    result.files.push(KnowledgeFileSummary {
                        file_id: row.get::<Uuid, _>("id").to_string(),
                        version_id: row.get::<Uuid, _>("current_version").to_string(),
                        path: format!("/knowledge/{}{p}", c.public_id()),
                        mime_type: row.get("mime_type"),
                        size_bytes: row.get("size_bytes"),
                        extraction_status: row.get("extraction_status"),
                        indexable: row.get("indexable"),
                        metadata,
                        metadata_truncated,
                    });
                    last_visible = Some(p);
                }
                if examined >= 500 {
                    if result.files.is_empty() {
                        return Err(CommandError::bad_request(
                            "Catalog scan budget exceeded; narrow the path prefix",
                        ));
                    }
                    // Never put a denied filename in a cursor. A caller may
                    // need to narrow the path when a denied run is too large.
                    result.next_cursor = last_visible;
                    result
                        .warnings
                        .push("Catalog scan budget reached; narrow the path prefix".into());
                    return Ok(result);
                }
            }
        }
    }
}
