---
title: Keep Knowledge embedding usage tied to its provider account
description: Knowledge indexing and query embeddings retain the serving provider account, keeping managed usage distinct from calls billed to your own keys.
date: 2026-10-28
kind: breaking
significance: notable
products: [platform, framework]
surfaces: [operations, api, code]
links:
  docs: [/capabilities/knowledge/]
---

Knowledge embedding usage now retains the provider account that served the call,
including background indexing without a session. Hosts can distinguish managed
model usage from calls made with an organization's own provider key. Query
embeddings retain that attribution even when no documents match.

Custom embedding driver implementations must include the new `response_id` field
in `EmbedResponse`. Populate it when available, or return `None`.

```rust
EmbedResponse {
    embeddings,
    usage_tokens,
    actual_cost_usd,
    response_id: None, // Set the serving endpoint's response ID when available.
}
```

Deployment-owned search backends can also retain provider-reported index operation
receipts. These records contain resource quantities for host accounting; they do
not set monetary prices. See [Knowledge](/capabilities/knowledge/) for storage,
indexing and provider configuration.
