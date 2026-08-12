//! Server-side `KnowledgeStore` over `StorageBackend`, backing the agent-facing
//! `search_knowledge` tool. See knowledge/runtime-resources/okf-adoption.md and knowledge/runtime-resources/knowledge-bases.md.
//!
//! Scopes every search to the caller's org and resolves bound Knowledge Base
//! public ids within that org, so ids from other orgs are silently skipped
//! (no cross-tenant existence leak).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::organization::org_internal_id_from_public;
use everruns_core::typed_id::{KnowledgeBaseId, OrgId};
use everruns_platform::{KnowledgeSearchHit, KnowledgeStore};

use crate::storage::StorageBackend;

/// Max characters of an entry body returned as a search snippet.
const SNIPPET_LEN: usize = 400;

pub struct StorageBackendKnowledgeStore {
    db: Arc<StorageBackend>,
}

impl StorageBackendKnowledgeStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= SNIPPET_LEN {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(SNIPPET_LEN).collect();
    format!("{cut}…")
}

#[async_trait]
impl KnowledgeStore for StorageBackendKnowledgeStore {
    async fn search_knowledge(
        &self,
        org_id: OrgId,
        kb_public_ids: &[String],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeSearchHit>> {
        let internal_org = org_internal_id_from_public(org_id);

        // Resolve each bound KB within the org. Unknown or other-org ids resolve
        // to None and are skipped — no existence leak.
        let mut kb_uuids = Vec::new();
        let mut public_by_uuid: HashMap<uuid::Uuid, String> = HashMap::new();
        for pid in kb_public_ids {
            let Ok(kb_id) = pid.parse::<KnowledgeBaseId>() else {
                continue;
            };
            if let Some(kb) = self
                .db
                .get_knowledge_base(internal_org, kb_id)
                .await
                .map_err(|e| AgentLoopError::store(format!("resolve knowledge base: {e}")))?
            {
                public_by_uuid.insert(kb.id, kb.public_id.clone());
                kb_uuids.push(kb.id);
            }
        }
        if kb_uuids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = self
            .db
            .search_knowledge_entries(&kb_uuids, query, kind, tags, limit)
            .await
            .map_err(|e| AgentLoopError::store(format!("search knowledge entries: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| KnowledgeSearchHit {
                id: r.public_id,
                kb_id: public_by_uuid.get(&r.kb_id).cloned().unwrap_or_default(),
                title: r.title,
                kind: r.kind,
                // Hide reserved okf:* round-trip tags from the agent.
                tags: r
                    .tags
                    .into_iter()
                    .filter(|t| !t.starts_with("okf:"))
                    .collect(),
                snippet: snippet(&r.body),
                resource: r.resource,
            })
            .collect())
    }
}
