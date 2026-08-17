# everruns-turbopuffer

> Turbopuffer vector-store backend for Everruns Knowledge Indexes.

Part of the [Everruns](https://everruns.com) ecosystem.

## What It Provides

Implements the hosted `VectorStore` trait from `everruns-platform` against
Turbopuffer's v2 HTTP API. This is the reference production backend; the
in-memory store stays the default and Turbopuffer is **opt-in** via the
`TURBOPUFFER_API_KEY` environment variable (regional endpoint via
`TURBOPUFFER_BASE_URL`).

Each Knowledge Index maps to one org-prefixed Turbopuffer namespace, so the
store is multitenant and multi-index by construction. Hybrid retrieval fuses
vector ANN and BM25 with reciprocal-rank fusion (RRF) server-side.

```rust
use everruns_turbopuffer::TurbopufferVectorStore;

let _store = TurbopufferVectorStore::new("https://example.turbopuffer.com", "api-key");
```

## Documentation

- [Everruns documentation](https://docs.everruns.com)
- [API reference](https://docs.rs/everruns-turbopuffer)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
