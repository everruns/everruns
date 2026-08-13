# everruns-integrations-openrouter-workspace

> OpenRouter workspace policy, model scouting, and server tools for Everruns.

`everruns-integrations-openrouter-workspace` owns the OpenRouter-specific
workspace metadata, compatibility checks, and bounded model-scout probes that
were formerly compiled into the execution kernel. It also owns the opt-in
provider-executed server-tool capability and its routing-config adapter.

Part of the [Everruns](https://everruns.com) ecosystem. Hosted product
composition registers these capabilities explicitly; advanced hosts can opt in
without coupling provider protocol crates to `everruns-core`.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_openrouter_workspace::OpenRouterWorkspaceCapability;

assert_eq!(OpenRouterWorkspaceCapability.id(), "openrouter_workspace");
```

## What It Provides

- OpenRouter key/workspace policy inspection
- Local routing compatibility reports
- Bounded model/provider probe and ranking tools
- High-risk, explicit OpenRouter server-tool routing (`web_search`, `web_fetch`)
- Credential redaction and explicit operator-apply semantics

## Documentation

- [Framework models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [API reference](https://docs.rs/everruns-integrations-openrouter-workspace)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
