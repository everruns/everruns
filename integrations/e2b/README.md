# everruns-integrations-e2b

E2B cloud sandbox integration for Everruns agents.

This crate is part of the [Everruns](https://everruns.com) ecosystem. It adds
cloud sandboxes backed by [E2B](https://e2b.dev), letting agents create
sandboxes, run processes, and read or write files inside an isolated
environment.

## What It Provides

- Per-session E2B sandbox lifecycle with leased-resource cleanup
- Process execution over the E2B Connect RPC API
- File read/write tools inside the sandbox
- Bring-your-own E2B API key via the user connection provider
- Inventory-based Everruns integration registration

## Configuration

The E2B API key is resolved from the user's `e2b` connection only; there is no
platform-owned or environment-variable fallback. Tools fail with
`ConnectionRequired` until the connection is configured.

## License

MIT. See the repository-level `LICENSE` file.
