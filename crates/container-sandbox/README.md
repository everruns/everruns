# everruns-container-sandbox

> Self-hosted container sandbox capability for Everruns agents.

`everruns-container-sandbox` runs agent code in self-hosted containers via the
Docker Engine REST API, no external sandbox provider and no `docker` CLI
dependency. It is a core capability rather than an external integration because
container execution is infrastructure that Everruns operators own, similar to
session-scoped SQL databases.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
as a capability hosts can enable.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_container_sandbox::ContainerSandboxCapability;

let capability = ContainerSandboxCapability;

assert_eq!(capability.id(), "container_sandbox");
```

## What It Provides

- Session-scoped container execution over the Docker Engine REST API
- Command execution and file access inside the container
- A self-hosted alternative to provider-backed sandboxes (Daytona, E2B, …)

## Documentation

- [Container sandbox integration](https://docs.everruns.com/integrations/container-sandbox/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
