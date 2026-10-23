//! Versioned knowledge retrieval. Credentials and persistence stay in the host.
use crate::{
    error::Result,
    typed_id::{MessageId, SessionId},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum IndexMode {
    #[default]
    Off,
    Lexical,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub automatic: bool,
    #[serde(default = "default_limit")]
    pub limit: usize,
}
fn default_limit() -> usize {
    8
}
impl KnowledgeConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.collections.len() > 32 || !(1..=25).contains(&self.limit) {
            return Err("Knowledge permits at most 32 collections and 1–25 results".into());
        }
        let mut seen = std::collections::HashSet::new();
        for id in &self.collections {
            if id != "everruns-docs"
                && !(id.starts_with("kn_")
                    && id.len() == 35
                    && id[3..].bytes().all(|c| c.is_ascii_hexdigit()))
            {
                return Err("Invalid Knowledge collection identifier".into());
            }
            if !seen.insert(id) {
                return Err("Duplicate Knowledge collection".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct KnowledgeQuery {
    pub query: String,
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub lexical_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KnowledgeHit {
    pub id: String,
    pub collection_id: String,
    pub file_id: String,
    pub version_id: String,
    pub path: String,
    pub title: String,
    pub source_uri: String,
    pub location: serde_json::Value,
    pub snippet: String,
    pub score: f64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct KnowledgeResults {
    pub results: Vec<KnowledgeHit>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}
/// A bounded, session-authorized page through an indexed or unindexed catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct KnowledgeCatalogQuery {
    pub collection: String,
    pub path: Option<String>,
    pub after: Option<String>,
    #[serde(default = "catalog_limit")]
    pub limit: usize,
    #[serde(default)]
    pub include_metadata: bool,
}
fn catalog_limit() -> usize {
    50
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFileSummary {
    pub file_id: String,
    pub version_id: String,
    pub path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub extraction_status: String,
    pub indexable: bool,
    pub metadata: Option<serde_json::Value>,
    pub metadata_truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCatalog {
    pub collection_id: String,
    pub files: Vec<KnowledgeFileSummary>,
    pub next_cursor: Option<String>,
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait KnowledgeRuntime: Send + Sync {
    /// The implementation must intersect requested collections with live session bindings.
    async fn search(&self, session: SessionId, query: KnowledgeQuery) -> Result<KnowledgeResults>;
    /// Browse only live attached collections; enforce file denies before returning metadata.
    async fn files(
        &self,
        _session: SessionId,
        _query: KnowledgeCatalogQuery,
    ) -> Result<KnowledgeCatalog> {
        Err(crate::error::AgentLoopError::config(
            "Knowledge browsing unavailable",
        ))
    }
    /// Persist bounded retrieval evidence by input ID; replay must reuse it.
    async fn automatic_context(
        &self,
        session: SessionId,
        input: MessageId,
        query: String,
    ) -> Result<Option<String>>;
}
#[derive(Clone)]
pub struct KnowledgeRuntimeExt(pub Arc<dyn KnowledgeRuntime>);

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            collections: vec![],
            automatic: false,
            limit: default_limit(),
        }
    }
}
impl Default for KnowledgeQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            collections: vec![],
            path: None,
            metadata: Default::default(),
            limit: default_limit(),
            lexical_only: false,
        }
    }
}

/// Platform mount roots; normalize workspace aliases before comparing.
pub fn is_mount(path: &str) -> bool {
    let path = crate::runtime::session_path::to_session_path(path);
    path == "/knowledge"
        || path.starts_with("/knowledge/")
        || path == "/knowledge-text"
        || path.starts_with("/knowledge-text/")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_binding_validation() {
        assert_eq!(KnowledgeConfig::default().limit, 8);
        assert_eq!(
            serde_json::from_str::<KnowledgeConfig>("{}").unwrap().limit,
            8
        );
        assert!(KnowledgeConfig::default().validate().is_ok());
        let mut config = KnowledgeConfig {
            collections: vec!["everruns-docs".into()],
            ..Default::default()
        };
        assert!(config.validate().is_ok());
        config.collections.push("everruns-docs".into());
        assert!(config.validate().is_err());
        assert!(serde_json::from_str::<KnowledgeConfig>(r#"{"token":"secret"}"#).is_err());
    }
}
