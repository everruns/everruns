# everruns-contracts

> Shared contracts for the Everruns framework and hosts.

Part of the [Everruns](https://everruns.com) ecosystem.

## What It Provides

This crate owns provider and driver boundaries, capability identity and code-defined
capability authoring, model profiles, typed IDs, and shared runtime contracts. It
replaces `everruns-provider`, `everruns-capability`, and `everruns-model-profiles`.

The default build includes capability `definition` authoring and omits HTTP transport.
Enable `http` for shared protocol drivers and clients, `responses-websocket` for
Responses WebSocket transport, `openapi` for schema derives, and `sqlx` for typed ID
encoding. `sqlx` supplies value encoding only; this crate never opens a connection.
The `session-sandbox` feature enables inventory registration for sandbox providers;
sandbox types and the neutral context interface remain available without it.

```rust
use everruns_contracts::{CapabilityRef, ModelSpec};

let model = ModelSpec::on("company-gateway", "assistant-v2");
let capability = CapabilityRef::new("vendor.search");
assert_eq!(model.provider.as_str(), "company-gateway");
assert_eq!(capability.id(), "vendor.search");
```

Provider APIs retain their existing root and module paths. Capability APIs live in
`capability` and are also exported from the root. `model_profile_data` exposes the
offline profile registry keyed by provider wire ID; `model_profiles` keeps the typed
`DriverId` adapters.

The `http` protocols also build for `wasm32-unknown-unknown`: inject an
`http_transport::HttpTransport` with `.with_transport(...)` on
`OpenAIProtocolChatDriver` or `OpenResponsesProtocolChatDriver`. Applications
supply host fetch, network policy and cancellation; the shared protocols keep
encoding, SSE, tools and errors. See the
[host fetch guide](https://everruns.com/docs/framework/models-and-providers/#wasm-and-host-fetch).

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-contracts)
- [Everruns Framework](https://everruns.com/docs/framework/)
- [Everruns documentation](https://everruns.com/docs)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
