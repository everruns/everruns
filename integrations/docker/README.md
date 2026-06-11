# everruns-integrations-docker

> Session-scoped Docker container execution for Everruns agents.

`everruns-integrations-docker` gives agents a Docker container tied to the
session lifecycle. The container starts lazily on first tool use, and agents can
run commands, read and write files, and stop it when done — a self-hosted way to
give an agent a real execution environment.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_docker::DockerContainerCapability;

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

- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
