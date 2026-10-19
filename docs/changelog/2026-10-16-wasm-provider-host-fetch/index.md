---
title: "Run Everruns provider drivers with WASM host fetch"
description: "Use shared OpenAI and OpenRouter chat drivers in Workers and Durable Objects with an injected HTTP adapter, including streaming and tool calls."
date: 2026-10-16
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/models-and-providers/]
---

OpenAI-compatible and OpenRouter chat drivers now run on
`wasm32-unknown-unknown`. Applications supply an HTTP adapter backed by host
`fetch`; the existing drivers handle request encoding, provider options,
streaming responses, tool calls, retries and provider errors.

```rust
use std::sync::Arc;
use everruns_contracts::http_transport::HttpTransport;
use everruns_drivers::openrouter::OpenRouterChatDriver;

fn driver(fetch: Arc<dyn HttpTransport>) -> OpenRouterChatDriver {
    OpenRouterChatDriver::new().with_transport(fetch)
}
```

The celld engine example uses the shared driver with a Workers-compatible
adapter. Integration tests exercise model and tool-call exchanges inside both
workerd Worker handlers and Durable Objects. Native applications keep their
existing HTTP client unless they inject a transport.

See [Models and Providers](/framework/models-and-providers/#wasm-and-host-fetch)
for the adapter contract and supported chat drivers.
