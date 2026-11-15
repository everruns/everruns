//! Application-owned Knowledge collections and citable retrieval. Stability: alpha.
use crate::capability::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result as RuntimeResult},
    knowledge::*,
    runtime::session_services::SessionStorageStore,
    typed_id::{MessageId, SessionId},
};
use serde_json::{Map, Value, json};
use std::{fmt, path::Path, sync::Arc};
mod files;

/// An original text document and its citation provenance.
#[derive(Clone, Debug)]
pub struct Document {
    /// Reader-facing title.
    pub title: String,
    /// Original UTF-8 contents.
    pub text: String,
    /// Stable citation URI.
    pub uri: String,
    /// Source-relative path. Traversal and empty components are rejected.
    pub path: String,
    /// Application annotations, also searchable with metadata filters.
    pub metadata: Map<String, Value>,
}
impl Document {
    /// Create a document with a stable URI-derived Markdown path.
    pub fn new(title: impl Into<String>, text: impl Into<String>, uri: impl Into<String>) -> Self {
        let uri = uri.into();
        let path = format!(
            "{}.md",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, uri.as_bytes()).simple()
        );
        Self {
            title: title.into(),
            text: text.into(),
            uri,
            path,
            metadata: Map::new(),
        }
    }
    /// Preserve an original source-relative path in the collection.
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }
}
/// A ranked passage from an application retriever.
#[derive(Clone, Debug)]
pub struct Passage {
    /// Document and source provenance.
    pub document: Document,
    /// Finite relevance score, higher is better.
    pub score: f32,
    /// Optional source location.
    pub location: Option<Value>,
}
/// A bounded application retrieval request.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Search {
    /// Search text.
    pub query: String,
    /// Optional source-relative path prefix.
    pub path: Option<String>,
    /// Exact application-annotation filters.
    pub metadata: Map<String, Value>,
    /// Maximum passages.
    pub limit: usize,
    /// Request lexical retrieval without query embeddings.
    pub lexical_only: bool,
}
/// Implement semantic/vector retrieval with application-owned services.
#[async_trait]
pub trait Retriever: Send + Sync {
    /// Return relevant passages; errors are sanitized before model output.
    async fn search(&self, request: Search) -> Result<Vec<Passage>, String>;
}
/// One named collection. Local sources expose originals through read-only mounts.
#[derive(Clone)]
pub struct Knowledge {
    pub(crate) source: Arc<Source>,
    automatic: bool,
    limit: usize,
}
pub(crate) struct Source {
    id: String,
    name: String,
    retriever: Arc<dyn Retriever>,
    documents: Vec<Document>,
    files: files::Snapshot,
}
impl fmt::Debug for Knowledge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Knowledge")
            .field("collection", &self.source.id)
            .finish_non_exhaustive()
    }
}
fn id(key: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).to_string()
}
fn collection(name: &str) -> String {
    format!(
        "kn_{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, name.as_bytes()).simple()
    )
}
fn path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
impl Knowledge {
    /// Bind a search provider. Use `catalog` to supply browsable/readable originals.
    pub fn new(name: impl Into<String>, retriever: impl Retriever + 'static) -> Self {
        let name = name.into();
        Self {
            source: Arc::new(Source {
                id: collection(&name),
                name,
                retriever: Arc::new(retriever),
                documents: vec![],
                files: files::Snapshot::default(),
            }),
            automatic: false,
            limit: 8,
        }
    }
    /// Search local originals with a deterministic lexical scorer.
    pub fn documents(name: impl Into<String>, documents: Vec<Document>) -> Self {
        Self::new(name, Documents(documents.clone())).catalog(documents)
    }
    /// Attach original text files to an application retriever. Bounds and paths are validated at Agent build.
    pub fn catalog(mut self, documents: Vec<Document>) -> Self {
        self.source = Arc::new(Source {
            id: self.source.id.clone(),
            name: self.source.name.clone(),
            retriever: self.source.retriever.clone(),
            documents,
            files: files::Snapshot::default(),
        });
        self
    }
    /// Stable collection identity used by browse/search collection filters.
    pub fn collection_id(&self) -> &str {
        &self.source.id
    }
    /// Retrieve bounded context automatically before each input. Evidence is retained by input ID.
    pub fn automatic(mut self, enabled: bool) -> Self {
        self.automatic = enabled;
        self
    }
    /// Set the collection's maximum result count (1..=25), validated at Agent build.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
    /// Snapshot Markdown/text originals. Refuse symlinks, unbounded traversal and sources over 8 MiB.
    pub fn directory(name: impl Into<String>, root: impl AsRef<Path>) -> std::io::Result<Self> {
        let name = name.into();
        let root = std::fs::canonicalize(root)?;
        let mut pending = vec![root.clone()];
        let mut documents = vec![];
        let mut bytes = 0u64;
        let mut entries = 0;
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                entries += 1;
                if entries > 10000 {
                    return Err(std::io::Error::other(
                        "Knowledge source has too many entries",
                    ));
                }
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    return Err(std::io::Error::other("Knowledge source contains a symlink"));
                }
                if kind.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !kind.is_file()
                    || !entry
                        .path()
                        .extension()
                        .is_some_and(|e| e == "md" || e == "mdx" || e == "txt")
                {
                    continue;
                }
                bytes = bytes.saturating_add(entry.metadata()?.len());
                if bytes > 8 * 1024 * 1024 || documents.len() >= 1000 {
                    return Err(std::io::Error::other("Knowledge source exceeds limits"));
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(&root)
                    .map_err(std::io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path)?;
                documents.push(
                    Document::new(&relative, text, format!("knowledge://{name}/{relative}"))
                        .at(relative),
                );
            }
        }
        Ok(Self::documents(name, documents))
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.source.name.trim().is_empty()
            || self.source.name.len() > 200
            || !(1..=25).contains(&self.limit)
            || self.source.documents.len() > 1000
        {
            return Err("Invalid Knowledge name, result limit or document count".into());
        }
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0usize;
        for document in &self.source.documents {
            if !path_valid(&document.path) || !seen.insert(&document.path) {
                return Err(
                    "Knowledge document paths must be distinct contained relative paths".into(),
                );
            }
            bytes = bytes.saturating_add(
                document.text.len() + Value::Object(document.metadata.clone()).to_string().len(),
            );
            if bytes > 8 * 1024 * 1024 {
                return Err("Knowledge originals exceed 8 MiB".into());
            }
        }
        Ok(())
    }
    pub(crate) fn bind(&self, store: Arc<dyn SessionStorageStore>) -> Arc<Bound> {
        Arc::new(Bound {
            source: self.source.clone(),
            store,
            limit: self.limit,
        })
    }
    pub(crate) fn mount(&self) -> Arc<dyn everruns_core::session_files::SessionFileSystem> {
        Arc::new(files::CollectionFiles {
            source: self.source.clone(),
        })
    }
}
struct Documents(Vec<Document>);
#[async_trait]
impl Retriever for Documents {
    async fn search(&self, request: Search) -> Result<Vec<Passage>, String> {
        let terms = request
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let mut results = self
            .0
            .iter()
            .filter(|d| matches_document(d, request.path.as_deref(), &request.metadata))
            .filter_map(|document| {
                let text = format!("{} {}", document.title, document.text).to_lowercase();
                let score = terms.iter().filter(|t| text.contains(t.as_str())).count() as f32;
                (score > 0.0).then(|| Passage {
                    document: document.clone(),
                    score,
                    location: None,
                })
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.document.path.cmp(&b.document.path))
        });
        results.truncate(request.limit);
        Ok(results)
    }
}
fn matches_document(d: &Document, path: Option<&str>, metadata: &Map<String, Value>) -> bool {
    path.is_none_or(|p| d.path.starts_with(p.trim_start_matches('/')))
        && metadata
            .iter()
            .all(|(key, value)| d.metadata.get(key) == Some(value))
}
pub(crate) struct Bound {
    source: Arc<Source>,
    store: Arc<dyn SessionStorageStore>,
    limit: usize,
}
impl Bound {
    fn file_id(&self, document: &Document) -> String {
        id(&format!("{}:{}", self.source.id, document.path))
    }
    fn version_id(&self, document: &Document) -> String {
        id(&format!("{}:{}", self.file_id(document), document.text))
    }
}
#[async_trait]
impl KnowledgeRuntime for Bound {
    async fn search(
        &self,
        _session: SessionId,
        query: KnowledgeQuery,
    ) -> RuntimeResult<KnowledgeResults> {
        if !query.collections.is_empty() && !query.collections.contains(&self.source.id) {
            return Ok(KnowledgeResults::default());
        }
        if query.query.len() > 16 * 1024
            || query
                .path
                .as_ref()
                .is_some_and(|p| p.split('/').any(|part| part == ".."))
            || query.metadata.len() > 32
        {
            return Err(AgentLoopError::tool("Invalid Knowledge query"));
        }
        let limit = query.limit.clamp(1, self.limit);
        let mut passages = self
            .source
            .retriever
            .search(Search {
                query: query.query,
                path: query.path.clone(),
                metadata: query.metadata.clone(),
                limit,
                lexical_only: query.lexical_only,
            })
            .await
            .map_err(|_| AgentLoopError::store("Knowledge retrieval failed"))?;
        passages.retain(|p| {
            p.score.is_finite()
                && p.document.text.len() <= 8 * 1024 * 1024
                && path_valid(&p.document.path)
                && matches_document(&p.document, query.path.as_deref(), &query.metadata)
        });
        passages.sort_by(|a, b| b.score.total_cmp(&a.score));
        let truncated = passages.len() > limit;
        passages.truncate(limit);
        let results = passages
            .into_iter()
            .map(|p| KnowledgeHit {
                id: id(&format!(
                    "{}:{}",
                    self.version_id(&p.document),
                    p.location
                        .as_ref()
                        .map(Value::to_string)
                        .unwrap_or_default()
                )),
                collection_id: self.source.id.clone(),
                file_id: self.file_id(&p.document),
                version_id: self.version_id(&p.document),
                path: format!("/knowledge/{}/{}", self.source.id, p.document.path),
                title: p.document.title.chars().take(512).collect(),
                source_uri: p.document.uri.chars().take(4096).collect(),
                location: p
                    .location
                    .filter(|v| v.to_string().len() <= 16 * 1024)
                    .unwrap_or(Value::Null),
                snippet: p.document.text.chars().take(4000).collect(),
                score: p.score as f64,
            })
            .collect();
        Ok(KnowledgeResults {
            results,
            warnings: vec![],
            truncated,
        })
    }
    async fn files(
        &self,
        _session: SessionId,
        query: KnowledgeCatalogQuery,
    ) -> RuntimeResult<KnowledgeCatalog> {
        if query.collection != self.source.id {
            return Err(AgentLoopError::tool("Knowledge collection is not attached"));
        }
        if query
            .path
            .as_ref()
            .is_some_and(|p| p.split('/').any(|c| c == ".."))
        {
            return Err(AgentLoopError::tool("Invalid Knowledge path"));
        }
        let limit = query.limit.clamp(1, 100);
        let mut docs = self
            .source
            .documents
            .iter()
            .filter(|d| {
                query
                    .path
                    .as_ref()
                    .is_none_or(|p| d.path.starts_with(p.trim_start_matches('/')))
                    && query.after.as_ref().is_none_or(|after| d.path > *after)
            })
            .collect::<Vec<_>>();
        docs.sort_by(|a, b| a.path.cmp(&b.path));
        let more = docs.len() > limit;
        docs.truncate(limit);
        let next_cursor = more.then(|| docs.last().map(|d| d.path.clone())).flatten();
        let files = docs
            .into_iter()
            .map(|d| KnowledgeFileSummary {
                file_id: self.file_id(d),
                version_id: self.version_id(d),
                path: d.path.clone(),
                mime_type: "text/plain".into(),
                size_bytes: d.text.len() as i64,
                extraction_status: "ready".into(),
                indexable: true,
                metadata: query
                    .include_metadata
                    .then(|| Value::Object(d.metadata.clone()))
                    .filter(|m| m.to_string().len() <= 16 * 1024),
                metadata_truncated: query.include_metadata
                    && Value::Object(d.metadata.clone()).to_string().len() > 16 * 1024,
            })
            .collect();
        Ok(KnowledgeCatalog {
            collection_id: self.source.id.clone(),
            files,
            next_cursor,
            warnings: vec![],
        })
    }
    async fn automatic_context(
        &self,
        session: SessionId,
        input: MessageId,
        query: String,
    ) -> RuntimeResult<Option<String>> {
        let key = format!(
            "{}knowledge:{}:{input}",
            everruns_contracts::runtime::FRAMEWORK_SESSION_KV_PREFIX,
            self.source.id
        );
        if let Some(cached) = self.store.get_value(session, &key).await? {
            return Ok(Some(cached));
        }
        let results = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.search(
                session,
                KnowledgeQuery {
                    query,
                    limit: self.limit,
                    ..Default::default()
                },
            ),
        )
        .await
        .map_err(|_| AgentLoopError::store("Knowledge retrieval timed out"))??;
        let evidence = format!(
            "<knowledge-evidence untrusted=\"true\">\n{}\n</knowledge-evidence>",
            serde_json::to_string(&results)
                .map_err(|_| AgentLoopError::store("Invalid Knowledge evidence"))?
        );
        self.store.set_value(session, &key, &evidence).await?;
        Ok(Some(evidence))
    }
}
impl crate::AgentBuilder {
    /// Attach one application collection to the current native Knowledge tools and read-only mounts.
    pub fn knowledge(mut self, knowledge: Knowledge) -> Self {
        self = self.capability(crate::CapabilityRef::new("knowledge").config(json!({
            "collections": [knowledge.source.id],
            "automatic": knowledge.automatic,
            "limit": knowledge.limit,
        })));
        self.native_services.knowledge = Some(knowledge);
        self
    }
}
