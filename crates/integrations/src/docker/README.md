# everruns-integrations

This integration is shipped as the `docker` module of `everruns-integrations` Enable it with `features = ["docker"]` in Cargo.

> Session-scoped Docker container execution for Everruns agents.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations` gives agents a Docker container tied to the
session lifecycle. The container starts lazily on first tool use, and agents can
run commands, read and write files, and stop it when done, a self-hosted way to
give an agent a real execution environment.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-contracts`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::docker::DockerContainerCapability;

let capability = DockerContainerCapability;

assert_eq!(capability.id(), "docker_container");
```

## What It Provides

- A session-scoped Docker container, lazily started on first use
- `docker_exec` for running commands in the container
- `docker_read_file` and `docker_write_file` for in-container file access
- `docker_stop` to tear the container down
- Inventory-based Everruns integration registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
