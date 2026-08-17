//! Turbopuffer vector-store backend for Knowledge Indexes.
//!
//! Implements [`everruns_platform::vector_store::VectorStore`] against Turbopuffer's
//! v2 HTTP API (<https://turbopuffer.com/docs>). This is the reference
//! production backend; the in-memory store stays the default and Turbopuffer is
//! opt-in via `TURBOPUFFER_API_KEY` (see `crates/server/src/platform.rs`).
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```rust
//! use everruns_turbopuffer::TurbopufferVectorStore;
//! let _store = TurbopufferVectorStore::new("https://example.turbopuffer.com", "api-key");
//! ```
//!
//! Multitenancy and multi-index isolation are handled by the caller: each
//! Knowledge Index gets one org-prefixed namespace (see
//! [`everruns_platform::vector_store::index_namespace`]), and this backend simply
//! maps each namespace string onto a Turbopuffer namespace path segment.
//!
//! ## Error handling
//!
//! Non-2xx responses are mapped to sanitized [`anyhow`] errors. The API key is
//! only ever sent in the `Authorization` header and is **never** included in any
//! error message or log line.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use everruns_platform::vector_store::{VectorMatch, VectorQuery, VectorRecord, VectorStore};
use serde::Deserialize;
use serde_json::{Value, json};

/// Distance metric used for all namespaces. Cosine matches the in-memory
/// backend's ranking so the two stores rank embeddings the same way.
const DISTANCE_METRIC: &str = "cosine_distance";

/// Turbopuffer-backed [`VectorStore`].
///
/// Cheap to clone is unnecessary; construct once and share behind an `Arc` (as
/// the platform definition does).
pub struct TurbopufferVectorStore {
    http: reqwest::Client,
    /// Regional base URL, e.g. `https://gcp-us-central1.turbopuffer.com`. No
    /// trailing slash.
    base_url: String,
    api_key: String,
}

impl TurbopufferVectorStore {
    /// Build a new backend. `base_url` is the regional Turbopuffer endpoint
    /// (any trailing slash is trimmed); `api_key` authenticates every request.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            // EVE-635: shared client with connect + overall request timeouts so
            // a hung vector-store read cannot block indefinitely.
            http: everruns_provider::driver_helpers::shared_request_http_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    fn namespace_url(&self, namespace: &str) -> String {
        format!("{}/v2/namespaces/{namespace}", self.base_url)
    }

    /// POST a JSON body to the namespace write endpoint and require a 2xx.
    async fn post_write(&self, namespace: &str, body: Value, op: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.namespace_url(namespace))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("turbopuffer {op} request failed"))?;
        ensure_success(resp, op).await?;
        Ok(())
    }
}

