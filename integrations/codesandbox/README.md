# CodeSandbox Integration

Cloud-based sandboxed code execution via [CodeSandbox](https://codesandbox.io/) REST API. Agents can create, manage, and interact with multiple isolated Firecracker microVMs per session.

## Module Structure

| Module | Purpose |
|--------|---------|
| `lib.rs` | Plugin registration, `CodeSandboxCapability` |
| `types.rs` | Constants, API response types, `SandboxState`, utilities |
| `client.rs` | HTTP client for Management + Pint APIs |
| `state.rs` | Session secrets-backed state persistence |
| `tools/sandbox.rs` | Create, list, manage sandbox lifecycle |
| `tools/exec.rs` | Execute commands, check status |
| `tools/files.rs` | Read, write files, download workspace |
| `tools/git.rs` | Clone git repositories (with GitHub auth) |

## Architecture

Two-tier API design:

- **Management API** (`api.codesandbox.io`) — sandbox lifecycle (create, start, shutdown, hibernate, delete)
- **Pint API** (dynamic `{sandbox_id}-57468.csb.app`) — in-sandbox operations (exec, files, directories)

See [specs/codesandbox.md](../../specs/codesandbox.md) for the full specification.

## Links

- [CodeSandbox API docs](https://codesandbox.io/docs/learn/sandboxes/api)
- [Capability spec](../../specs/codesandbox.md)
- [Architecture spec](../../specs/architecture.md)
