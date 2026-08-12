# everruns-turbopuffer

Turbopuffer vector-store backend for Everruns Knowledge Indexes.

Implements the hosted `VectorStore` trait from `everruns-platform` against
Turbopuffer's v2 HTTP API. This is the reference production backend; the
in-memory store stays the default and Turbopuffer is **opt-in** via the
`TURBOPUFFER_API_KEY` environment variable (regional endpoint via
`TURBOPUFFER_BASE_URL`).

Each Knowledge Index maps to one org-prefixed Turbopuffer namespace, so the
store is multitenant and multi-index by construction. Hybrid retrieval fuses
vector ANN and BM25 with reciprocal-rank fusion (RRF) server-side.

See `knowledge/runtime-resources/knowledge-indexes.md` for the full design.