#[async_trait]
impl VectorStore for TurbopufferVectorStore {
    async fn upsert(&self, namespace: &str, records: Vec<VectorRecord>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        // Derive the concrete vector dimension from the records so the schema
        // declares `[N]f32`. All records in one index share `vector_dim`.
        let dim = records[0].vector.len();
        let rows: Vec<Value> = records
            .into_iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "vector": r.vector,
                    "text": r.text,
                    "document_id": r.document_id,
                })
            })
            .collect();

        // Send the schema on every upsert so ANN + BM25 are enabled. Turbopuffer
        // treats the schema as idempotent; re-sending it is a no-op once set.
        let body = json!({
            "upsert_rows": rows,
            "distance_metric": DISTANCE_METRIC,
            "schema": {
                "text": { "type": "string", "full_text_search": true },
                "vector": { "type": format!("[{dim}]f32"), "ann": true },
                "document_id": { "type": "string" },
            },
        });
        self.post_write(namespace, body, "upsert").await
    }

    async fn query(&self, namespace: &str, query: VectorQuery) -> Result<Vec<VectorMatch>> {
        if query.top_k == 0 {
            return Ok(Vec::new());
        }
        let top_k = query.top_k;
        let vector = query.vector.filter(|v| !v.is_empty());
        let text = query
            .text
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());

        // Pick a query shape based on the available signals and remember whether
        // the response `$dist` is a raw ANN distance (lower = better, must be
        // negated) or a relevance score (BM25 / RRF; higher = better).
        let (body, dist_is_distance) = match (vector, text) {
            // Hybrid: multi-query (ANN + BM25) fused server-side with RRF. The
            // fused `$dist` is an RRF score where higher is more relevant.
            (Some(v), Some(t)) => (
                json!({
                    "queries": [
                        { "rank_by": ["vector", "ANN", v], "top_k": top_k,
                          "include_attributes": ["document_id"] },
                        { "rank_by": ["text", "BM25", t], "top_k": top_k,
                          "include_attributes": ["document_id"] },
                    ],
                    "rerank_by": ["RRF"],
                }),
                false,
            ),
            // Vector only: raw ANN with cosine_distance. `$dist` is a distance
            // where lower = closer, so we negate it into `score`.
            (Some(v), None) => (
                json!({
                    "rank_by": ["vector", "ANN", v],
                    "top_k": top_k,
                    "include_attributes": ["document_id"],
                }),
                true,
            ),
            // Text only: BM25 relevance score, higher = better.
            (None, Some(t)) => (
                json!({
                    "rank_by": ["text", "BM25", t],
                    "top_k": top_k,
                    "include_attributes": ["document_id"],
                }),
                false,
            ),
            // No signal: nothing to rank against.
            (None, None) => return Ok(Vec::new()),
        };

        let resp = self
            .http
            .post(format!("{}/query", self.namespace_url(namespace)))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("turbopuffer query request failed")?;
        let resp = ensure_success(resp, "query").await?;
        let parsed: QueryResponse = resp
            .json()
            .await
            .context("turbopuffer query: malformed response body")?;

        // Both single- and multi-query (RRF) responses surface a fused `rows`
        // array already ordered best-first. Map `$dist` to `VectorMatch.score`
        // so ordering is best-first regardless of mode: negate for raw ANN
        // distance (lower distance => higher score), pass through for BM25/RRF
        // relevance scores. We preserve the server's ordering and only take the
        // requested `top_k`.
        let matches = parsed
            .rows
            .into_iter()
            .take(top_k)
            .map(|row| {
                let raw = row.dist.unwrap_or(0.0);
                let score = if dist_is_distance { -raw } else { raw };
                VectorMatch {
                    id: row.id,
                    document_id: row.document_id.unwrap_or_default(),
                    score,
                }
            })
            .collect();
        Ok(matches)
    }

    async fn delete_by_document(&self, namespace: &str, document_id: &str) -> Result<()> {
        let body = json!({
            "delete_by_filter": ["document_id", "Eq", document_id],
        });
        self.post_write(namespace, body, "delete_by_document").await
    }

    async fn delete_namespace(&self, namespace: &str) -> Result<()> {
        let resp = self
            .http
            .delete(self.namespace_url(namespace))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("turbopuffer delete_namespace request failed")?;
        // Idempotent: deleting an already-absent namespace is success.
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        ensure_success(resp, "delete_namespace").await?;
        Ok(())
    }
}

/// A single result row from the query endpoint. Turbopuffer keys the distance
/// as `$dist`; requested attributes (here `document_id`) are flattened onto the
/// row alongside `id`.
#[derive(Debug, Deserialize)]
struct QueryRow {
    id: String,
    #[serde(rename = "$dist")]
    dist: Option<f32>,
    #[serde(default)]
    document_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    #[serde(default)]
    rows: Vec<QueryRow>,
}

/// Require a 2xx status, returning a sanitized error otherwise.
///
/// The API key lives only in the request `Authorization` header and is never
/// part of the response body or the error we build here, so error text is safe
/// to surface and log.
async fn ensure_success(resp: reqwest::Response, op: &str) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    // Read the body for diagnostics; Turbopuffer returns a JSON error message.
    // It never echoes the API key.
    let body = resp.text().await.unwrap_or_default();
    let detail = sanitize_error_body(&body);
    bail!("turbopuffer {op} failed: HTTP {status}{detail}");
}

/// Trim and bound the error body so we never log unbounded server output.
fn sanitize_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    const MAX: usize = 500;
    let snippet: String = trimmed.chars().take(MAX).collect();
    format!(": {snippet}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn record(id: &str, doc: &str, vector: Vec<f32>, text: &str) -> VectorRecord {
        VectorRecord {
            id: id.to_string(),
            vector,
            text: text.to_string(),
            document_id: doc.to_string(),
        }
    }

    #[tokio::test]
    async fn upsert_sends_rows_schema_and_distance_metric() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a"))
            .and(body_json(json!({
                "upsert_rows": [
                    { "id": "kchk_a", "vector": [1.0, 0.0], "text": "alpha", "document_id": "kidoc_1" },
                ],
                "distance_metric": "cosine_distance",
                "schema": {
                    "text": { "type": "string", "full_text_search": true },
                    "vector": { "type": "[2]f32", "ann": true },
                    "document_id": { "type": "string" },
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        store
            .upsert(
                "org_1__kidx_a",
                vec![record("kchk_a", "kidoc_1", vec![1.0, 0.0], "alpha")],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upsert_empty_is_noop() {
        // No mock mounted: an HTTP call would fail. Empty upsert must not call.
        let store = TurbopufferVectorStore::new("http://127.0.0.1:1", "secret-key");
        store.upsert("org_1__kidx_a", vec![]).await.unwrap();
    }

    #[tokio::test]
    async fn query_vector_only_negates_distance_into_score() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a/query"))
            .and(body_json(json!({
                "rank_by": ["vector", "ANN", [1.0, 0.0]],
                "top_k": 2,
                "include_attributes": ["document_id"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "rows": [
                    { "id": "kchk_a", "$dist": 0.1, "document_id": "kidoc_1" },
                    { "id": "kchk_c", "$dist": 0.4, "document_id": "kidoc_2" },
                ],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let matches = store
            .query(
                "org_1__kidx_a",
                VectorQuery {
                    vector: Some(vec![1.0, 0.0]),
                    text: None,
                    top_k: 2,
                },
            )
            .await
            .unwrap();

        let ids: Vec<_> = matches.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["kchk_a", "kchk_c"]);
        // ANN cosine_distance: lower $dist => higher score (negated), best-first.
        assert!(matches[0].score > matches[1].score);
        assert_eq!(matches[0].score, -0.1);
        assert_eq!(matches[0].document_id, "kidoc_1");
    }

    #[tokio::test]
    async fn query_text_only_passes_bm25_score_through() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a/query"))
            .and(body_json(json!({
                "rank_by": ["text", "BM25", "quick fox"],
                "top_k": 5,
                "include_attributes": ["document_id"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "rows": [ { "id": "kchk_a", "$dist": 3.2, "document_id": "kidoc_1" } ],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let matches = store
            .query(
                "org_1__kidx_a",
                VectorQuery {
                    vector: None,
                    text: Some("  quick fox  ".to_string()),
                    top_k: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "kchk_a");
        // BM25: higher $dist => higher score (passed through).
        assert_eq!(matches[0].score, 3.2);
    }

    #[tokio::test]
    async fn query_hybrid_uses_multi_query_with_rrf() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a/query"))
            .and(body_json(json!({
                "queries": [
                    { "rank_by": ["vector", "ANN", [1.0, 0.0]], "top_k": 2,
                      "include_attributes": ["document_id"] },
                    { "rank_by": ["text", "BM25", "database"], "top_k": 2,
                      "include_attributes": ["document_id"] },
                ],
                "rerank_by": ["RRF"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "rows": [
                    { "id": "kchk_a", "$dist": 0.032, "document_id": "kidoc_1" },
                    { "id": "kchk_b", "$dist": 0.016, "document_id": "kidoc_1" },
                ],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let matches = store
            .query(
                "org_1__kidx_a",
                VectorQuery {
                    vector: Some(vec![1.0, 0.0]),
                    text: Some("database".to_string()),
                    top_k: 2,
                },
            )
            .await
            .unwrap();
        // RRF fused $dist: higher => better, preserve server order best-first.
        assert_eq!(matches[0].id, "kchk_a");
        assert!(matches[0].score >= matches[1].score);
    }

    #[tokio::test]
    async fn delete_by_document_posts_filter() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a"))
            .and(body_json(json!({
                "delete_by_filter": ["document_id", "Eq", "kidoc_1"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        store
            .delete_by_document("org_1__kidx_a", "kidoc_1")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_namespace_uses_delete_verb() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("DELETE"))
            .and(path("/v2/namespaces/org_1__kidx_a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        store.delete_namespace("org_1__kidx_a").await.unwrap();
    }

    #[tokio::test]
    async fn delete_namespace_treats_404_as_success() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("DELETE"))
            .and(path("/v2/namespaces/org_1__missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": "namespace not found",
            })))
            .expect(1)
            .mount(&server)
            .await;

        store.delete_namespace("org_1__missing").await.unwrap();
    }

    #[tokio::test]
    async fn non_2xx_fails_closed_without_leaking_api_key() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "super-secret-api-key");

        Mock::given(method("POST"))
            .and(path("/v2/namespaces/org_1__kidx_a"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "unauthorized",
            })))
            .mount(&server)
            .await;

        let err = store
            .upsert(
                "org_1__kidx_a",
                vec![record("kchk_a", "kidoc_1", vec![1.0, 0.0], "alpha")],
            )
            .await
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("401"), "error should report status: {msg}");
        assert!(
            !msg.contains("super-secret-api-key"),
            "error must not leak the API key: {msg}"
        );
    }

    #[tokio::test]
    async fn query_top_k_zero_skips_request() {
        // No mock: a request would fail. top_k == 0 must short-circuit.
        let store = TurbopufferVectorStore::new("http://127.0.0.1:1", "secret-key");
        let matches = store
            .query(
                "org_1__kidx_a",
                VectorQuery {
                    vector: Some(vec![1.0]),
                    text: None,
                    top_k: 0,
                },
            )
            .await
            .unwrap();
        assert!(matches.is_empty());
    }

    #[tokio::test]
    async fn auth_header_carries_bearer_key() {
        let server = MockServer::start().await;
        let store = TurbopufferVectorStore::new(server.uri(), "secret-key");

        Mock::given(method("DELETE"))
            .and(path("/v2/namespaces/org_1__kidx_a"))
            .and(|req: &Request| {
                req.headers
                    .get("authorization")
                    .map(|v| v == "Bearer secret-key")
                    .unwrap_or(false)
            })
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        store.delete_namespace("org_1__kidx_a").await.unwrap();
    }

    /// Live test: hits the real Turbopuffer API. Gated behind the
    /// `turbopuffer-live-tests` feature so it never runs in the default suite.
    /// When the feature is on but `TURBOPUFFER_API_KEY` is missing/empty the
    /// test fails closed (panics), per knowledge/integrations/integrations.md parity.
    #[cfg(feature = "turbopuffer-live-tests")]
    #[tokio::test]
    async fn live_roundtrip() {
        let api_key = std::env::var("TURBOPUFFER_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| {
                panic!(
                    "turbopuffer-live-tests enabled but TURBOPUFFER_API_KEY is missing/empty; \
                     live tests must fail closed"
                )
            });
        let base_url = std::env::var("TURBOPUFFER_BASE_URL")
            .unwrap_or_else(|_| "https://gcp-us-central1.turbopuffer.com".to_string());
        let store = TurbopufferVectorStore::new(base_url, api_key);
        let ns = "org_test__kidx_live_roundtrip";
        store.delete_namespace(ns).await.unwrap();
        store
            .upsert(
                ns,
                vec![record(
                    "kchk_live",
                    "kidoc_live",
                    vec![1.0, 0.0],
                    "alpha live",
                )],
            )
            .await
            .unwrap();
        store.delete_namespace(ns).await.unwrap();
    }
}
